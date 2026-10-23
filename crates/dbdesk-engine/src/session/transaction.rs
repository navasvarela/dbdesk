//! A console's transaction: opened lazily in manual mode, tracked after the statements that
//! can change it, and ended on request.

use dbdesk_core::{DriverError, TransactionStatus, describe_error};
use dbdesk_sql::controls_transactions;

use super::{Disconnected, SessionOwner, Worker};
use crate::{Event, RunId, TransactionEnd, TransactionMode};

/// Opens a transaction before `sql` when manual mode needs one: none is open yet, and the
/// statement isn't itself transaction control (a user's own `BEGIN` would fail inside one,
/// and a `COMMIT` has nothing to commit).
pub(super) async fn begin_if_needed(
    worker: &mut Worker<'_>,
    mode: TransactionMode,
    run: RunId,
    sql: &str,
) -> Result<Result<(), DriverError>, Disconnected> {
    let needed = mode == TransactionMode::Manual
        && worker.transaction == TransactionStatus::Idle
        && !controls_transactions(sql, worker.context.dialect());
    if !needed {
        return Ok(Ok(()));
    }
    let begun = run_command(worker, "BEGIN", Some(run)).await?;
    if begun.is_ok() {
        set(worker, TransactionStatus::InTransaction);
    }
    Ok(begun)
}

/// Asks the database for the transaction status after a statement that may have changed it,
/// and tells the UI if it did. A failed check is logged and changes nothing.
pub(super) async fn refresh(worker: &mut Worker<'_>) -> Result<(), Disconnected> {
    let check = worker.connection.transaction_status();
    match worker
        .signals
        .guard(check, None, worker.cancel_handle)
        .await?
    {
        Ok(status) => set(worker, status),
        Err(error) => tracing::warn!(
            data_source = %worker.context.name(),
            "could not check the transaction status: {}",
            describe_error(&error)
        ),
    }
    Ok(())
}

/// Commits or rolls back, then reports what happened. Committing a failed transaction
/// rolls it back, which is reported as such.
pub(super) async fn end(worker: &mut Worker<'_>, end: TransactionEnd) -> Result<(), Disconnected> {
    let SessionOwner::Console(console) = worker.context.owner else {
        return Ok(());
    };
    let happened = match (end, worker.transaction) {
        (TransactionEnd::Commit, TransactionStatus::Failed) => TransactionEnd::Rollback,
        _ => end,
    };
    let sql = match end {
        TransactionEnd::Commit => "COMMIT",
        TransactionEnd::Rollback => "ROLLBACK",
    };
    let result = run_command(worker, sql, None).await?;
    refresh(worker).await?;
    let result = match result {
        Ok(()) => {
            tracing::info!(data_source = %worker.context.name(), "{}", match happened {
                TransactionEnd::Commit => "committed",
                TransactionEnd::Rollback => "rolled back",
            });
            Ok(happened)
        }
        Err(error) => {
            let error = describe_error(&error);
            tracing::warn!(data_source = %worker.context.name(), "{sql} failed: {error}");
            Err(error)
        }
    };
    worker
        .context
        .send(Event::TransactionEnded { console, result });
    Ok(())
}

fn set(worker: &mut Worker<'_>, status: TransactionStatus) {
    if worker.transaction != status {
        worker.transaction = status;
        worker.context.notify_transaction(status);
    }
}

/// Runs a statement that returns no rows, such as `BEGIN` or `COMMIT`, to completion.
async fn run_command(
    worker: &mut Worker<'_>,
    sql: &str,
    run: Option<RunId>,
) -> Result<Result<(), DriverError>, Disconnected> {
    let connection = &mut *worker.connection;
    let work = async move {
        let cursor = connection.execute(sql).await?;
        cursor.finish().await.map(drop)
    };
    worker.signals.guard(work, run, worker.cancel_handle).await
}
