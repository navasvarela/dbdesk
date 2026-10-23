//! The lint rules that need the catalog: tables and columns the explorer's cached catalog
//! doesn't have.
//!
//! A name is only reported when the catalog level that would hold it is loaded, so a missing
//! warning means "not checked" rather than "fine", but a warning is never a guess. Names the
//! statement or the console defines itself (CTEs, `CREATE TABLE` earlier in the console,
//! `SELECT … INTO`), table functions, and system catalogs are left alone.

use std::ops::Range;

use dbdesk_sql::{
    Dialect, LintRule, Problem, ProblemSource, Severity, Token, TokenKind, first_word,
    is_checkable, is_keyword, split_statements, tokenize,
};

use super::completion::{CatalogSource, TableRef, resolve, table_references};

/// Qualifiers that name a row rather than a table: `excluded` in an upsert, `new` and `old`
/// in a trigger body.
const ROW_QUALIFIERS: &[&str] = &["excluded", "new", "old"];

/// Names of tables and views the console's text creates (`CREATE TABLE`, `CREATE VIEW`,
/// `SELECT … INTO`), which the catalog may not have yet.
pub fn created_names(text: &str, dialect: Dialect) -> Vec<String> {
    split_statements(text, dialect)
        .into_iter()
        .flat_map(|range| created_by(&text[range], dialect))
        .collect()
}

fn created_by(statement: &str, dialect: Dialect) -> Vec<String> {
    let words = words(statement, dialect);
    let selects = first_word(statement, dialect).as_deref() == Some("select");
    let mut names = Vec::new();
    for (index, word) in words.iter().enumerate() {
        let creates = matches!(word.lower.as_deref(), Some("table" | "view"))
            && words[..index]
                .iter()
                .rev()
                .take(4)
                .any(|before| before.lower.as_deref() == Some("create"));
        let selects_into = selects && word.lower.as_deref() == Some("into");
        if !(creates || selects_into) {
            continue;
        }
        // Skip `IF NOT EXISTS`, then take the last part of a possibly qualified name.
        let mut next = index + 1;
        while words
            .get(next)
            .and_then(|word| word.lower.as_deref())
            .is_some_and(|word| matches!(word, "if" | "not" | "exists"))
        {
            next += 1;
        }
        while words.get(next + 1).is_some_and(|word| word.text == ".") {
            next += 2;
        }
        if let Some(name) = words.get(next).and_then(|word| word.name.clone()) {
            names.push(name);
        }
    }
    names
}

/// The unknown tables and columns in one statement, with ranges relative to it.
pub fn problems(
    statement: &str,
    dialect: Dialect,
    catalog: &dyn CatalogSource,
    created: &[String],
) -> Vec<Problem> {
    if !is_checkable(statement, dialect) {
        return Vec::new();
    }
    let references = table_references(statement, dialect);
    let defined = defined_names(statement, dialect);
    let is_defined = |name: &str| {
        defined
            .iter()
            .chain(created)
            .any(|defined| defined.eq_ignore_ascii_case(name))
    };
    let mut problems: Vec<Problem> = references
        .iter()
        .filter(|reference| !is_defined(&reference.table))
        .filter(|reference| is_plain_table_reference(statement, dialect, reference))
        .filter_map(|reference| unknown_table(reference, dialect, catalog))
        .collect();
    problems.extend(unknown_columns(
        statement,
        dialect,
        catalog,
        &references,
        &is_defined,
    ));
    problems
}

fn warning(rule: LintRule, range: Range<usize>, message: String) -> Problem {
    Problem {
        range,
        severity: Severity::Warning,
        message,
        source: ProblemSource::Lint(rule),
    }
}

fn unknown_table(
    reference: &TableRef,
    dialect: Dialect,
    catalog: &dyn CatalogSource,
) -> Option<Problem> {
    if is_system_name(reference, dialect) || !catalog.tables_loaded(reference.schema.as_deref()) {
        return None;
    }
    let tables = catalog.tables();
    let known = tables.iter().any(|table| {
        table.name.eq_ignore_ascii_case(&reference.table)
            && reference
                .schema
                .as_deref()
                .is_none_or(|schema| table.schema.eq_ignore_ascii_case(schema))
    });
    if known {
        return None;
    }
    let name = match &reference.schema {
        Some(schema) => format!("{schema}.{}", reference.table),
        None => reference.table.clone(),
    };
    Some(warning(
        LintRule::UnknownTable,
        reference.range.clone(),
        format!("No table or view {name} in the explorer's catalog (refresh it if {name} is new)"),
    ))
}

