//! Running work inside the database's own read-only mode, with a time limit.
//!
//! Every call is wrapped the same way:
//! 1. start the engine's read-only transaction ([`begin`]);
//! 2. run the work under a watchdog that cancels it at the time limit ([`watched`]);
//! 3. roll back, whatever happened ([`end`]).
//!
//! The parser has already refused statements that don't read, but it can't see inside a
//! function; the read-only transaction is what stops `SELECT writes_something()`.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use dbdesk_core::{
    CancelHandle, ColumnInfo, Connection, DriverError, DriverKind, Row, describe_error,
};

/// How long a cancelled statement gets to give up before its connection is given up on.
const CANCEL_GRACE: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub(crate) enum RunError {
    #[error(transparent)]
    Driver(#[from] DriverError),
    #[error("cancelled after the statement timeout of {0:?}")]
    TimedOut(Duration),
}

impl RunError {
    /// Whether the connection should be closed rather than reused: it failed, or a statement
    /// on it may still be running.
    pub(crate) fn breaks_connection(&self) -> bool {
        matches!(
            self,
            Self::TimedOut(_) | Self::Driver(DriverError::Connection(_))
        )
    }
}

/// A query's rows, cut at the row limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rows {
    pub(crate) columns: Vec<ColumnInfo>,
    pub(crate) rows: Vec<Row>,
    /// More rows were left unread.
    pub(crate) truncated: bool,
    /// Rows changed or returned, as the database reports it, when all were read.
    pub(crate) row_count: Option<u64>,
}

/// Starts the engine's read-only transaction; on SQLite the connection itself was opened
/// read-only. On PostgreSQL the time limit is also set on the server, which ends a runaway
/// statement even if the cancel can't get through.
pub(crate) async fn begin(
    connection: &mut dyn Connection,
    driver: DriverKind,
    timeout: Duration,
) -> Result<(), DriverError> {
    let statements = match driver {
        DriverKind::Postgres => vec![
            "BEGIN READ ONLY".to_owned(),
            format!("SET LOCAL statement_timeout = {}", timeout.as_millis()),
        ],
        DriverKind::Mysql => vec!["START TRANSACTION READ ONLY".to_owned()],
        DriverKind::Sqlite => vec![],
    };
    for statement in statements {
        run_control(connection, &statement).await?;
    }
    Ok(())
}

/// Rolls back what [`begin`] started, and hands back the work's outcome. A failed rollback
/// leaves the connection in an unknown state, so it counts as a broken connection.
pub(crate) async fn end<T>(
    connection: &mut dyn Connection,
    driver: DriverKind,
    outcome: Result<T, RunError>,
) -> Result<T, RunError> {
    let rollback = match driver {
        DriverKind::Postgres | DriverKind::Mysql => run_control(connection, "ROLLBACK").await,
        DriverKind::Sqlite => Ok(()),
    };
    match (outcome, rollback) {
        (outcome, Ok(())) => outcome,
        (Err(error), Err(rollback)) => {
            tracing::warn!("the rollback failed too: {}", describe_error(&rollback));
            Err(error)
        }
        (Ok(_), Err(rollback)) => Err(RunError::Driver(DriverError::Connection(Box::new(
            rollback,
        )))),
    }
}

async fn run_control(connection: &mut dyn Connection, sql: &str) -> Result<(), DriverError> {
    connection.execute(sql).await?.finish().await.map(drop)
}

/// Runs `work`, cancelling it through `cancel` once `timeout` has passed.
pub(crate) async fn watched<T>(
    cancel: Arc<dyn CancelHandle>,
    timeout: Duration,
    work: impl Future<Output = Result<T, DriverError>>,
) -> Result<T, RunError> {
    let mut work = std::pin::pin!(work);
    tokio::select! {
        outcome = &mut work => return outcome.map_err(RunError::Driver),
        () = tokio::time::sleep(timeout) => {}
    }
    if let Err(error) = cancel.cancel().await {
        tracing::warn!("could not cancel the statement: {}", describe_error(&error));
    }
    // Whatever the work returns now, it ended because of the cancel. If it doesn't end, it is
    // dropped, and the caller closes the connection it was running on.
    let _ = tokio::time::timeout(CANCEL_GRACE, work).await;
    Err(RunError::TimedOut(timeout))
}

/// Runs a statement and reads up to `max_rows` of its rows.
pub(crate) async fn fetch(
    connection: &mut dyn Connection,
    sql: &str,
    max_rows: usize,
) -> Result<Rows, DriverError> {
    let mut cursor = connection.execute(sql).await?;
    let columns = cursor.columns().to_vec();
    // One row past the limit tells "exactly max_rows" from "more than max_rows", which a
    // cursor can't always tell before it is asked for another row.
    let wanted = max_rows.saturating_add(1);
    let mut rows = Vec::new();
    while rows.len() < wanted && !cursor.is_exhausted() {
        let page = cursor.next_page(wanted - rows.len()).await?;
        if page.is_empty() {
            break;
        }
        rows.extend(page);
    }
    let truncated = rows.len() > max_rows;
    rows.truncate(max_rows);
    // Unread rows are abandoned with the cursor rather than read to the end; the rollback
    // that follows ends the statement either way.
    let row_count = if truncated {
        None
    } else {
        cursor.finish().await?
    };
    Ok(Rows {
        columns,
        rows,
        truncated,
        row_count,
    })
}
