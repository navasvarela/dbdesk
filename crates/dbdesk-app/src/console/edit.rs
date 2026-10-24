//! Editing result rows: which results can be edited, the changes waiting to be submitted, and
//! the `UPDATE`s that submit them. Pure; the grid draws it and the engine runs the SQL.

use std::collections::BTreeMap;

use dbdesk_core::{Cell, ColumnInfo, Identifier, Quoting, RelationName, Row, TableDescription};
use dbdesk_sql::{Dialect, Token, TokenKind, first_word, tokenize};

/// Words that make a `SELECT` combine, group or deduplicate rows, so a result row no longer
/// stands for one table row.
const NOT_ONE_ROW_PER_ROW: &[&str] = &[
    "join",
    "union",
    "intersect",
    "except",
    "group",
    "having",
    "distinct",
    "window",
];

/// Whether a statement's result can be edited, as far as is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Editability {
    /// Not looked at yet.
    Unknown,
    /// The table is being described; editing waits for the answer.
    Describing(RelationName),
    Editable(EditTarget),
    ReadOnly(String),
}

/// What submitting edits needs: the table, and how result columns map onto it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditTarget {
    pub table: TableDescription,
    /// The result columns holding the primary key, in key order.
    key: Vec<usize>,
    /// Per result column: whether it is a column of the table (and named once).
    editable: Vec<bool>,
}

impl EditTarget {
    pub fn is_editable(&self, column: usize) -> bool {
        self.editable.get(column).copied().unwrap_or(false)
    }
}

/// The table a statement's rows come from, if they come from exactly one table, one row per
/// table row: a `SELECT` from a single table, without joins, set operations, grouping or
/// `DISTINCT`. `Err` says why not.
pub fn editable_source(sql: &str, dialect: Dialect) -> Result<RelationName, String> {
    let not_editable = || Err("Only rows selected from a single table can be edited".to_owned());
    if first_word(sql, dialect).as_deref() != Some("select") {
        return not_editable();
    }
    let tokens: Vec<Token> = tokenize(sql, dialect)
        .into_iter()
        .filter(|token| !token.kind.is_trivia())
        .collect();
    let text = |token: &Token| &sql[token.range.clone()];
    let mut depth = 0usize;
    let mut from = None;
    for (index, token) in tokens.iter().enumerate() {
        match text(token) {
            "(" => depth += 1,
            ")" => depth = depth.saturating_sub(1),
            _ if depth > 0 || token.kind != TokenKind::Word => {}
            word => {
                let word = word.to_ascii_lowercase();
                if NOT_ONE_ROW_PER_ROW.contains(&word.as_str())
                    || (word == "from" && from.is_some())
                {
                    return not_editable();
                }
                if word == "from" {
                    from = Some(index + 1);
                }
            }
        }
    }
    let Some(start) = from else {
        return not_editable();
    };
    let Some((name, next)) = relation_name(sql, &tokens, start) else {
        return not_editable();
    };
    if tokens.get(next).is_some_and(|token| text(token) == ",") {
        return not_editable();
    }
    Ok(name)
}

fn relation_name(sql: &str, tokens: &[Token], start: usize) -> Option<(RelationName, usize)> {
    let identifier = |index: usize| {
        let token = tokens.get(index)?;
        let written = &sql[token.range.clone()];
        match token.kind {
            TokenKind::Word => Some(Identifier {
                name: written.to_owned(),
                quoted: false,
            }),
            TokenKind::QuotedIdentifier => Some(Identifier {
                name: written
                    .get(1..written.len().checked_sub(1)?)?
                    .replace("\"\"", "\""),
                quoted: true,
            }),
            _ => None,
        }
    };
    let first = identifier(start)?;
    let dotted = tokens
        .get(start + 1)
        .is_some_and(|token| &sql[token.range.clone()] == ".");
    if dotted {
        let name = identifier(start + 2)?;
        Some((
            RelationName {
                schema: Some(first),
                name,
            },
            start + 3,
        ))
    } else {
        Some((
            RelationName {
                schema: None,
                name: first,
            },
            start + 1,
        ))
    }
}

