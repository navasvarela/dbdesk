//! Server errors as dbdesk errors: where in the statement they point, and what kind of
//! rejection they are.
//!
//! SQL Server reports the line of an error and, in its message, the token it stopped at or
//! the name it couldn't find (`Incorrect syntax near the keyword 'WHERE'.`, `Invalid object
//! name 'orders'.`), but not the column. The token may appear more than once on that line, and
//! which one the server means is found with `dbdesk-sql`'s parser: SQL Server stops at the
//! first token it can't read, and `sqlparser` reads at least as far (it is more lenient: it
//! takes `WHERE WHERE id` for a condition on a column called `WHERE`, and stops at `id`), so
//! for a syntax error the server's token is the last appearance at or before where the parser
//! stops. Otherwise, and for names that don't exist, it is the first appearance.

use dbdesk_core::{DriverError, Rejection, RejectionKind};
use dbdesk_sql::{Dialect, ProblemSource, TokenKind, check_statement, tokenize};
use tiberius::error::{Error, TokenError};

/// `Incorrect syntax near …` (102), `… near the keyword …` (156), an unclosed quotation mark
/// (105), and `'…' is not a recognized … option` (195 is a function, below).
const SYNTAX_ERRORS: &[u32] = &[102, 105, 156];

/// Errors that name something that doesn't exist: an unknown column (207), object (208),
/// built-in function (195), database (911), procedure (2812), function (4121), or a
/// multi-part name that binds to nothing (4104).
const UNKNOWN_NAMES: &[u32] = &[195, 207, 208, 911, 2812, 4104, 4121];

/// The server's message, without `tiberius`'s wrapping (server name, line, state and class),
/// which a person reading the console doesn't need.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct ServerError {
    message: String,
    code: u32,
}

/// A failed user statement, with the byte offset of the error when it can be placed.
pub(crate) fn statement_error(error: Error, sql: &str) -> DriverError {
    match error {
        Error::Server(token) => DriverError::Statement {
            position: position(&token, sql),
            source: Box::new(ServerError {
                message: token.message().to_owned(),
                code: token.code(),
            }),
        },
        other => DriverError::Statement {
            source: Box::new(other),
            position: None,
        },
    }
}

/// What compiling a statement found wrong with it.
pub(crate) fn rejection(error: &TokenError, sql: &str) -> Rejection {
    let kind = if SYNTAX_ERRORS.contains(&error.code()) {
        RejectionKind::Syntax
    } else if UNKNOWN_NAMES.contains(&error.code()) {
        RejectionKind::UnknownName
    } else {
        RejectionKind::Other
    };
    Rejection {
        message: error.message().to_owned(),
        position: position(error, sql),
        kind,
    }
}

fn position(error: &TokenError, sql: &str) -> Option<usize> {
    let named = quoted_name(error.message())?;
    let line = usize::try_from(error.line()).ok()?;
    let parser_stop = SYNTAX_ERRORS
        .contains(&error.code())
        .then(|| where_the_parser_stops(sql))
        .flatten();
    find_on_line(sql, line, &named, parser_stop)
}

/// The first `'…'` in a message: the token or name it is about. A quote inside it is doubled,
/// as in SQL.
fn quoted_name(message: &str) -> Option<String> {
    let (_, after) = message.split_once('\'')?;
    let mut name = String::new();
    let mut chars = after.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() != Some(&'\'') {
                return Some(name);
            }
            chars.next();
        }
        name.push(c);
    }
    None
}

/// Where `dbdesk-sql`'s parser finds a syntax error in `sql`, if it parses this kind of
/// statement (data queries and changes) and finds one.
fn where_the_parser_stops(sql: &str) -> Option<usize> {
    check_statement(sql, Dialect::SqlServer)
        .into_iter()
        .find(|problem| problem.source == ProblemSource::Syntax)
        .map(|problem| problem.range.start)
}

/// A token on 1-based `line` that reads `name`, ignoring case and brackets (SQL Server names
/// `[order]` as `order`): the last one at or before `before` if there is one, else the first.
/// A name of several parts (`dbo.orders`) is found by its first.
fn find_on_line(sql: &str, line: usize, name: &str, before: Option<usize>) -> Option<usize> {
    let line_start = if line <= 1 {
        0
    } else {
        sql.match_indices('\n').nth(line - 2)?.0 + 1
    };
    let line_end = sql[line_start..]
        .find('\n')
        .map_or(sql.len(), |end| line_start + end);
    let first_part = name.split('.').next().unwrap_or(name);
    let unbracketed = |text: &str| {
        text.strip_prefix('[')
            .and_then(|text| text.strip_suffix(']'))
            .map_or_else(|| text.to_owned(), |text| text.replace("]]", "]"))
    };
    let starts: Vec<usize> = tokenize(sql, Dialect::SqlServer)
        .into_iter()
        .filter(|token| token.range.start >= line_start && token.range.end <= line_end)
        .filter(|token| !token.kind.is_trivia() && token.kind != TokenKind::String)
        .filter(|token| unbracketed(&sql[token.range.clone()]).eq_ignore_ascii_case(first_part))
        .map(|token| token.range.start)
        .collect();
    before
        .and_then(|before| starts.iter().rev().find(|&&start| start <= before))
        .or_else(|| starts.first())
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_errors_at_the_token_they_name() {
        let cases = [
            (
                "SELECT id FROM t WHERE WHERE id = 1",
                "Incorrect syntax near the keyword 'WHERE'.",
                1,
                Some("SELECT id FROM t WHERE ".len()),
            ),
            (
                "SELECT n n n FROM t",
                "Incorrect syntax near 'n'.",
                1,
                Some("SELECT n n ".len()),
            ),
            (
                "SELECT a\nFROM [no such] x",
                "Invalid object name 'no such'.",
                2,
                Some("SELECT a\nFROM ".len()),
            ),
            (
                "SELECT 'missing', missing FROM t",
                "Invalid column name 'missing'.",
                1,
                Some("SELECT 'missing', ".len()),
            ),
            ("SELECT 1", "Arithmetic overflow error.", 1, None),
        ];
        for (sql, message, line, expected) in cases {
            let syntax = message.starts_with("Incorrect syntax");
            let found = quoted_name(message).and_then(|named| {
                let stop = syntax.then(|| where_the_parser_stops(sql)).flatten();
                find_on_line(sql, line, &named, stop)
            });
            assert_eq!(found, expected, "{sql:?}");
        }
    }

    #[test]
    fn reads_the_quoted_name_with_doubled_quotes() {
        assert_eq!(
            quoted_name("Invalid object name 'it''s'."),
            Some("it's".to_owned())
        );
        assert_eq!(quoted_name("No quotes."), None);
    }
}
