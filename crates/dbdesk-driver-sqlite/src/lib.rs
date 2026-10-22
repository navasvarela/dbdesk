//! SQLite driver for dbdesk, built on `rusqlite`.
//!
//! SQLite calls block, so every one runs on tokio's blocking pool and never stalls the
//! engine's async workers. Must run inside a tokio runtime.

use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use dbdesk_core::{BoxError, Connection, DriverError, ServerInfo, SqliteConfig};
use rusqlite::OpenFlags;

const PRODUCT: &str = "SQLite";

/// Opens an existing database file.
///
/// Never creates the file: a typo in the path would otherwise silently produce a new, empty
/// database and the user would wonder where their tables went.
///
/// # Errors
///
/// [`DriverError::Connect`] if the file doesn't exist, can't be opened, or isn't a SQLite
/// database.
pub async fn connect(config: &SqliteConfig) -> Result<SqliteConnection, DriverError> {
    let path = config.path.clone();
    let read_only = config.read_only;
    let (connection, version) = tokio::task::spawn_blocking(move || open(&path, read_only))
        .await
        .map_err(|join_error| DriverError::Connect(Box::new(join_error)))?
        .map_err(|error| DriverError::Connect(Box::new(error)))?;

    Ok(SqliteConnection {
        connection: Arc::new(Mutex::new(connection)),
        server_info: ServerInfo {
            product: PRODUCT.to_owned(),
            version,
        },
    })
}

fn open(path: &Path, read_only: bool) -> rusqlite::Result<(rusqlite::Connection, String)> {
    let access = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let connection = rusqlite::Connection::open_with_flags(
        path,
        access | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // SQLite opens lazily and accepts any file here; the first query is what fails on a
    // file that isn't a database. Asking for the version doubles as that check.
    let version = connection.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    Ok((connection, version))
}

pub struct SqliteConnection {
    /// Behind a mutex only so it can move into blocking tasks; there is one user at a time.
    connection: Arc<Mutex<rusqlite::Connection>>,
    server_info: ServerInfo,
}

impl SqliteConnection {
    async fn run_blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> Result<T, DriverError> {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().map_err(|_| poisoned())?;
            work(&connection).map_err(connection_error)
        })
        .await
        .map_err(connection_error)?
    }
}

fn connection_error(error: impl Into<BoxError>) -> DriverError {
    DriverError::Connection(error.into())
}

fn poisoned() -> DriverError {
    connection_error("a previous operation on this connection panicked")
}

#[async_trait]
impl Connection for SqliteConnection {
    fn server_info(&self) -> &ServerInfo {
        &self.server_info
    }

    async fn ping(&mut self) -> Result<(), DriverError> {
        self.run_blocking(|connection| connection.query_row("SELECT 1", [], |_| Ok(())))
            .await
    }

    async fn close(self: Box<Self>) -> Result<(), DriverError> {
        let Ok(mutex) = Arc::try_unwrap(self.connection) else {
            // Only `run_blocking` clones the Arc, and it has finished by the time `close`
            // can be called, so this is unreachable. Dropping the handle still closes.
            return Ok(());
        };
        let connection = mutex.into_inner().map_err(|_| poisoned())?;
        tokio::task::spawn_blocking(move || connection.close().map_err(|(_, error)| error))
            .await
            .map_err(connection_error)?
            .map_err(connection_error)
    }
}

#[cfg(test)]
mod tests {
    use dbdesk_core::conformance;

    use super::*;

    fn database_file() -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        rusqlite::Connection::open(file.path())
            .unwrap()
            .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY)")
            .unwrap();
        file
    }

    #[tokio::test]
    async fn conforms_to_the_connection_lifecycle() {
        for read_only in [false, true] {
            let file = database_file();
            let config = SqliteConfig {
                path: file.path().to_owned(),
                read_only,
            };

            let connection = connect(&config).await.unwrap();

            assert_eq!(connection.server_info().product, PRODUCT);
            conformance::check_lifecycle(Box::new(connection)).await;
        }
    }

    #[tokio::test]
    async fn missing_file_is_a_connect_error_and_is_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.db");

        let result = connect(&SqliteConfig {
            path: path.clone(),
            read_only: false,
        })
        .await;

        conformance::check_connect_failure(result);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn non_database_file_is_a_connect_error() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "not a database, just some text").unwrap();

        let result = connect(&SqliteConfig {
            path: file.path().to_owned(),
            read_only: false,
        })
        .await;

        conformance::check_connect_failure(result);
    }
}
