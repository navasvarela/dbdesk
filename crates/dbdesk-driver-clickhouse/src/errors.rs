//! ClickHouse's errors as dbdesk errors: their message, where in the statement they point, and
//! what kind of rejection they are.
//!
//! ClickHouse writes an error as `Code: 62. DB::Exception: Syntax error: failed at position 39
//! (id): … (SYNTAX_ERROR) (version 25.8.33.6 (official build))`, the position counting
//! characters from 1.

use dbdesk_core::{DriverError, Rejection, RejectionKind};

/// The error code of a statement ClickHouse can't read.
const SYNTAX_ERROR: u32 = 62;

/// Codes of errors that name something that doesn't exist: a column (16), function (46),
/// identifier (47), table (60) or database (81).
const UNKNOWN_NAMES: &[u32] = &[16, 46, 47, 60, 81];

/// `SESSION_NOT_FOUND`: the server has no session of that id, as after it restarts.
const SESSION_NOT_FOUND: u32 = 372;

/// Whether a server error means the connection's session is gone.
pub(crate) fn ends_the_session(response: &str) -> bool {
    code(&message(response)) == Some(SESSION_NOT_FOUND)
}

/// ClickHouse's message without the server's version.
#[derive(Debug)]
pub(crate) struct ClickHouseError(String);

impl std::fmt::Display for ClickHouseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ClickHouseError {}

/// A failed user statement, from ClickHouse's `response` to it, with the byte offset of the error when ClickHouse says where.
/// `offset` is how many bytes the request added before the statement (an `EXPLAIN PLAN `), to
/// take off the position.
pub(crate) fn statement_error(response: &str, sql: &str, offset: usize) -> DriverError {
    let message = message(response);
    DriverError::Statement {
        position: position(&message, sql, offset),
        source: Box::new(ClickHouseError(message)),
        sqlstate: None,
    }
}

/// What ClickHouse found wrong with a statement it was asked to explain.
pub(crate) fn rejection(response: &str, sql: &str, offset: usize) -> Rejection {
    let message = message(response);
    let kind = match code(&message) {
        Some(SYNTAX_ERROR) => RejectionKind::Syntax,
        Some(code) if UNKNOWN_NAMES.contains(&code) => RejectionKind::UnknownName,
        _ => RejectionKind::Other,
    };
    Rejection {
        position: position(&message, sql, offset),
        message,
        kind,
    }
}

/// A statement stopped by a cancel.
pub(crate) fn cancelled() -> DriverError {
    DriverError::Statement {
        source: Box::new(ClickHouseError("the query was cancelled".to_owned())),
        position: None,
        sqlstate: None,
    }
}

fn message(response: &str) -> String {
    response
        .rsplit_once(" (version ")
        .map_or(response, |(message, _)| message)
        .trim()
        .to_owned()
}

/// `62` from `Code: 62. DB::Exception: …`.
fn code(message: &str) -> Option<u32> {
    message
        .strip_prefix("Code: ")?
        .split_once('.')?
        .0
        .parse()
        .ok()
}

/// The byte offset in `sql` of ClickHouse's `failed at position N`, a character count from 1
/// in what was sent, `offset` bytes of which came before `sql`.
fn position(message: &str, sql: &str, offset: usize) -> Option<usize> {
    let (_, after) = message.split_once("failed at position ")?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    let character: usize = digits.parse::<usize>().ok()?.checked_sub(1)?;
    let character = character.checked_sub(offset)?;
    Some(
        sql.char_indices()
            .nth(character)
            .map_or(sql.len(), |(byte, _)| byte),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_the_server_has_forgotten_is_over() {
        assert!(ends_the_session(
            "Code: 372. DB::Exception: Session 1b2c not found. (SESSION_NOT_FOUND) (version 25.8)"
        ));
        assert!(!ends_the_session("Code: 60. DB::Exception: Unknown table"));
    }

    #[test]
    fn reads_the_code_and_the_position() {
        let message = "Code: 62. DB::Exception: Syntax error: failed at position 25 (n): n FROM v";
        assert_eq!(code(message), Some(62));
        assert_eq!(position(message, "SELECT n n n FROM v", 13), Some(11));
        assert_eq!(
            position(message, "EXPLAIN PLAN SELECT n n n FROM v", 0),
            Some(24)
        );
        assert_eq!(
            position("Code: 60. DB::Exception: Unknown table", "SELECT 1", 0),
            None
        );
        let accented = "Code: 62. DB::Exception: Syntax error: failed at position 12 (=)";
        assert_eq!(
            position(accented, "SELECT 'é' = = 1", 0),
            Some("SELECT 'é' ".len())
        );
    }
}
