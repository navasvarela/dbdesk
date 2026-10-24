//! How dbdesk refuses changes: for the MCP server's read-only projects, and for the consoles
//! of read-only data sources. One implementation, so both refuse the same statements.
//!
//! Changes are refused twice:
//! 1. [`check`] classifies each statement from its parse tree before it is sent, and refuses
//!    anything that doesn't only read, or can't be read;
//! 2. the statement runs in the database's own read-only mode ([`mode`]): a read-only
//!    transaction, or a read-only connection for SQLite. That is what stops
//!    `SELECT writes_something()`, since the parser can't see inside a function.

use std::time::Duration;

use dbdesk_core::{Connection, ConnectionConfig, DataSource, DriverError, DriverKind};
use dbdesk_sql::{Dialect, StatementKind, Unclassified, classify};

/// How an engine refuses writes on its own, whatever the statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyMode {
    /// A transaction opened with `begin` around the work, and rolled back after it.
    Transaction {
        begin: &'static str,
        /// Whether the engine can also be told a time limit for the transaction, which ends
        /// a runaway statement even if a cancel can't get through.
        sets_timeout: bool,
    },
    /// The connection itself is opened read-only; see [`connection_config`].
    Connection,
}

/// The engine's read-only mode, or `None` when it has none dbdesk can use.
pub fn mode(driver: DriverKind) -> Option<ReadOnlyMode> {
    match driver {
        DriverKind::Postgres => Some(ReadOnlyMode::Transaction {
            begin: "BEGIN READ ONLY",
            sets_timeout: true,
        }),
        DriverKind::Mysql => Some(ReadOnlyMode::Transaction {
            begin: "START TRANSACTION READ ONLY",
            sets_timeout: false,
        }),
        DriverKind::Sqlite => Some(ReadOnlyMode::Connection),
        // SQL Server has no read-only transaction, DuckDB's read-only mode is a choice of the
        // data source, not of the connection MCP would open, and ClickHouse's (the `readonly`
        // setting) isn't wired into its driver yet. The plan (docs/mcp-server.md) has each
        // such engine name its equivalent, such as a read-only login, before MCP serves it.
        DriverKind::SqlServer | DriverKind::DuckDb | DriverKind::ClickHouse => None,
    }
}

/// The settings to connect with: a SQLite file is opened read-only, the engine's own
/// equivalent of a read-only transaction. Other engines connect as usual.
pub fn connection_config(mut data_source: DataSource) -> DataSource {
    if let ConnectionConfig::Sqlite(config) = &mut data_source.config {
        config.read_only = true;
    }
    data_source
}

/// Why [`check`] refused a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The statement changes something.
    Changes(StatementKind),
    /// What the statement does can't be known.
    Unknown(Unclassified),
}

impl Refusal {
    /// What the statement does, as in "this statement changes data", or why that is unknown.
    pub fn reason(&self) -> String {
        match self {
            Self::Changes(kind) => format!("this statement {}", kind.description()),
            Self::Unknown(unclassified) => unclassified.to_string(),
        }
    }
}

/// Accepts `sql`, one statement, only if it only reads.
///
/// # Errors
///
/// [`Refusal`] for a statement that changes anything, controls the transaction, or can't be
/// classified.
pub fn check(sql: &str, dialect: Dialect) -> Result<(), Refusal> {
    match classify(sql, dialect) {
        Ok(StatementKind::Read) => Ok(()),
        Ok(kind) => Err(Refusal::Changes(kind)),
        Err(unclassified) => Err(Refusal::Unknown(unclassified)),
    }
}

/// Opens the read-only transaction, if the engine uses one, with `timeout` as its time limit
/// where the engine can be told one.
///
/// # Errors
///
/// [`DriverError`] if the database refuses to open it.
pub async fn begin(
    connection: &mut dyn Connection,
    mode: ReadOnlyMode,
    timeout: Option<Duration>,
) -> Result<(), DriverError> {
    let ReadOnlyMode::Transaction {
        begin,
        sets_timeout,
    } = mode
    else {
        return Ok(());
    };
    run_control(connection, begin).await?;
    if let Some(timeout) = timeout.filter(|_| sets_timeout) {
        let set = format!("SET LOCAL statement_timeout = {}", timeout.as_millis());
        run_control(connection, &set).await?;
    }
    Ok(())
}

/// Rolls back what [`begin`] opened.
///
/// # Errors
///
/// [`DriverError`] if the rollback fails, which leaves the connection in an unknown state.
pub async fn rollback(
    connection: &mut dyn Connection,
    mode: ReadOnlyMode,
) -> Result<(), DriverError> {
    match mode {
        ReadOnlyMode::Transaction { .. } => run_control(connection, "ROLLBACK").await,
        ReadOnlyMode::Connection => Ok(()),
    }
}

async fn run_control(connection: &mut dyn Connection, sql: &str) -> Result<(), DriverError> {
    connection.execute(sql).await?.finish().await.map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_whatever_does_not_only_read() {
        let cases = [
            ("SELECT * FROM orders", None),
            (
                "INSERT INTO orders VALUES (1)",
                Some(Refusal::Changes(StatementKind::Write)),
            ),
            (
                "CREATE TABLE t (id int)",
                Some(Refusal::Changes(StatementKind::Schema)),
            ),
            ("BEGIN", Some(Refusal::Changes(StatementKind::Transaction))),
            (
                "SET search_path TO x",
                Some(Refusal::Changes(StatementKind::Admin)),
            ),
        ];
        for (sql, expected) in cases {
            assert_eq!(check(sql, Dialect::Postgres).err(), expected, "{sql}");
        }
        assert!(matches!(
            check("SELEC nonsense", Dialect::Postgres),
            Err(Refusal::Unknown(_))
        ));
        assert_eq!(
            Refusal::Changes(StatementKind::Write).reason(),
            "this statement changes data"
        );
    }
}
