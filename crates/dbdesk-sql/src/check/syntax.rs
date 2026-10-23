//! Syntax errors, from `sqlparser` with its PostgreSQL and SQLite dialects.
//!
//! `sqlparser` reads most of both dialects' queries, but not all: the constructs it rejects
//! although the database accepts them are listed in [`KNOWN_GAPS`] and [`has_known_gap`], and
//! a statement using one is not checked. That list came from running a corpus of valid
//! statements through the parser (`check::tests`); a statement the parser wrongly rejects is a
//! bug to add there.

use sqlparser::dialect::{PostgreSqlDialect, SQLiteDialect};
use sqlparser::parser::Parser;

use super::{Problem, ProblemSource, Severity, is_checkable, token_at};
use crate::lexer::{Dialect, Token, TokenKind, tokenize};

/// Word sequences `sqlparser` 0.63 rejects in valid statements, per dialect.
const KNOWN_GAPS: &[(Dialect, &[&str])] = &[
    // `SELECT * FROM ROWS FROM (f(), g())`
    (Dialect::Postgres, &["rows", "from"]),
    // `FOR NO KEY UPDATE`, `FOR KEY SHARE`
    (Dialect::Postgres, &["no", "key"]),
    (Dialect::Postgres, &["key", "share"]),
    (Dialect::Postgres, &["between", "symmetric"]),
    // `FROM t INDEXED BY i`, `FROM t NOT INDEXED`
    (Dialect::Sqlite, &["indexed", "by"]),
    (Dialect::Sqlite, &["not", "indexed"]),
];

/// What may follow `IS` (or `IS NOT`) for `sqlparser`; SQLite also accepts any expression
/// there (`a IS b`).
const AFTER_IS: &[&str] = &[
    "null", "true", "false", "distinct", "unknown", "json", "not",
];

/// The statement's syntax error, if it is a checked kind of statement and the parser rejects
/// it.
pub(super) fn problem(statement: &str, dialect: Dialect) -> Option<Problem> {
    if !is_checkable(statement, dialect) || has_known_gap(statement, dialect) {
        return None;
    }
    let parsed = match dialect {
        Dialect::Postgres => Parser::parse_sql(&PostgreSqlDialect {}, statement),
        Dialect::Sqlite => Parser::parse_sql(&SQLiteDialect {}, statement),
    };
    let error = parsed.err()?.to_string();
    let error = error.strip_prefix("sql parser error: ").unwrap_or(&error);
    let (message, location) = match error.rsplit_once(" at Line: ") {
        Some((message, location)) => (message, parse_location(location)),
        None => (error, None),
    };
    let offset = location
        .and_then(|(line, column)| byte_offset(statement, line, column))
        .unwrap_or(statement.len());
    Some(Problem {
        range: token_at(statement, dialect, offset),
        severity: Severity::Error,
        message: describe(message),
        source: ProblemSource::Syntax,
    })
}

/// Reads `3, Column: 14` into a 1-based line and column.
fn parse_location(location: &str) -> Option<(usize, usize)> {
    let (line, column) = location.split_once(", Column: ")?;
    Some((line.trim().parse().ok()?, column.trim().parse().ok()?))
}

/// The byte offset of a 1-based line and column (counted in characters, as `sqlparser` does).
fn byte_offset(text: &str, line: usize, column: usize) -> Option<usize> {
    let line_start = if line <= 1 {
        0
    } else {
        text.match_indices('\n').nth(line - 2)?.0 + 1
    };
    let rest = &text[line_start..];
    let within = rest
        .char_indices()
        .nth(column.checked_sub(1)?)
        .map_or(rest.len(), |(byte, _)| byte);
    Some(line_start + within)
}

/// `Expected: an expression, found: =` reads as `Syntax error: expected an expression, found
/// =`; the end of the text is called that rather than `EOF`.
fn describe(message: &str) -> String {
    let message = message
        .replace("Expected: ", "expected ")
        .replace(", found: EOF", " at the end of the statement")
        .replace(", found: ", ", found ");
    format!("Syntax error: {message}")
}

fn has_known_gap(statement: &str, dialect: Dialect) -> bool {
    let tokens: Vec<Token> = tokenize(statement, dialect)
        .into_iter()
        .filter(|token| !token.kind.is_trivia())
        .collect();
    let word = |index: usize| {
        tokens
            .get(index)
            .filter(|token| token.kind == TokenKind::Word)
            .map(|token| statement[token.range.clone()].to_ascii_lowercase())
    };
    let text = |index: usize| {
        tokens
            .get(index)
            .map(|token| &statement[token.range.clone()])
    };
    (0..tokens.len()).any(|index| {
        let sequence = KNOWN_GAPS
            .iter()
            .filter(|(gap_dialect, _)| *gap_dialect == dialect)
            .any(|(_, words)| {
                words
                    .iter()
                    .enumerate()
                    .all(|(offset, expected)| word(index + offset).as_deref() == Some(expected))
            });
        let current = word(index);
        // PostgreSQL's XML functions take keyword arguments: `xmlelement(name foo)`.
        let xml_function = dialect == Dialect::Postgres
            && current
                .as_deref()
                .is_some_and(|word| word.starts_with("xml"))
            && text(index + 1) == Some("(");
        // `JOIN t USING (id) AS j` (PostgreSQL 14) names the join's columns.
        let using_alias = dialect == Dialect::Postgres
            && current.as_deref() == Some("using")
            && text(index + 1) == Some("(")
            && (index + 2..tokens.len())
                .find(|&close| text(close) == Some(")"))
                .is_some_and(|close| word(close + 1).as_deref() == Some("as"));
        let is_expression = dialect == Dialect::Sqlite && current.as_deref() == Some("is") && {
            let next = if word(index + 1).as_deref() == Some("not") {
                index + 2
            } else {
                index + 1
            };
            !word(next).is_some_and(|word| AFTER_IS.contains(&word.as_str()))
        };
        sequence || xml_function || using_alias || is_expression
    })
}
