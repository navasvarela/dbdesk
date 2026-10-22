//! Running a console's statements: one page of rows per statement, and the last statement's
//! cursor kept open ("parked") so the UI can fetch more.
//!
//! Only the last statement stays open because a cursor holds the connection: earlier
//! statements' unread rows are discarded so the next statement can run. Their results are
//! marked truncated.

use std::time::{Duration, Instant};

use dbdesk_core::{CancelHandle, DriverError, ResultCursor, describe_error};
use dbdesk_store::{HistoryEntry, HistoryOutcome};
use jiff::Timestamp;

use super::{Disconnected, SessionContext, Worker, cancel_running_statement};
use crate::{Event, RunId, RunOutcome, StatementFailure, StatementSummary};

/// Rows per page: DataGrip's default. Plenty to look at, few enough to arrive at once.
const PAGE_SIZE: usize = 500;
/// How much of a statement the log console shows.
const LOG_PREVIEW_CHARS: usize = 80;

/// Which statement of which run an event or history entry is about.
#[derive(Debug, Clone)]
struct StatementRef {
    run: RunId,
    index: usize,
    sql: String,
    started_at: Timestamp,
}

/// A statement with rows still unread, holding its connection until finished.
pub(super) struct Parked {
    statement: StatementRef,
    cursor: Box<dyn ResultCursor>,
    /// Time until the first page arrived, reported as the statement's duration.
    elapsed: Duration,
}

impl Parked {
    pub(super) fn run(&self) -> RunId {
        self.statement.run
    }
}

enum StatementEnd {
    Completed,
    Parked(Parked),
    Failed,
}

pub(super) async fn run_script(
    worker: &mut Worker<'_>,
    run: RunId,
    statements: &[String],
) -> Result<Option<Parked>, Disconnected> {
    let started = Instant::now();
    let last = statements.len().saturating_sub(1);
    let mut outcome = RunOutcome::Completed;
    let mut parked = None;
    for (index, sql) in statements.iter().enumerate() {
        // A cancel that arrives between statements stops the script before the next one.
        if worker.signals.is_cancelled(run) {
            outcome = RunOutcome::Cancelled;
            break;
        }
        tracing::info!(data_source = %worker.context.name(), "running {}", preview(sql));
        let statement = StatementRef {
            run,
            index,
            sql: sql.clone(),
            started_at: Timestamp::now(),
        };
        match run_statement(worker, statement, index == last).await? {
            StatementEnd::Completed => {}
            StatementEnd::Parked(statement) => parked = Some(statement),
            StatementEnd::Failed if worker.signals.is_cancelled(run) => {
                outcome = RunOutcome::Cancelled;
                break;
            }
            StatementEnd::Failed => {
                outcome = RunOutcome::Failed;
                break;
            }
        }
    }
    worker.context.send(Event::RunFinished {
        run,
        outcome,
        elapsed: started.elapsed(),
    });
    Ok(parked)
}

async fn run_statement(
    worker: &mut Worker<'_>,
    statement: StatementRef,
    is_last: bool,
) -> Result<StatementEnd, Disconnected> {
    let run = statement.run;
    let started = Instant::now();
    let execute = worker.connection.execute(&statement.sql);
    let mut cursor = match worker
        .signals
        .guard(execute, Some(run), worker.cancel_handle)
        .await?
    {
        Ok(cursor) => cursor,
        Err(error) => {
            report_failure(worker, &statement, started.elapsed(), &error);
            return Ok(StatementEnd::Failed);
        }
    };
    if cursor.columns().is_empty() {
        return finish(worker, statement, cursor, started, None).await;
    }

    worker.context.send(Event::StatementStarted {
        run,
        statement: statement.index,
        columns: cursor.columns().to_vec(),
    });
    let page = cursor.next_page(PAGE_SIZE);
    let rows = match worker
        .signals
        .guard(page, Some(run), worker.cancel_handle)
        .await?
    {
        Ok(rows) => rows,
        Err(error) => {
            // The statement already failed; whatever finishing reports adds nothing.
            let _ = cursor.finish().await;
            report_failure(worker, &statement, started.elapsed(), &error);
            return Ok(StatementEnd::Failed);
        }
    };
    let elapsed = started.elapsed();
    let more = !cursor.is_exhausted();
    worker.context.send(Event::RowsFetched {
        run,
        statement: statement.index,
        rows,
        more: more && is_last,
    });
    if more && is_last {
        return Ok(StatementEnd::Parked(Parked {
            statement,
            cursor,
            elapsed,
        }));
    }
    finish(worker, statement, cursor, started, Some(elapsed)).await
}

