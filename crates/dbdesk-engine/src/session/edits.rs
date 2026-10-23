//! Applying the grid's edits: generated `UPDATE`s, all or none.

use dbdesk_core::{DriverError, TransactionStatus, describe_error};

use super::{Disconnected, SessionOwner, Worker, transaction};
use crate::{EditFailure, Event};

/// How the edits are wrapped: a transaction of their own, or a savepoint inside the console's
/// open one (PostgreSQL refuses `SAVEPOINT` outside a transaction, and a nested `BEGIN` would
/// fail).
struct Wrapping {
    open: &'static str,
    keep: &'static [&'static str],
    undo: &'static [&'static str],
}

const OWN_TRANSACTION: Wrapping = Wrapping {
    open: "BEGIN",
    keep: &["COMMIT"],
    undo: &["ROLLBACK"],
};

const SAVEPOINT: Wrapping = Wrapping {
    open: "SAVEPOINT dbdesk_edits",
    keep: &["RELEASE SAVEPOINT dbdesk_edits"],
    undo: &[
        "ROLLBACK TO SAVEPOINT dbdesk_edits",
        "RELEASE SAVEPOINT dbdesk_edits",
    ],
};

/// Runs `statements`, each of which must change exactly one row, and reports
/// [`Event::EditsApplied`]. On any failure, everything they changed is undone.
pub(super) async fn apply(
    worker: &mut Worker<'_>,
    statements: &[String],
) -> Result<(), Disconnected> {
    let SessionOwner::Console(console) = worker.context.owner else {
        return Ok(());
    };
    let wrapping = match worker.transaction {
        TransactionStatus::Idle => &OWN_TRANSACTION,
        TransactionStatus::InTransaction | TransactionStatus::Failed => &SAVEPOINT,
    };
    let result = match run_wrapped(worker, wrapping, statements).await? {
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
    match &result {
        Ok(()) => tracing::info!(
            data_source = %worker.context.name(),
            "applied {} row edits",
            statements.len()
        ),
        Err(failure) => tracing::warn!(
            data_source = %worker.context.name(),
            "edits rolled back: {}",
            failure.message
        ),
    }
    worker.context.send(Event::EditsApplied { console, result });
    Ok(())
}

async fn run_wrapped(
    worker: &mut Worker<'_>,
    wrapping: &Wrapping,
    statements: &[String],
) -> Result<Result<(), EditFailure>, Disconnected> {
    let unwrapped = |error: DriverError| EditFailure {
        statement: None,
        message: describe_error(&error),
    };
    if let Err(error) = transaction::run_counting(worker, wrapping.open, None).await? {
        // Nothing was started, so there is nothing to undo; report it as it is.
        return Ok(Err(unwrapped(error)));
    }
    for (index, sql) in statements.iter().enumerate() {
        tracing::info!(data_source = %worker.context.name(), "{sql}");
        let failure = match transaction::run_counting(worker, sql, None).await? {
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