/// Maps a result onto the described table. Read-only, with the reason, if the table has no
/// primary key or the result lacks part of it.
pub fn edit_target(table: TableDescription, columns: &[ColumnInfo]) -> Editability {
    if table.primary_key.is_empty() {
        return Editability::ReadOnly(format!(
            "{} has no primary key, so its rows can't be told apart",
            table.name
        ));
    }
    let named_once = |name: &str| columns.iter().filter(|column| column.name == name).count() == 1;
    let editable: Vec<bool> = columns
        .iter()
        .map(|column| table.columns.contains(&column.name) && named_once(&column.name))
        .collect();
    let key: Option<Vec<usize>> = table
        .primary_key
        .iter()
        .map(|key_column| {
            columns
                .iter()
                .position(|column| &column.name == key_column)
                .filter(|&index| editable[index])
        })
        .collect();
    match key {
        Some(key) => Editability::Editable(EditTarget {
            table,
            key,
            editable,
        }),
        None => Editability::ReadOnly(format!(
            "Select the primary key ({}) to edit rows",
            table.primary_key.join(", ")
        )),
    }
}

/// Editing one statement's result: whether it can be, what changed, and the cell being
/// typed into.
#[derive(Debug)]
pub struct EditSession {
    pub editability: Editability,
    pub pending: PendingEdits,
    pub editing: Option<CellEditor>,
    /// Edits were sent and the database hasn't answered yet.
    pub submitting: bool,
    /// Why the last submit failed; the changes stay pending.
    pub error: Option<String>,
}

impl Default for EditSession {
    fn default() -> Self {
        Self {
            editability: Editability::Unknown,
            pending: PendingEdits::default(),
            editing: None,
            submitting: false,
            error: None,
        }
    }
}

impl EditSession {
    pub fn target(&self) -> Option<&EditTarget> {
        match &self.editability {
            Editability::Editable(target) => Some(target),
            Editability::Unknown | Editability::Describing(_) | Editability::ReadOnly(_) => None,
        }
    }

    /// Drops every pending change.
    pub fn revert(&mut self) {
        self.pending.clear();
        self.editing = None;
        self.error = None;
    }

    pub fn is_editable(&self, column: usize) -> bool {
        !self.submitting
            && self
                .target()
                .is_some_and(|target| target.is_editable(column))
    }
}

/// A cell being typed into, by loaded row and column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellEditor {
    pub row: usize,
    pub column: usize,
    pub text: String,
    /// Whether the text field has been given focus yet; it asks once, when it appears.
    pub focused: bool,
}

/// Changes typed into the grid and not submitted yet, keyed by loaded row and column.
#[derive(Debug, Default)]
pub struct PendingEdits {
    changes: BTreeMap<usize, BTreeMap<usize, Cell>>,
}

impl PendingEdits {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// How many cells have changed.
    pub fn len(&self) -> usize {
        self.changes.values().map(BTreeMap::len).sum()
    }

    pub fn get(&self, row: usize, column: usize) -> Option<&Cell> {
        self.changes.get(&row)?.get(&column)
    }

    /// Records a new value; setting a cell back to what was loaded drops the change.
    pub fn set(&mut self, rows: &[Row], row: usize, column: usize, value: Cell) {
        let original = rows.get(row).and_then(|loaded| loaded.get(column));
        let changes = self.changes.entry(row).or_default();
        if original == Some(&value) {
            changes.remove(&column);
        } else {
            changes.insert(column, value);
        }
        if changes.is_empty() {
            self.changes.remove(&row);
        }
    }

    pub fn clear(&mut self) {
        self.changes.clear();
    }

    /// The changed rows' indices, in the order [`PendingEdits::statements`] lists them.
    pub fn rows(&self) -> Vec<usize> {
        self.changes.keys().copied().collect()
    }

