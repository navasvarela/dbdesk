//! Applying the grid's changes: generated `DELETE`s, `UPDATE`s and `INSERT`s, all or none; or,
//! in a language without transactions (Redis), one command at a time.

use std::time::Instant;

use dbrover_core::{DriverError, TransactionStatus, describe_error};
use dbrover_language::EditsRun;
use dbrover_sql::{Dialect, Savepoint};
use dbrover_store::{HistoryEntry, HistoryOutcome, HistorySource, redact_secrets};
use jiff::Timestamp;

use super::{Disconnected, SessionOwner, Worker, transaction};
use crate::{EditFailure, Event, RowEdit};

/// How the edits are wrapped: a transaction of their own, or a savepoint inside the console's
/// open one (PostgreSQL refuses `SAVEPOINT` outside a transaction, and a nested `BEGIN` would
/// fail). Both are spelt as the dialect writes them. `None` inside an open transaction on a
/// database without savepoints (DuckDB): a failed edit couldn't be undone without undoing the
/// whole transaction.
fn wrapping(worker: &Worker<'_>, dialect: Dialect) -> Option<Savepoint> {
    match worker.transaction {
        TransactionStatus::Idle => Some(Savepoint {
            open: dialect.begin_transaction(),
            keep: &["COMMIT"],
            undo: &["ROLLBACK"],
        }),
        TransactionStatus::InTransaction | TransactionStatus::Failed => dialect.edits_savepoint(),
    }
}

/// Runs `statements`, each of which must change exactly one row, and reports
/// [`Event::EditsApplied`]. On any failure, everything they changed is undone. Each statement
/// sent is recorded in the history, with its row's change, as it runs.
pub(super) async fn apply(
    worker: &mut Worker<'_>,
    statements: &[RowEdit],
) -> Result<(), Disconnected> {
    let SessionOwner::Console(console) = worker.context.owner else {
        return Ok(());
    };
    if worker.context.read_only.is_some() {
        let message = format!(
            "\u{201c}{}\u{201d} is read-only, so its rows can't be edited.",
            worker.context.name()
        );
        tracing::warn!(data_source = %worker.context.name(), "edits refused: read-only");
        worker.context.send(Event::EditsApplied {
            console,
            result: Err(EditFailure {
                statement: None,
                message,
            }),
        });
        return Ok(());
    }
    let dialect = match worker.context.language().edits_run() {
        Some(EditsRun::Transaction(dialect)) => dialect,
        Some(EditsRun::OneByOne) => {
            let result = run_one_by_one(worker, statements).await?;
            report(worker, console, statements.len(), result);
            return Ok(());
        }
        None => {
            worker.context.send(Event::EditsApplied {
                console,
                result: Err(EditFailure {
                    statement: None,
                    message: worker
                        .context
                        .needs_sql("its rows can't be edited from the grid"),
                }),
            });
            return Ok(());
        }
    };
    let Some(wrapping) = wrapping(worker, dialect) else {
        let failure = EditFailure {
            statement: None,
            message: "This database has no savepoints, so edits can't be applied inside an open \
                      transaction and undone on their own: commit or roll back first."
                .to_owned(),
        };
        worker.context.send(Event::EditsApplied {
            console,
            result: Err(failure),
        });
        return Ok(());
    };
    let result = match run_wrapped(worker, &wrapping, statements).await? {
        Ok(()) => Ok(()),
        Err(failure) => {
            for undo in wrapping.undo {
                if let Err(error) = transaction::run_counting(worker, undo, None).await? {
                    tracing::warn!(
                        "{undo} failed after an edit failed: {}",
                        describe_error(&error)
                    );
                }
            }
            Err(failure)
        }
    };
    transaction::refresh(worker).await?;
    report(worker, console, statements.len(), result);
    Ok(())
}

