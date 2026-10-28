//! `:name` parameters in a console's statements: finding them, and replacing them with the
//! placeholders a driver binds values to, or with the values as literals.
//!
//! Found with the lexer, so a `:` inside a string, a comment or a quoted identifier is never
//! one. What remains is told apart from the other uses of `:` in the dialects DbRover speaks:
//!
//! - `::` casts (PostgreSQL, DuckDB) and static methods (SQL Server), and `:=` assignments
//!   (MySQL, PostgreSQL's named arguments): the `:` must be followed at once by a name, and
//!   not come right after another `:`.
//! - Labels (`retry:`), DuckDB's `lambda x: x + 1`, and anything else written against what
//!   comes before it: a parameter's `:` follows whitespace, an operator or a keyword
//!   (`BETWEEN:lo`), never another name, a number, a string or a closing bracket.
//! - Array slices (`a[1:n]`, `a[:n]`) and anything in braces: DuckDB's struct and map
//!   literals, ClickHouse's own `{name:Type}` parameters. Nothing inside `[…]` or `{…}` is a
//!   parameter.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::ops::Range;

use dbrover_core::Cell;

use crate::lexer::{Token, TokenKind, tokenize};
use crate::{Dialect, is_keyword};

/// One `:name` in a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    /// Without the `:`.
    pub name: String,
    /// The `:name`, colon included, as byte offsets into the statement.
    pub range: Range<usize>,
}

/// How a dialect's driver takes values for parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterBinding {
    /// Numbered placeholders, one number per name: `?1` (SQLite), `$1` (DuckDB), `@P1` (SQL
    /// Server). The prefix comes before the number.
    Numbered(&'static str),
    /// A `?` per occurrence, with a value for each (MySQL), so a name used twice is bound
    /// twice.
    Positional,
    /// No bound parameters: the values are written into the statement as literals.
    Literal,
}

/// The values of a run's parameters, by name, as typed: text, or `NULL`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParameterValues(BTreeMap<String, Cell>);

impl ParameterValues {
    pub fn get(&self, name: &str) -> Option<&Cell> {
        self.0.get(name)
    }

    pub fn set(&mut self, name: impl Into<String>, value: Cell) {
        self.0.insert(name.into(), value);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// Takes `other`'s values, over any of the same names.
    pub fn extend(&mut self, other: &Self) {
        self.0.extend(
            other
                .0
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
    }
}

impl<N: Into<String>> FromIterator<(N, Cell)> for ParameterValues {
    fn from_iter<I: IntoIterator<Item = (N, Cell)>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
        )
    }
}

/// A parameter with no value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("there is no value for :{0}")]
pub struct MissingValue(pub String);

/// A statement as sent to the database: its parameters replaced, and the values to bind to
/// the placeholders, in order. Maps positions the database reports back to the statement as
/// typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub sql: String,
    pub values: Vec<Cell>,
    /// Each replacement's range in `sql`, and the `:name` it replaced in the typed statement.
    replaced: Vec<(Range<usize>, Range<usize>)>,
}

impl Prepared {
    /// The statement as typed, with nothing to bind.
    pub fn unchanged(sql: &str) -> Self {
        Self {
            sql: sql.to_owned(),
            values: Vec::new(),
            replaced: Vec::new(),
        }
    }

    /// The byte offset in the typed statement of `offset` in [`Prepared::sql`]: the same text
    /// outside the replacements, and the start of the `:name` inside one.
    pub fn typed_offset(&self, offset: usize) -> usize {
        let mut shift: isize = 0;
        for (sent, typed) in &self.replaced {
            if offset < sent.start {
                break;
            }
            if offset < sent.end {
                return typed.start;
            }
            shift = typed.end.cast_signed() - sent.end.cast_signed();
        }
        offset.saturating_add_signed(shift)
    }
}

/// Every `:name` parameter in `sql`, in order, repeats included.
pub fn parameters(sql: &str, dialect: Dialect) -> Vec<Parameter> {
    let tokens = tokenize(sql, dialect);
    let mut found = Vec::new();
    let mut brackets = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match punctuation(sql, token) {
            Some('[' | '{') => brackets += 1,
            Some(']' | '}') => brackets = brackets.saturating_sub(1),
            Some(':') if brackets == 0 => found.extend(parameter_at(sql, &tokens, index)),
            _ => {}
        }
    }
    found
}