    /// One `UPDATE` per changed row, identifying it by its primary key as loaded.
    pub fn statements(
        &self,
        target: &EditTarget,
        columns: &[ColumnInfo],
        rows: &[Row],
        quoting: Quoting,
    ) -> Vec<String> {
        self.changes
            .iter()
            .filter_map(|(&row, changes)| {
                let loaded = rows.get(row)?;
                let row = RowChange { loaded, changes };
                Some(update_statement(target, columns, row, quoting))
            })
            .collect()
    }

    /// Writes submitted values into the loaded rows, once the database has them.
    pub fn apply_to(&mut self, rows: &mut [Row]) {
        for (row, changes) in std::mem::take(&mut self.changes) {
            if let Some(loaded) = rows.get_mut(row) {
                for (column, value) in changes {
                    if let Some(cell) = loaded.get_mut(column) {
                        *cell = value;
                    }
                }
            }
        }
    }
}

/// One loaded row and the values typed over it, by column.
struct RowChange<'a> {
    loaded: &'a Row,
    changes: &'a BTreeMap<usize, Cell>,
}

fn update_statement(
    target: &EditTarget,
    columns: &[ColumnInfo],
    row: RowChange<'_>,
    quoting: Quoting,
) -> String {
    let assignments: Vec<String> = row
        .changes
        .iter()
        .filter_map(|(&column, value)| {
            let name = quoting.identifier(&columns.get(column)?.name);
            Some(format!("{name} = {}", literal(value, quoting)))
        })
        .collect();
    let conditions: Vec<String> = target
        .key
        .iter()
        .filter_map(|&column| {
            let name = quoting.identifier(&columns.get(column)?.name);
            Some(match row.loaded.get(column)? {
                None => format!("{name} IS NULL"),
                value => format!("{name} = {}", literal(value, quoting)),
            })
        })
        .collect();
    format!(
        "UPDATE {} SET {} WHERE {}",
        target.table.qualified_name(quoting),
        assignments.join(", "),
        conditions.join(" AND ")
    )
}