/// Ends a statement and reports it. If rows were left unread, the server is asked to stop
/// producing them first, and the error that cancel causes is expected rather than reported.
async fn finish(
    worker: &mut Worker<'_>,
    statement: StatementRef,
    cursor: Box<dyn ResultCursor>,
    started: Instant,
    first_page: Option<Duration>,
) -> Result<StatementEnd, Disconnected> {
    let truncated = !cursor.is_exhausted();
    if truncated {
        cancel_running_statement(worker.cancel_handle).await;
    }
    let finished = worker
        .signals
        .guard(cursor.finish(), Some(statement.run), worker.cancel_handle)
        .await?;
    let elapsed = first_page.unwrap_or_else(|| started.elapsed());
    let rows_changed = match finished {
        Ok(rows_changed) => rows_changed,
        Err(_) if truncated => None,
        Err(error) => {
            report_failure(worker, &statement, elapsed, &error);
            return Ok(StatementEnd::Failed);
        }
    };
    report_success(
        worker.context,
        &statement,
        StatementSummary {
            rows_changed,
            elapsed,
            truncated,
        },
    );
    Ok(StatementEnd::Completed)
}

pub(super) async fn fetch_more(
    worker: &mut Worker<'_>,
    parked: Parked,
) -> Result<Option<Parked>, Disconnected> {
    let Parked {
        statement,
        mut cursor,
        elapsed,
    } = parked;
    let run = statement.run;
    let page = cursor.next_page(PAGE_SIZE);
    let rows = match worker
        .signals
        .guard(page, Some(run), worker.cancel_handle)
        .await?
    {
        Ok(rows) => rows,
        Err(error) => {
            let _ = cursor.finish().await;
            report_failure(worker, &statement, elapsed, &error);
            return Ok(None);
        }
    };
    let more = !cursor.is_exhausted();
    worker.context.send(Event::RowsFetched {
        run,
        statement: statement.index,
        rows,
        more,
    });
    if more {
        return Ok(Some(Parked {
            statement,
            cursor,
            elapsed,
        }));
    }
    let finished = worker
        .signals
        .guard(cursor.finish(), Some(run), worker.cancel_handle)
        .await?;
    match finished {
        Ok(rows_changed) => report_success(
            worker.context,
            &statement,
            StatementSummary {
                rows_changed,
                elapsed,
                truncated: false,
            },
        ),
        Err(error) => report_failure(worker, &statement, elapsed, &error),
    }
    Ok(None)
}

/// Drops a parked statement's unread rows, when the next request needs the connection or
/// the user cancels. Not interruptible: it is quick once the server has been told to stop.
pub(super) async fn discard(
    context: &SessionContext,
    parked: Parked,
    cancel_handle: &dyn CancelHandle,
) {
    cancel_running_statement(cancel_handle).await;
    // An error here is the cancellation just requested.
    let _ = parked.cursor.finish().await;
    report_success(
        context,
        &parked.statement,
        StatementSummary {
            rows_changed: None,
            elapsed: parked.elapsed,
            truncated: true,
        },
    );
}

fn report_success(context: &SessionContext, statement: &StatementRef, summary: StatementSummary) {
    tracing::info!(
        data_source = %context.name(),
        elapsed_ms = summary.elapsed.as_millis(),
        rows = summary.rows_changed,
        truncated = summary.truncated,
        "statement {} finished",
        statement.index + 1
    );
    context.record(history_entry(
        context,
        statement,
        summary.elapsed,
        summary.rows_changed,
        HistoryOutcome::Ok,
    ));
    context.send(Event::StatementFinished {
        run: statement.run,
        statement: statement.index,
        result: Ok(summary),
    });
}

fn report_failure(
    worker: &Worker<'_>,
    statement: &StatementRef,
    elapsed: Duration,
    error: &DriverError,
) {
    let context = worker.context;
    let (message, position) = match error {
        // The outer "the statement failed" adds nothing in a console that shows the result
        // of each statement; the database's own message is what matters.
        DriverError::Statement { source, position } => (describe_error(source.as_ref()), *position),
        other => (describe_error(other), None),
    };
    tracing::warn!(data_source = %context.name(), "statement {} failed: {message}", statement.index + 1);
    let outcome = if worker.signals.is_cancelled(statement.run) {
        HistoryOutcome::Cancelled
    } else {
        HistoryOutcome::Error
    };
    context.record(history_entry(context, statement, elapsed, None, outcome));
    context.send(Event::StatementFinished {
        run: statement.run,
        statement: statement.index,
        result: Err(StatementFailure { message, position }),
    });
}

fn history_entry(
    context: &SessionContext,
    statement: &StatementRef,
    elapsed: Duration,
    rows: Option<u64>,
    outcome: HistoryOutcome,
) -> HistoryEntry {
    HistoryEntry {
        data_source: context.data_source.id,
        executed_at: statement.started_at,
        sql: statement.sql.clone(),
        duration: Some(elapsed),
        rows,
        outcome,
    }
}

/// The statement's first line, shortened for the log.
fn preview(sql: &str) -> String {
    let first_line = sql.lines().next().unwrap_or_default();
    let shortened: String = first_line.chars().take(LOG_PREVIEW_CHARS).collect();
    if shortened.len() < sql.len() {
        format!("{shortened}…")
    } else {
        shortened
    }
}
