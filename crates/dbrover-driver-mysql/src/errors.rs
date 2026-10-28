//! Server errors as DbRover errors: where in the statement they point, and what kind of
//! rejection they are.

use dbrover_core::{DriverError, Rejection, RejectionKind};

/// `ER_PARSE_ERROR`: the statement can't be read.
const PARSE_ERROR: u16 = 1064;

/// Errors that name something that doesn't exist: an unknown database (1049), column (1054),
/// table (1146) or function (1305).
const UNKNOWN_NAMES: &[u16] = &[1049, 1054, 1146, 1305];

/// Errors the server sends as it ends a session: it is shutting down (1053, MariaDB's 1927
/// for a killed connection), the client's connection went away or was lost (2006, 2013), or
/// it was idle past `wait_timeout` (MySQL 8's 4031).
const SESSION_ENDED: &[u16] = &[1053, 1927, 2006, 2013, 4031];

/// Whether `error` means the session is over: the socket failed or closed, or the server
/// said it ended the session.
pub(crate) fn is_lost(error: &mysql_async::Error) -> bool {
    match error {
        mysql_async::Error::Io(_)
        | mysql_async::Error::Driver(mysql_async::DriverError::ConnectionClosed) => true,
        mysql_async::Error::Server(server) => SESSION_ENDED.contains(&server.code),
        _ => false,
    }
}

/// A failed user statement, with the byte offset of the error when the server says where.
pub(crate) fn statement_error(error: mysql_async::Error, sql: &str) -> DriverError {
    if is_lost(&error) {
        return DriverError::Lost(Box::new(error));
    }
    let (position, sqlstate) = match &error {
        mysql_async::Error::Server(server) => {
            (position(&server.message, sql), Some(server.state.clone()))
        }
        _ => (None, None),
    };
    DriverError::Statement {
        source: Box::new(error),
        position,
        sqlstate,
    }
}

/// What preparing a statement found wrong with it.
pub(crate) fn rejection(error: &mysql_async::ServerError, sql: &str) -> Rejection {
    let kind = if error.code == PARSE_ERROR {
        RejectionKind::Syntax
    } else if UNKNOWN_NAMES.contains(&error.code) {
        RejectionKind::UnknownName
    } else {
        RejectionKind::Other
    };
    Rejection {
        message: error.message.clone(),
        position: position(&error.message, sql),
        kind,
    }
}

/// The offset MySQL means by `… near 'rest of the statement' at line 3`: it quotes the text
/// from the error onwards, cut to its first 80 characters, so the quote is a suffix of the
/// statement when it is short, and otherwise the text at the error's position on that line.
fn position(message: &str, sql: &str) -> Option<usize> {
    let (_, after_near) = message.split_once(" near '")?;
    let (snippet, line) = after_near.rsplit_once("' at line ")?;
    let line: usize = line.trim().parse().ok()?;
    if sql.ends_with(snippet) {
        return Some(sql.len() - snippet.len());
    }
    let line_start = if line <= 1 {
        0
    } else {
        sql.match_indices('\n').nth(line - 2)?.0 + 1
    };
    sql[line_start..]
        .find(snippet)
        .map(|offset| line_start + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syntax_error(near: &str, line: usize) -> String {
        format!(
            "You have an error in your SQL syntax; check the manual that corresponds to your \
             MySQL server version for the right syntax to use near '{near}' at line {line}"
        )
    }

    #[test]
    fn finds_the_text_a_syntax_error_quotes() {
        let long_tail = "x, ".repeat(40);
        let long = format!("SELECT a\nFROM t WHERE WHERE {long_tail}");
        let cases = [
            (
                "SELECT id FROM t WHERE WHERE id = 1",
                syntax_error("WHERE id = 1", 1),
                Some("SELECT id FROM t WHERE ".len()),
            ),
            ("SELECT n n n FROM t", syntax_error("n FROM t", 1), Some(11)),
            (
                "SELECT 1 FROM",
                syntax_error("", 1),
                Some("SELECT 1 FROM".len()),
            ),
            (
                long.as_str(),
                syntax_error(&format!("WHERE {}", &long_tail[..74]), 2),
                Some("SELECT a\nFROM t WHERE ".len()),
            ),
            ("SELECT 1", "Table 'shop.t' doesn't exist".to_owned(), None),
        ];
        for (sql, message, expected) in cases {
            assert_eq!(position(&message, sql), expected, "{sql:?}");
        }
    }
}