/// System catalogs, which the explorer doesn't list: PostgreSQL's `pg_*` tables (always on the
/// search path) and its catalog schemas, SQLite's `sqlite_*` tables and `pragma_*` functions.
fn is_system_name(reference: &TableRef, dialect: Dialect) -> bool {
    let table = reference.table.to_ascii_lowercase();
    let schema = reference.schema.as_deref().map(str::to_ascii_lowercase);
    match dialect {
        Dialect::Postgres => {
            table.starts_with("pg_")
                || matches!(
                    schema.as_deref(),
                    Some("pg_catalog" | "information_schema" | "pg_toast")
                )
        }
        Dialect::Sqlite => {
            table.starts_with("sqlite_")
                || table.starts_with("pragma_")
                || schema.as_deref() == Some("temp")
        }
    }
}

/// Whether the reference names a table: not a table function (`generate_series(1, 3)`), and
/// not the new table of a `SELECT … INTO`.
fn is_plain_table_reference(statement: &str, dialect: Dialect, reference: &TableRef) -> bool {
    let tokens = meaningful_tokens(statement, dialect);
    let after = tokens
        .iter()
        .find(|token| token.range.start >= reference.range.end);
    if after.is_some_and(|token| &statement[token.range.clone()] == "(") {
        return false;
    }
    let before = tokens
        .iter()
        .rev()
        .find(|token| token.range.end <= reference.range.start)
        .map(|token| statement[token.range.clone()].to_ascii_lowercase());
    let verb = first_word(statement, dialect).unwrap_or_default();
    !(before.as_deref() == Some("into") && verb != "insert" && verb != "replace")
}

/// `alias.column` where the alias (or table) names a catalog table whose columns are loaded
/// and that has no such column.
fn unknown_columns(
    statement: &str,
    dialect: Dialect,
    catalog: &dyn CatalogSource,
    references: &[TableRef],
    is_defined: &dyn Fn(&str) -> bool,
) -> Vec<Problem> {
    let words = words(statement, dialect);
    let tables = catalog.tables();
    let mut problems = Vec::new();
    for index in 0..words.len().saturating_sub(2) {
        let [qualifier, dot, column] = [&words[index], &words[index + 1], &words[index + 2]];
        let (Some(qualifier_name), Some(column_name)) = (&qualifier.name, &column.name) else {
            continue;
        };
        let chained = index > 0 && words[index - 1].text == ".";
        let continues = words
            .get(index + 3)
            .is_some_and(|next| next.text == "." || next.text == "(");
        // `c.` still being typed, before the next clause: `c.FROM` names no column.
        let keyword = !column.quoted && is_keyword(column_name);
        if dot.text != "." || chained || continues || keyword {
            continue;
        }
        if ROW_QUALIFIERS
            .iter()
            .any(|row| row.eq_ignore_ascii_case(qualifier_name))
        {
            continue;
        }
        let Some(reference) = references.iter().find(|reference| {
            reference
                .alias
                .as_deref()
                .unwrap_or(&reference.table)
                .eq_ignore_ascii_case(qualifier_name)
        }) else {
            continue;
        };
        if is_defined(&reference.table) {
            continue;
        }
        let Some(table) = resolve(
            reference.schema.as_deref(),
            &reference.table,
            &tables,
            catalog,
        ) else {
            continue;
        };
        let Some(columns) = catalog.columns(table) else {
            continue;
        };
        let known = columns.iter().any(|known| {
            if column.quoted {
                known.name == *column_name
            } else {
                known.name.eq_ignore_ascii_case(column_name)
            }
        });
        if !known {
            problems.push(warning(
                LintRule::UnknownColumn,
                column.range.clone(),
                format!(
                    "{} has no column {column_name} in the explorer's catalog",
                    table.name
                ),
            ));
        }
    }
    problems
}

/// Names the statement defines for itself: CTEs (`name AS (`, `name (a, b) AS (`) and
/// subquery aliases are the ones a table reference could collide with.
fn defined_names(statement: &str, dialect: Dialect) -> Vec<String> {
    let words = words(statement, dialect);
    let mut names = Vec::new();
    for (index, word) in words.iter().enumerate() {
        let Some(name) = &word.name else {
            continue;
        };
        let mut next = index + 1;
        if words.get(next).is_some_and(|word| word.text == "(") {
            // A column list: skip to its closing parenthesis.
            let mut depth = 0usize;
            while let Some(word) = words.get(next) {
                match word.text.as_str() {
                    "(" => depth += 1,
                    ")" => depth -= 1,
                    _ => {}
                }
                next += 1;
                if depth == 0 {
                    break;
                }
            }
        }
        let is_as = words.get(next).and_then(|word| word.lower.as_deref()) == Some("as");
        let opens = words.get(next + 1).is_some_and(|word| word.text == "(");
        if is_as && opens {
            names.push(name.clone());
        }
    }
    names
}