fn report(
    worker: &Worker<'_>,
    console: crate::ConsoleId,
    count: usize,
    result: Result<(), EditFailure>,
) {
    match &result {
        Ok(()) => tracing::info!(
            data_source = %worker.context.name(),
            "applied {count} row edits"
        ),
        Err(failure) => tracing::warn!(
            data_source = %worker.context.name(),
            "edits stopped: {}",
            failure.message
        ),
    }
    worker.context.send(Event::EditsApplied { console, result });
}

/// Runs each of `statements`, a command each, until one fails. Those before it took effect and
/// stay: there is no transaction to undo them, and the failure says so. A command's own answer
/// isn't a count of rows (`HSET` answers how many fields it added), so any answer that isn't an
/// error is success.
async fn run_one_by_one(
    worker: &mut Worker<'_>,
    statements: &[RowEdit],
) -> Result<Result<(), EditFailure>, Disconnected> {
    for (index, edit) in statements.iter().enumerate() {
        tracing::info!(data_source = %worker.context.name(), "{}", redact_secrets(&edit.sql));
        let started_at = Timestamp::now();
        let started = Instant::now();
        let result = transaction::run_counting(worker, &edit.sql, None).await?;
        record(worker, edit, &result, (started_at, started));
        if let Err(error) = result {
            let reason = match &error {
                DriverError::Statement { source, .. } => describe_error(source.as_ref()),
                other => describe_error(other),
            };
            let message = match index {
                0 => reason,
                1 => format!("{reason}; the change before it was made"),
                applied => format!("{reason}; the {applied} changes before it were made"),
            };
            return Ok(Err(EditFailure {
                statement: Some(index),
                message,
            }));
        }
    }
    Ok(Ok(()))
}

async fn run_wrapped(
    worker: &mut Worker<'_>,
    wrapping: &Savepoint,
    statements: &[RowEdit],
) -> Result<Result<(), EditFailure>, Disconnected> {
    let unwrapped = |error: DriverError| EditFailure {
        statement: None,
        message: describe_error(&error),
    };
    if let Err(error) = transaction::run_counting(worker, wrapping.open, None).await? {
        // Nothing was started, so there is nothing to undo; report it as it is.
        return Ok(Err(unwrapped(error)));
    }
    for (index, edit) in statements.iter().enumerate() {
        tracing::info!(data_source = %worker.context.name(), "{}", redact_secrets(&edit.sql));
        let started_at = Timestamp::now();
        let started = Instant::now();
        let result = transaction::run_counting(worker, &edit.sql, None).await?;
        record(worker, edit, &result, (started_at, started));
        let failure = match result {
            Ok(Some(1)) => continue,
            Ok(changed) => format!(
                "changed {} rows instead of one; the row may have changed or gone since it was \
                 read",
                changed.unwrap_or(0)
            ),
            Err(DriverError::Statement { source, .. }) => describe_error(source.as_ref()),
            Err(other) => describe_error(&other),
        };
        return Ok(Err(EditFailure {
            statement: Some(index),
            message: failure,
        }));
    }
    for keep in wrapping.keep {
        if let Err(error) = transaction::run_counting(worker, keep, None).await? {
            return Ok(Err(unwrapped(error)));
        }
    }
    Ok(Ok(()))
}

/// Records a statement the grid sent, as it ran: one that later edits undid still reached the
/// database, which is what the history holds.
fn record(
    worker: &Worker<'_>,
    edit: &RowEdit,
    result: &Result<Option<u64>, DriverError>,
    (started_at, started): (Timestamp, Instant),
) {
    let (rows, outcome) = match result {
        Ok(rows) => (*rows, HistoryOutcome::Ok),
        Err(_) => (None, HistoryOutcome::Error),
    };
    worker.context.record(HistoryEntry {
        data_source: worker.context.data_source.id,
        executed_at: started_at,
        sql: edit.sql.clone(),
        duration: Some(started.elapsed()),
        rows,
        outcome,
        source: HistorySource::GridEdit(edit.change.clone()),
    });
}
