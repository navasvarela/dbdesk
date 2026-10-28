//! SQLite's table rebuild, for the changes its `ALTER TABLE` can't make in place, as SQLite's
//! documentation describes it (<https://www.sqlite.org/lang_altertable.html#otheralter>):
//! 1. create the new table under a temporary name;
//! 2. copy the rows into it;
//! 3. drop the old table, and give the new one its name;
//! 4. create the old table's indexes and triggers again, from their saved definitions.
//!
//! Where it differs from the documentation: the new table keeps the old names of renamed
//! columns, and the columns being dropped, and only once the indexes and triggers are back
//! are columns renamed and dropped, in place. SQLite then rewrites, itself, every index,
//! trigger, view and other table's foreign key that names a renamed column, and refuses to
//! drop a column one of them still uses, so the saved definitions never name a column that
//! isn't there. The documentation leaves views and dropped columns to the user (its step 9);
//! a view naming a dropped column would otherwise break silently.
//!
//! The engine runs the statements in one transaction with foreign keys switched off (see
//! [`Capabilities::foreign_key_switch`](crate::Capabilities::foreign_key_switch)).

use super::super::{ColumnSpec, DdlError, ForeignKey, TableSpec, UniqueKey, column_list, table};
use super::Diff;
use crate::{Dialect, TokenKind, tokenize};

pub(super) fn statements(diff: &Diff<'_>, dialect: Dialect) -> Result<Vec<String>, DdlError> {
    if let Some(reason) = diff.table.rebuild_refusal {
        return Err(DdlError::Unrebuildable(reason));
    }
    let before = diff.before();
    let old_name = |new: &str| -> String {
        diff.renamed
            .iter()
            .find(|(_, renamed)| *renamed == new)
            .map_or(new, |(old, _)| old)
            .to_owned()
    };
    // The interim table keeps renamed columns under their old names, which a new column
    // can't have too.
    if diff
        .added
        .iter()
        .any(|column| diff.renamed.iter().any(|(old, _)| *old == column.name))
    {
        return Err(DdlError::CantAlter {
            what: "rebuild a table and add a column under a renamed column's old name in one \
                   change",
            dialect,
        });
    }
    let interim = interim_table(diff, &old_name);
    let copied: Vec<String> = interim
        .columns
        .iter()
        .map(|column| column.name.clone())
        .filter(|name| before.columns.iter().any(|column| &column.name == name))
        .collect();
    let copied = column_list(dialect, &copied);
    let table_name = before.name.to_sql(dialect);
    let mut statements = vec![
        table::create(&interim, dialect)?,
        format!(
            "INSERT INTO {} {copied} SELECT {} FROM {table_name}",
            interim.name.to_sql(dialect),
            copied.trim_start_matches('(').trim_end_matches(')')
        ),
        format!("DROP TABLE {table_name}"),
        // Without it, the rename checks every view and trigger first, and refuses while one
        // names the table it is about to take the place of. The renames and drops in place
        // below need it off again, to rewrite and check what names their columns.
        "PRAGMA legacy_alter_table = ON".to_owned(),
        super::super::rename("TABLE", &interim.name, &before.name.name, dialect),
        "PRAGMA legacy_alter_table = OFF".to_owned(),
    ];
    statements.extend(
        diff.table
            .dependents
            .iter()
            .map(|statement| statement.trim().trim_end_matches(';').to_owned()),
    );
    let writer = super::Writer::new(diff, dialect);
    for (old, new) in &diff.renamed {
        statements.push(writer.rename_column(old, new));
    }
    for column in &diff.dropped {
        statements.extend(writer.drop_column(column));
    }
    if let Some(new_name) = diff.new_name {
        statements.push(super::super::rename(
            "TABLE",
            &before.name,
            new_name,
            dialect,
        ));
    }
    Ok(statements)
}

/// The table the rows are copied into: the new definition with renamed columns under their
/// old names, and the columns being dropped still at the end, so that renaming and dropping
/// them in place comes last.
fn interim_table(diff: &Diff<'_>, old_name: &dyn Fn(&str) -> String) -> TableSpec {
    let before = diff.before();
    let after = diff.after;
    let old_names = |columns: &[String]| -> Vec<String> {
        columns.iter().map(|column| old_name(column)).collect()
    };
    let mut columns: Vec<ColumnSpec> = after
        .columns
        .iter()
        .map(|column| ColumnSpec {
            name: old_name(&column.name),
            ..column.clone()
        })
        .collect();
    columns.extend(diff.dropped.iter().map(|column| (*column).clone()));
    TableSpec {
        name: before.name.sibling(&format!("new_{}", before.name.name)),
        columns,
        primary_key: old_names(&after.primary_key),
        unique_keys: after
            .unique_keys
            .iter()
            .map(|key| UniqueKey {
                name: key.name.clone(),
                columns: old_names(&key.columns),
            })
            .collect(),
        foreign_keys: after
            .foreign_keys
            .iter()
            .map(|key| {
                // A key to the table itself names it as it will be called once the interim
                // table takes its name; the table's own rename comes last, in place.
                let to_itself = key.target.name == after.name.name;
                ForeignKey {
                    name: key.name.clone(),
                    columns: old_names(&key.columns),
                    target: if to_itself {
                        before.name.clone()
                    } else {
                        key.target.clone()
                    },
                    target_columns: if to_itself {
                        old_names(&key.target_columns)
                    } else {
                        key.target_columns.clone()
                    },
                    on_delete: key.on_delete,
                    on_update: key.on_update,
                }
            })
            .collect(),
        checks: after.checks.clone(),
    }
}

/// Why SQLite can't rebuild the table its `CREATE TABLE` statement describes, if it can't:
/// what the statement says that a table spec doesn't, which the rebuilt table would lose.
/// Completes "the rebuilt table would lose that it …".
pub fn rebuild_refusal(create_table: &str) -> Option<&'static str> {
    let mut depth = 0usize;
    let mut words = tokenize(create_table, Dialect::Sqlite)
        .into_iter()
        .filter(|token| {
            !matches!(
                token.kind,
                TokenKind::Whitespace | TokenKind::LineComment | TokenKind::BlockComment
            )
        })
        .map(|token| &create_table[token.range]);
    for word in words.by_ref() {
        match word {
            "(" => depth += 1,
            ")" => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            _ if depth == 1 && word.eq_ignore_ascii_case("COLLATE") => {
                return Some("has a column with its own collation (COLLATE)");
            }
            _ if depth == 1
                && (word.eq_ignore_ascii_case("GENERATED") || word.eq_ignore_ascii_case("AS")) =>
            {
                return Some("has a generated column");
            }
            _ => {}
        }
    }
    // Options follow the column list: `WITHOUT ROWID`, `STRICT`.
    words.find_map(|word| {
        if word.eq_ignore_ascii_case("WITHOUT") {
            Some("is a WITHOUT ROWID table")
        } else if word.eq_ignore_ascii_case("STRICT") {
            Some("is a STRICT table")
        } else {
            None
        }
    })
}