/// Each parameter's name once, in the order they first appear.
pub fn parameter_names(sql: &str, dialect: Dialect) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for parameter in parameters(sql, dialect) {
        if !names.contains(&parameter.name) {
            names.push(parameter.name);
        }
    }
    names
}

fn punctuation(sql: &str, token: &Token) -> Option<char> {
    (token.kind == TokenKind::Punctuation)
        .then(|| sql[token.range.clone()].chars().next())
        .flatten()
}

/// The parameter the `:` at `tokens[index]` starts, if it starts one.
fn parameter_at(sql: &str, tokens: &[Token], index: usize) -> Option<Parameter> {
    let colon = &tokens[index];
    let name = tokens.get(index + 1)?;
    let starts_name = sql[name.range.clone()]
        .chars()
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_');
    if name.kind != TokenKind::Word || name.range.start != colon.range.end || !starts_name {
        return None;
    }
    let attached = index
        .checked_sub(1)
        .map(|before| &tokens[before])
        .filter(|before| before.range.end == colon.range.start);
    if let Some(before) = attached {
        let follows_value = match before.kind {
            TokenKind::Word => !is_keyword(&sql[before.range.clone()]),
            TokenKind::Number | TokenKind::String | TokenKind::QuotedIdentifier => true,
            _ => matches!(punctuation(sql, before), Some(':' | ')' | ']' | '}')),
        };
        if follows_value {
            return None;
        }
    }
    Some(Parameter {
        name: sql[name.range.clone()].to_owned(),
        range: colon.range.start..name.range.end,
    })
}

/// `sql` with its parameters replaced as the dialect's driver binds them, and the values for
/// the placeholders. A dialect whose driver doesn't bind gets literals.
///
/// # Errors
///
/// [`MissingValue`] if a parameter has no value.
pub fn bind(
    sql: &str,
    dialect: Dialect,
    values: &ParameterValues,
) -> Result<Prepared, MissingValue> {
    rewrite(sql, dialect, values, dialect.parameter_binding())
}

/// `sql` with each parameter replaced by its value as a literal, as the database would read
/// it typed: `'text'` or `NULL`. For the checks that must see what runs, and for drivers and
/// statements that can't bind.
///
/// # Errors
///
/// [`MissingValue`] if a parameter has no value.
pub fn inline(
    sql: &str,
    dialect: Dialect,
    values: &ParameterValues,
) -> Result<Prepared, MissingValue> {
    rewrite(sql, dialect, values, ParameterBinding::Literal)
}

fn rewrite(
    sql: &str,
    dialect: Dialect,
    values: &ParameterValues,
    binding: ParameterBinding,
) -> Result<Prepared, MissingValue> {
    let mut prepared = Prepared::unchanged("");
    let mut numbered: Vec<&str> = Vec::new();
    let mut copied = 0;
    let found = parameters(sql, dialect);
    for parameter in &found {
        let value = values
            .get(&parameter.name)
            .ok_or_else(|| MissingValue(parameter.name.clone()))?;
        prepared.sql.push_str(&sql[copied..parameter.range.start]);
        copied = parameter.range.end;
        let start = prepared.sql.len();
        match binding {
            ParameterBinding::Numbered(prefix) => {
                let number = match numbered.iter().position(|name| *name == parameter.name) {
                    Some(index) => index + 1,
                    None => {
                        numbered.push(&parameter.name);
                        prepared.values.push(value.clone());
                        numbered.len()
                    }
                };
                // Writing to a String can't fail.
                let _ = write!(prepared.sql, "{prefix}{number}");
            }
            ParameterBinding::Positional => {
                prepared.sql.push('?');
                prepared.values.push(value.clone());
            }
            ParameterBinding::Literal => prepared.sql.push_str(&literal(dialect, value)),
        }
        prepared
            .replaced
            .push((start..prepared.sql.len(), parameter.range.clone()));
    }
    prepared.sql.push_str(&sql[copied..]);
    Ok(prepared)
}

