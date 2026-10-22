//! Running a console's statements: one page of rows per statement, and the last statement's
//! cursor kept open ("parked") so the UI can fetch more.
//!
//! Only the last statement stays open because a cursor holds the connection: earlier
//! statements' unread rows are discarded so the next statement can run. Their results are
//! marked truncated.

use std::time::{Duration, Instant};

use dbdesk_core::{CancelHandle, DriverError, ResultCursor, describe_error};

use super::{Disconnected, SessionContext, Worker, cancel_running_statement};
use crate::{Event, RunId, RunOutcome, StatementFailure, StatementSummary};

/// Rows per page: DataGrip's default. Plenty to look at, few enough to arrive at once.
const PAGE_SIZE: usize = 500;
/// How much of a statement the log console shows.
const LOG_PREVIEW_CHARS: usize = 80;

/// A statement with rows still unread, holding its connection until finished.
pub(super) struct Parked {
    pub(super) run: RunId,
    statement: usize,
    cursor: Box<dyn ResultCursor>,
    /// Time until the first page arrived, reported as the statement's duration.
    elapsed: Duration,
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
        match run_statement(worker, run, index, sql, index == last).await? {
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
    run: RunId,
    index: usize,
    sql: &str,
    is_last: bool,
) -> Result<StatementEnd, Disconnected> {
    let started = Instant::now();
    let execute = worker.connection.execute(sql);
    let mut cursor = match worker
        .signals
        .guard(execute, Some(run), worker.cancel_handle)
        .await?
    {
        Ok(cursor) => cursor,
        Err(error) => {
            report_failure(worker.context, run, index, &error);
            return Ok(StatementEnd::Failed);
        }
    };
    if cursor.columns().is_empty() {
        return finish(worker, run, index, cursor, started, None).await;
    }

    worker.context.send(Event::StatementStarted {
        run,
        statement: index,
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
            report_failure(worker.context, run, index, &error);
            return Ok(StatementEnd::Failed);
        }
    };
    let elapsed = started.elapsed();
    let more = !cursor.is_exhausted();
    worker.context.send(Event::RowsFetched {
        run,
        statement: index,
        rows,
        more: more && is_last,
    });
    if more && is_last {
        return Ok(StatementEnd::Parked(Parked {
            run,
            statement: index,
            cursor,
            elapsed,
        }));
    }
    finish(worker, run, index, cursor, started, Some(elapsed)).await
}

/// Ends a statement and reports it. If rows were left unread, the server is asked to stop
/// producing them first, and the error that cancel causes is expected rather than reported.
async fn finish(
    worker: &mut Worker<'_>,
    run: RunId,
    index: usize,
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
        .guard(cursor.finish(), Some(run), worker.cancel_handle)
        .await?;
    let elapsed = first_page.unwrap_or_else(|| started.elapsed());
    match finished {
        Ok(rows_changed) => {
            report_success(worker.context, run, index, rows_changed, elapsed, truncated)
        }
        Err(_) if truncated => report_success(worker.context, run, index, None, elapsed, true),
        Err(error) => {
            report_failure(worker.context, run, index, &error);
            return Ok(StatementEnd::Failed);
        }
    }
    Ok(StatementEnd::Completed)
}

pub(super) async fn fetch_more(
    worker: &mut Worker<'_>,
    parked: Parked,
) -> Result<Option<Parked>, Disconnected> {
    let Parked {
        run,
        statement,
        mut cursor,
        elapsed,
    } = parked;
    let page = cursor.next_page(PAGE_SIZE);
    let rows = match worker
        .signals
        .guard(page, Some(run), worker.cancel_handle)
        .await?
    {
        Ok(rows) => rows,
        Err(error) => {
            let _ = cursor.finish().await;
            report_failure(worker.context, run, statement, &error);
            return Ok(None);
        }
    };
    let more = !cursor.is_exhausted();
    worker.context.send(Event::RowsFetched {
        run,
        statement,
        rows,
        more,
    });
    if more {
        return Ok(Some(Parked {
            run,
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
        Ok(rows_changed) => {
            report_success(worker.context, run, statement, rows_changed, elapsed, false)
        }
        Err(error) => report_failure(worker.context, run, statement, &error),
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
        parked.run,
        parked.statement,
        None,
        parked.elapsed,
        true,
    );
}

fn report_success(
    context: &SessionContext,
    run: RunId,
    statement: usize,
    rows_changed: Option<u64>,
    elapsed: Duration,
    truncated: bool,
) {
    tracing::info!(
        data_source = %context.name(),
        elapsed_ms = elapsed.as_millis(),
        rows = rows_changed,
        truncated,
        "statement {} finished",
        statement + 1
    );
    context.send(Event::StatementFinished {
        run,
        statement,
        result: Ok(StatementSummary {
            rows_changed,
            elapsed,
            truncated,
        }),
    });
}

fn report_failure(context: &SessionContext, run: RunId, statement: usize, error: &DriverError) {
    let (message, position) = match error {
        // The outer "the statement failed" adds nothing in a console that shows the result
        // of each statement; the database's own message is what matters.
        DriverError::Statement { source, position } => (describe_error(source.as_ref()), *position),
        other => (describe_error(other), None),
    };
    tracing::warn!(data_source = %context.name(), "statement {} failed: {message}", statement + 1);
    context.send(Event::StatementFinished {
        run,
        statement,
        result: Err(StatementFailure { message, position }),
    });
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