/// A value as a string literal, or `NULL`. The engines turn a string literal into the
/// column's type (PostgreSQL from an untyped literal, SQLite by type affinity), which is how
/// text from the grid (ADR-0004) goes back in without dbdesk knowing the types.
fn literal(value: &Cell, quoting: Quoting) -> String {
    match value {
        None => "NULL".to_owned(),
        Some(text) => quoting.string_literal(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.to_owned(),
            type_name: None,
        }
    }

    fn table(columns: &[&str], key: &[&str]) -> TableDescription {
        TableDescription {
            schema: "public".to_owned(),
            name: "people".to_owned(),
            columns: columns.iter().map(|c| (*c).to_owned()).collect(),
            primary_key: key.iter().map(|c| (*c).to_owned()).collect(),
        }
    }

    fn cells(values: &[Option<&str>]) -> Row {
        values
            .iter()
            .map(|value| value.map(str::to_owned))
            .collect()
    }

    #[test]
    fn finds_the_one_table_a_select_reads() {
        let editable = [
            (
                "select * from people where id > 1 order by id",
                None,
                "people",
                false,
            ),
            (
                "SELECT id FROM public.\"People\" p LIMIT 5",
                Some("public"),
                "People",
                true,
            ),
            ("select (select 1), name from people", None, "people", false),
        ];
        for (sql, schema, name, quoted) in editable {
            let found = editable_source(sql, Dialect::Postgres).unwrap();
            assert_eq!(found.schema.map(|s| s.name).as_deref(), schema, "{sql}");
            assert_eq!(
                (found.name.name.as_str(), found.name.quoted),
                (name, quoted),
                "{sql}"
            );
        }
        for sql in [
            "update people set name = 'x'",
            "select 1",
            "select * from a, b",
            "select * from a join b on true",
            "select name, count(*) from people group by name",
            "select distinct name from people",
            "select * from a union select * from b",
            "select * from (select * from people) p",
        ] {
            assert!(editable_source(sql, Dialect::Postgres).is_err(), "{sql}");
        }
    }

    #[test]
    fn needs_the_whole_primary_key_in_the_result() {
        let described = table(&["id", "part", "name"], &["id", "part"]);
        let result = [
            column("id"),
            column("name"),
            column("part"),
            column("upper"),
        ];
        let Editability::Editable(target) = edit_target(described.clone(), &result) else {
            panic!("expected the result to be editable");
        };
        assert_eq!(target.key, [0, 2]);
        assert!(target.is_editable(1));
        assert!(!target.is_editable(3), "not a table column");

        assert!(matches!(
            edit_target(described.clone(), &[column("id"), column("name")]),
            Editability::ReadOnly(reason) if reason.contains("(id, part)")
        ));
        assert!(matches!(
            edit_target(table(&["n"], &[]), &[column("n")]),
            Editability::ReadOnly(reason) if reason.contains("no primary key")
        ));
        assert!(matches!(
            edit_target(described, &[column("id"), column("id"), column("part")]),
            Editability::ReadOnly(_)
        ));
    }

    #[test]
    fn writes_mysql_updates_with_backticks_and_doubled_backslashes() {
        let columns = [column("id"), column("name")];
        let Editability::Editable(target) = edit_target(table(&["id", "name"], &["id"]), &columns)
        else {
            panic!("expected the result to be editable");
        };
        let rows = vec![cells(&[Some("1"), Some("ada")])];
        let mut pending = PendingEdits::default();
        pending.set(&rows, 0, 1, Some(r"C:\ada's".to_owned()));

        assert_eq!(
            pending.statements(&target, &columns, &rows, Dialect::Mysql.quoting()),
            [r"UPDATE `public`.`people` SET `name` = 'C:\\ada''s' WHERE `id` = '1'"]
        );
    }

    #[test]
    fn writes_sql_server_updates_with_brackets_and_unicode_literals() {
        let columns = [column("id"), column("name")];
        let Editability::Editable(target) = edit_target(table(&["id", "name"], &["id"]), &columns)
        else {
            panic!("expected the result to be editable");
        };
        let rows = vec![cells(&[Some("1"), Some("ada")])];
        let mut pending = PendingEdits::default();
        pending.set(&rows, 0, 1, Some("Zoë's]".to_owned()));

        assert_eq!(
            pending.statements(&target, &columns, &rows, Dialect::SqlServer.quoting()),
            ["UPDATE [public].[people] SET [name] = N'Zoë''s]' WHERE [id] = N'1'"]
        );
    }

    #[test]
    fn writes_one_update_per_changed_row_keyed_by_the_loaded_key() {
        let columns = [column("id"), column("name"), column("note")];
        let Editability::Editable(target) =
            edit_target(table(&["id", "name", "note"], &["id"]), &columns)
        else {
            panic!("expected the result to be editable");
        };
        let mut rows = vec![
            cells(&[Some("1"), Some("ada"), None]),
            cells(&[Some("2"), Some("bob"), Some("x")]),
        ];
        let mut pending = PendingEdits::default();
        pending.set(&rows, 1, 2, None);
        pending.set(&rows, 1, 1, Some("Bob's".to_owned()));
        pending.set(&rows, 0, 1, Some("changed".to_owned()));
        pending.set(&rows, 0, 1, Some("ada".to_owned()));
        assert_eq!(pending.len(), 2, "setting a value back drops the change");

        assert_eq!(
            pending.statements(&target, &columns, &rows, Quoting::STANDARD),
            [r#"UPDATE "public"."people" SET "name" = 'Bob''s', "note" = NULL WHERE "id" = '2'"#]
        );
        pending.apply_to(&mut rows);
        assert!(pending.is_empty());
        assert_eq!(rows[1], cells(&[Some("2"), Some("Bob's"), None]));
    }
}