fn literal(dialect: Dialect, value: &Cell) -> String {
    match value {
        None => "NULL".to_owned(),
        Some(text) => dialect.quoting().string_literal(text),
    }
}

/// A comment naming the values `sql`'s parameters ran with, `-- :since = '2026-01-01'`, for
/// the history to keep with the statement. `None` if it has no parameters. Line breaks in a
/// value are written `\n`, so the comment stays one line.
pub fn values_comment(sql: &str, dialect: Dialect, values: &ParameterValues) -> Option<String> {
    let names = parameter_names(sql, dialect);
    if names.is_empty() {
        return None;
    }
    let described: Vec<String> = names
        .iter()
        .map(|name| {
            let value = values
                .get(name)
                .map_or_else(|| "?".to_owned(), |value| literal(dialect, value));
            let value = value.replace('\r', "\\r").replace('\n', "\\n");
            format!(":{name} = {value}")
        })
        .collect();
    Some(format!("-- {}", described.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(entries: &[(&str, Option<&str>)]) -> ParameterValues {
        entries
            .iter()
            .map(|(name, value)| (*name, value.map(str::to_owned)))
            .collect()
    }

    /// One statement in every dialect: a name used twice, a string with a quote and a
    /// backslash, and a `NULL`, around a cast that must stay.
    #[test]
    fn binds_or_writes_literals_in_every_dialect() {
        let sql = "SELECT :id::text, :note, :gone WHERE x = :id";
        let given = values(&[
            ("id", Some("7")),
            ("note", Some(r"it's C:\x")),
            ("gone", None),
        ]);
        let text = |value: &str| Some(value.to_owned());
        let cases = [
            (
                Dialect::Sqlite,
                "SELECT ?1::text, ?2, ?3 WHERE x = ?1",
                vec![text("7"), text(r"it's C:\x"), None],
            ),
            (
                Dialect::DuckDb,
                "SELECT $1::text, $2, $3 WHERE x = $1",
                vec![text("7"), text(r"it's C:\x"), None],
            ),
            (
                Dialect::SqlServer,
                "SELECT @P1::text, @P2, @P3 WHERE x = @P1",
                vec![text("7"), text(r"it's C:\x"), None],
            ),
            (
                Dialect::Mysql,
                "SELECT ?::text, ?, ? WHERE x = ?",
                vec![text("7"), text(r"it's C:\x"), None, text("7")],
            ),
            (
                Dialect::Postgres,
                r"SELECT '7'::text, 'it''s C:\x', NULL WHERE x = '7'",
                vec![],
            ),
            (
                Dialect::ClickHouse,
                r"SELECT '7'::text, 'it''s C:\\x', NULL WHERE x = '7'",
                vec![],
            ),
        ];
        for (dialect, expected_sql, expected_values) in cases {
            let prepared = bind(sql, dialect, &given).unwrap();
            assert_eq!(prepared.sql, expected_sql, "{dialect:?}");
            assert_eq!(prepared.values, expected_values, "{dialect:?}");
        }
        assert_eq!(
            inline(sql, Dialect::SqlServer, &given).unwrap().sql,
            r"SELECT N'7'::text, N'it''s C:\x', NULL WHERE x = N'7'"
        );
    }

    #[test]
    fn refuses_a_parameter_without_a_value() {
        assert_eq!(
            bind("SELECT :a, :b", Dialect::Sqlite, &values(&[("a", None)])),
            Err(MissingValue("b".to_owned()))
        );
        assert_eq!(
            bind("SELECT 1", Dialect::Sqlite, &ParameterValues::default()),
            Ok(Prepared::unchanged("SELECT 1"))
        );
    }

    #[test]
    fn maps_positions_back_to_the_statement_as_typed() {
        let sql = "SELECT :long_name, x FROM :t";
        let given = values(&[("long_name", Some("a")), ("t", Some("b"))]);
        let prepared = bind(sql, Dialect::Sqlite, &given).unwrap();
        assert_eq!(prepared.sql, "SELECT ?1, x FROM ?2");

        let typed = |sent_text: &str| {
            let sent = prepared.sql.find(sent_text).unwrap();
            prepared.typed_offset(sent)
        };
        assert_eq!(typed("SELECT"), 0);
        assert_eq!(typed("?1"), sql.find(":long_name").unwrap());
        assert_eq!(typed("1, x"), sql.find(":long_name").unwrap());
        assert_eq!(typed("x FROM"), sql.find("x FROM").unwrap());
        assert_eq!(typed("?2"), sql.find(":t").unwrap());
        assert_eq!(prepared.typed_offset(prepared.sql.len()), sql.len());
    }

    #[test]
    fn describes_the_values_in_a_comment() {
        let given = values(&[
            ("since", Some("2026-01-01")),
            ("note", Some("a\nb")),
            ("n", None),
        ]);

        assert_eq!(
            values_comment(
                "SELECT :since, :note, :since, :n",
                Dialect::Postgres,
                &given
            ),
            Some(r"-- :since = '2026-01-01', :note = 'a\nb', :n = NULL".to_owned())
        );
        assert_eq!(values_comment("SELECT 1", Dialect::Postgres, &given), None);
    }

    /// The parameters found in a corpus per dialect, in `tests/parameters/<dialect>.sql`: each
    /// statement follows a `-- expect:` line listing them, or saying `none`.
    #[test]
    fn finds_the_parameters_of_each_dialects_corpus() {
        const EXPECT: &str = "-- expect:";
        let corpora = [
            (
                Dialect::Postgres,
                include_str!("../tests/parameters/postgres.sql"),
            ),
            (
                Dialect::Mysql,
                include_str!("../tests/parameters/mysql.sql"),
            ),
            (
                Dialect::Sqlite,
                include_str!("../tests/parameters/sqlite.sql"),
            ),
            (
                Dialect::SqlServer,
                include_str!("../tests/parameters/sqlserver.sql"),
            ),
            (
                Dialect::DuckDb,
                include_str!("../tests/parameters/duckdb.sql"),
            ),
            (
                Dialect::ClickHouse,
                include_str!("../tests/parameters/clickhouse.sql"),
            ),
        ];
        for (dialect, corpus) in corpora {
            let mut previous_end = 0;
            let mut checked = 0;
            for range in crate::split_statements(corpus, dialect) {
                let before = &corpus[previous_end..range.start];
                previous_end = range.end;
                let statement = &corpus[range];
                let expected = before
                    .lines()
                    .filter_map(|line| line.trim().strip_prefix(EXPECT))
                    .next_back()
                    .unwrap_or_else(|| panic!("{dialect:?}: no `{EXPECT}` line before {statement}"))
                    .trim();
                let found: Vec<String> = parameters(statement, dialect)
                    .into_iter()
                    .map(|parameter| format!(":{}", parameter.name))
                    .collect();
                let found = if found.is_empty() {
                    "none".to_owned()
                } else {
                    found.join(" ")
                };
                assert_eq!(found, expected, "{dialect:?}: {statement}");
                checked += 1;
            }
            assert!(
                checked >= 4,
                "{dialect:?}: only {checked} statements checked"
            );
        }
    }

    /// A statement with parameters isn't underlined as a syntax error while it is typed, nor
    /// unclassifiable for the confirmations, in any dialect.
    #[test]
    fn statements_with_parameters_still_parse() {
        let sql = "SELECT * FROM orders WHERE created_at >= :since AND id IN (:a, :b)";
        for dialect in [
            Dialect::Postgres,
            Dialect::Mysql,
            Dialect::Sqlite,
            Dialect::SqlServer,
            Dialect::DuckDb,
            Dialect::ClickHouse,
        ] {
            let problems = crate::check_statement(sql, dialect);
            assert!(problems.is_empty(), "{dialect:?}: {problems:?}");
            assert_eq!(
                crate::classify(sql, dialect).ok(),
                Some(crate::StatementKind::Read),
                "{dialect:?}"
            );
        }
    }

    #[test]
    fn names_each_parameter_once_in_order() {
        assert_eq!(
            parameter_names("SELECT :b, :a, :b, :_c1", Dialect::Postgres),
            ["b", "a", "_c1"]
        );
    }
}
