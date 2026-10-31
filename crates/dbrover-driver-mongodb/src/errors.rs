//! The driver's errors as DbRover's: which mean the connection is gone, and which belong to the
//! command that was run.

use dbrover_core::{BoxError, DriverError};
use dbrover_language::mongo::ParseError;
use mongodb::error::{Error, ErrorKind};

/// A failure while connecting.
pub(crate) fn connect(error: impl Into<BoxError>) -> DriverError {
    DriverError::Connect(error.into())
}

/// A failure of an established connection: the server can't be reached any more, so nothing
/// more can be done with it until it answers again; anything else is the command's.
pub(crate) fn command(error: Error) -> DriverError {
    if is_lost(&error) {
        DriverError::Lost(Box::new(error))
    } else {
        statement(Server(error), None)
    }
}

/// A failure reading the catalog, where a refusal is the connection's rather than a command's.
pub(crate) fn catalog(error: Error) -> DriverError {
    if is_lost(&error) {
        DriverError::Lost(Box::new(error))
    } else {
        DriverError::Connection(Box::new(Server(error)))
    }
}

fn is_lost(error: &Error) -> bool {
    matches!(
        *error.kind,
        ErrorKind::Io(_)
            | ErrorKind::ConnectionPoolCleared { .. }
            | ErrorKind::ServerSelection { .. }
    )
}

/// A command DbRover couldn't read or send, pointing at where in it the trouble is.
pub(crate) fn parse(error: ParseError) -> DriverError {
    let position = error.range.start;
    statement(error, Some(position))
}

pub(crate) fn statement(error: impl Into<BoxError>, position: Option<usize>) -> DriverError {
    DriverError::Statement {
        source: error.into(),
        position,
        sqlstate: None,
    }
}

/// The server's own message for a command it refused, `errmsg` with its code name, rather than
/// the driver's wording around it.
#[derive(Debug, thiserror::Error)]
#[error("{}", message(.0))]
pub(crate) struct Server(pub(crate) Error);

fn message(error: &Error) -> String {
    match &*error.kind {
        ErrorKind::Command(command) if command.code_name.is_empty() => command.message.clone(),
        ErrorKind::Command(command) => format!("{} ({})", command.message, command.code_name),
        _ => error.to_string(),
    }
}
