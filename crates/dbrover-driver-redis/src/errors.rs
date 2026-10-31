//! The client's and the server's errors as [`DriverError`]s.

use dbrover_core::{BoxError, DriverError};
use redis::RedisError;

use crate::reply;

/// An error the server answered a command with, as `redis-cli` prints it: its code, then its
/// message (`WRONGTYPE Operation against a key holding the wrong kind of value`).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct ServerFailure(pub(crate) String);

pub(crate) fn connect(error: impl Into<BoxError>) -> DriverError {
    DriverError::Connect(error.into())
}

pub(crate) fn statement(error: impl Into<BoxError>) -> DriverError {
    DriverError::Statement {
        source: error.into(),
        position: None,
        sqlstate: None,
    }
}

/// The server refused a command.
pub(crate) fn server(error: &redis::ServerError) -> DriverError {
    statement(ServerFailure(reply::error_text(error)))
}

/// A command that failed in the client or on the way: [`DriverError::Lost`] when the connection
/// is gone, since the client never reconnects a connection by itself.
pub(crate) fn command(error: RedisError) -> DriverError {
    if error.is_connection_dropped() || error.is_io_error() || error.is_unrecoverable_error() {
        DriverError::Lost(error.into())
    } else if let Some(code) = error.code() {
        let text = match error.detail() {
            Some(detail) => format!("{code} {detail}"),
            None => code.to_owned(),
        };
        statement(ServerFailure(text))
    } else {
        DriverError::Connection(error.into())
    }
}