fn meaningful_tokens(text: &str, dialect: Dialect) -> Vec<Token> {
    tokenize(text, dialect)
        .into_iter()
        .filter(|token| !token.kind.is_trivia())
        .collect()
}

/// A meaningful token, with what the rules above ask of it.
struct Word {
    text: String,
    /// Lowercase, for words.
    lower: Option<String>,
    /// The name it stands for, for words and quoted identifiers.
    name: Option<String>,
    quoted: bool,
    range: Range<usize>,
}

fn words(text: &str, dialect: Dialect) -> Vec<Word> {
    meaningful_tokens(text, dialect)
        .into_iter()
        .map(|token| {
            let written = &text[token.range.clone()];
            let quoted = token.kind == TokenKind::QuotedIdentifier;
            let name = match token.kind {
                TokenKind::Word => Some(written.to_owned()),
                TokenKind::QuotedIdentifier => written
                    .get(1..written.len().saturating_sub(1))
                    .map(|inner| inner.replace("\"\"", "\"")),
                _ => None,
            };
            Word {
                text: written.to_owned(),
                lower: (token.kind == TokenKind::Word).then(|| written.to_ascii_lowercase()),
                name,
                quoted,
                range: token.range,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::completion::{ColumnEntry, TableEntry};
    use super::*;

    /// A catalog with `public.orders (id, customer)`, and `loaded` deciding whether its tables
    /// count as fully loaded.
    struct Catalog {
        loaded: bool,
    }

    impl CatalogSource for Catalog {
        fn default_schema(&self) -> &str {
            "public"
        }

        fn schemas(&self) -> Vec<String> {
            vec!["public".to_owned()]
        }

        fn tables(&self) -> Vec<TableEntry> {
            vec![TableEntry {
                schema: "public".to_owned(),
                name: "orders".to_owned(),
                is_view: false,
            }]
        }

        fn columns(&self, _: &TableEntry) -> Option<Vec<ColumnEntry>> {
            let column = |name: &str| ColumnEntry {
                name: name.to_owned(),
                detail: None,
            };
            self.loaded.then(|| vec![column("id"), column("customer")])
        }

        fn tables_loaded(&self, _: Option<&str>) -> bool {
            self.loaded
        }

        fn functions(&self) -> Vec<super::super::completion::FunctionEntry> {
            Vec::new()
        }
    }

    fn flagged(sql: &str, loaded: bool) -> Vec<(LintRule, &str)> {
        let created = created_names("create temp table scratch (n int);", Dialect::Postgres);
        problems(sql, Dialect::Postgres, &Catalog { loaded }, &created)
            .into_iter()
            .filter_map(|problem| match problem.source {
                ProblemSource::Lint(rule) => Some((rule, &sql[problem.range])),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn flags_unknown_tables_and_columns_only_when_the_catalog_is_loaded() {
        use LintRule::*;
        let cases: [(&str, &[(LintRule, &str)]); 10] = [
            ("select * from ordres", &[(UnknownTable, "ordres")]),
            (
                "select o.custmer from orders o",
                &[(UnknownColumn, "custmer")],
            ),
            (
                "select orders.id, o.\"ID\" from orders, orders o",
                &[(UnknownColumn, "\"ID\"")],
            ),
            (
                "select * from public.nope",
                &[(UnknownTable, "public.nope")],
            ),
            ("select o.\nfrom orders o", &[]),
            ("with x as (select 1) select * from x", &[]),
            ("select * from scratch", &[]),
            ("select * from generate_series(1, 3) g, pg_class", &[]),
            ("select id into backup from orders", &[]),
            (
                "insert into orders (id) values (1) on conflict (id) do update set customer = excluded.customer",
                &[],
            ),
        ];
        for (sql, expected) in cases {
            assert_eq!(flagged(sql, true), expected, "{sql}");
            assert_eq!(flagged(sql, false), [], "{sql}: nothing until loaded");
        }
    }
}
