//! What the server tells the log and the query history about each call, so the user can see
//! afterwards what an assistant did and what it was refused.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dbdesk_core::{DataSource, describe_error};
use dbdesk_sql::StatementKind;
use dbdesk_store::{History, HistoryEntry, HistoryFile, HistoryOutcome, HistorySource};
use jiff::Timestamp;

pub(crate) struct Audit {
    project: String,
    /// `None` when the history couldn't be opened; calls still work, and the log says why.
    history: Option<Arc<Mutex<History>>>,
}

/// A statement that reached the database, for the history.
pub(crate) struct Ran<'a> {
    pub(crate) data_source: &'a DataSource,
    pub(crate) sql: &'a str,
    pub(crate) started: Timestamp,
    pub(crate) elapsed: Duration,
    pub(crate) rows: Option<u64>,
    pub(crate) outcome: HistoryOutcome,
}

impl Audit {
    pub(crate) fn open(project: &str, history: &HistoryFile) -> Self {
        let history = match history.open() {
            Ok(history) => Some(Arc::new(Mutex::new(history))),
            Err(error) => {
                tracing::warn!(
                    project,
                    "statements won't be recorded in the query history: {}",
                    describe_error(&error)
                );
                None
            }
        };
        Self {
            project: project.to_owned(),
            history,
        }
    }

    /// One info line per call: who asked for what, and what kind of statement it was.
    pub(crate) fn call(&self, tool: &str, data_source: Option<&str>, kind: Option<StatementKind>) {
        tracing::info!(
            project = %self.project,
            data_source = data_source.unwrap_or("-"),
            tool,
            kind = kind.map_or("-", StatementKind::description),
            "MCP call"
        );
    }

    /// A call refused before it reached the database, with the reason.
    pub(crate) fn refused(&self, tool: &str, data_source: &str, reason: &str) {
        tracing::warn!(
            project = %self.project,
            data_source,
            tool,
            "MCP call refused: {reason}"
        );
    }

    /// A call the database failed or refused. [`READ_ONLY_VIOLATION`] is what a read-only
    /// transaction raises for a write it caught, which the parser couldn't see.
    pub(crate) fn failed(
        &self,
        tool: &str,
        data_source: &str,
        message: &str,
        sqlstate: Option<&str>,
    ) {
        if sqlstate == Some(READ_ONLY_VIOLATION) {
            tracing::warn!(
                project = %self.project,
                data_source,
                tool,
                sqlstate,
                "MCP call refused by the read-only transaction: {message}"
            );
        } else {
            tracing::warn!(
                project = %self.project,
                data_source,
                tool,
                sqlstate = sqlstate.unwrap_or("-"),
                "MCP call failed: {message}"
            );
        }
    }

    /// Adds the statement to the query history, marked as coming from MCP. Failing to only
    /// costs the history entry, so it is logged, not reported to the client.
    pub(crate) async fn record(&self, ran: Ran<'_>) {
        let Some(history) = &self.history else {
            return;
        };
        let entry = HistoryEntry {
            data_source: ran.data_source.id,
            executed_at: ran.started,
            sql: ran.sql.to_owned(),
            duration: Some(ran.elapsed),
            rows: ran.rows,
            outcome: ran.outcome,
            source: HistorySource::Mcp,
        };
        let history = Arc::clone(history);
        let written = tokio::task::spawn_blocking(move || {
            history
                .lock()
                .map_err(|_| "the history lock was poisoned".to_owned())?
                .record(&entry)
                .map_err(|error| describe_error(&error))
        })
        .await;
        let error = match written {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error,
            Err(error) => describe_error(&error),
        };
        tracing::warn!(project = %self.project, "could not record the statement: {error}");
    }
}

/// `read_only_sql_transaction` on PostgreSQL, and what MySQL and MariaDB report for
/// `ER_CANT_EXECUTE_IN_READ_ONLY_TRANSACTION`.
const READ_ONLY_VIOLATION: &str = "25006";
