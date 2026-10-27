//! Importing a CSV file into a table on the explorer's connection: batches of rows, one
//! multi-row `INSERT` each, all in one transaction that the first failure rolls back.
//!
//! A batch that fails is retried a row at a time behind a savepoint, only to find the line the
//! database refused; everything is rolled back either way.

use std::fs::File;
use std::io::BufReader;
use std::time::Instant;

use dbdesk_core::{Cell, DriverError, describe_error};
use dbdesk_sql::{RowSql, Savepoint};
use dbdesk_store::{CsvError, CsvReader, CsvRecord, HistoryEntry, HistoryOutcome, HistorySource};
use jiff::Timestamp;

use super::{Disconnected, Worker, transaction};
use crate::{Event, ImportFailure, RowsImport};

/// Rows per `INSERT`. Enough that a round trip carries many rows; well under SQL Server's
/// limit of 1,000 rows in one `VALUES` list.
const BATCH_ROWS: usize = 500;

type Reader = CsvReader<BufReader<File>>;

/// Imports the file, records it in the history, and reports [`Event::RowsImported`].
pub(super) async fn import(
    worker: &mut Worker<'_>,
    import: &RowsImport,
) -> Result<(), Disconnected> {
    let context = worker.context;
    let refusal = if context.read_only.is_some() {
        Some(format!(
            "Refused: \u{201c}{}\u{201d} is read-only, so nothing can be imported into it.",
            context.name()
        ))
    } else {
        context.dialect().import_refusal().map(str::to_owned)
    };
    let result = match refusal {
        Some(message) => Err(ImportFailure {
            lines: None,
            message,
        }),
        None => {
            let started_at = Timestamp::now();
            let started = Instant::now();
            let result = in_transaction(worker, import).await?;
            record(worker, import, &result, (started_at, started));
            result
        }
    };
    match &result {
        Ok(rows) => tracing::info!(
            data_source = %context.name(),
            "imported {rows} rows from {}",
            import.file.display()
        ),
        Err(failure) => tracing::warn!(
            data_source = %context.name(),
            "import rolled back: {}",
            failure.message
        ),
    }
    context.send(Event::RowsImported {
        data_source: context.data_source.id,
        result,
    });
    Ok(())
}

/// Records the whole import as one entry: the `INSERT`'s table and columns without the rows,
/// which are the file's, how many went in, and the file.
fn record(
    worker: &Worker<'_>,
    import: &RowsImport,
    result: &Result<u64, ImportFailure>,
    (started_at, started): (Timestamp, Instant),
) {
    let columns: Vec<&str> = import
        .mapping
        .iter()
        .map(|(column, _)| column.as_str())
        .collect();
    let insert = RowSql::new(worker.context.dialect(), &import.table).insert_rows(&columns, &[]);
    let (rows, outcome) = match result {
        Ok(rows) => (Some(*rows), HistoryOutcome::Ok),
        Err(_) => (None, HistoryOutcome::Error),
    };
    worker.context.record(HistoryEntry {
        data_source: worker.context.data_source.id,
        executed_at: started_at,
        sql: format!("{insert}\u{2026}"),
        duration: Some(started.elapsed()),
        rows,
        outcome,
        source: HistorySource::Import {
            file: import.file.clone(),
        },
    });
}

/// Opens the file and a transaction, inserts every row, and commits, or rolls back at the
/// first failure.
async fn in_transaction(
    worker: &mut Worker<'_>,
    import: &RowsImport,
) -> Result<Result<u64, ImportFailure>, Disconnected> {
    let unblamed = |message: String| ImportFailure {
        lines: None,
        message,
    };
    let (file, separator) = (import.file.clone(), import.separator);
    let reader = match blocking(move || CsvReader::open(&file, separator)).await {
        Ok(Ok(reader)) => reader,
        Ok(Err(error)) => return Ok(Err(csv_failure(&error))),
        Err(failure) => return Ok(Err(failure)),
    };
    let begin = worker.context.dialect().begin_transaction();
    if let Err(error) = transaction::run_counting(worker, begin, None).await? {
        return Ok(Err(unblamed(describe_error(&error))));
    }
    let failure = match insert_all(worker, import, reader).await? {
        Ok(rows) => match transaction::run_counting(worker, "COMMIT", None).await? {
            Ok(_) => return Ok(Ok(rows)),
            Err(error) => unblamed(describe_error(&error)),
        },
        Err(failure) => failure,
    };
    if let Err(error) = transaction::run_counting(worker, "ROLLBACK", None).await? {
        tracing::warn!(
            data_source = %worker.context.name(),
            "ROLLBACK failed after an import failed: {}",
            describe_error(&error)
        );
    }
    Ok(Err(failure))
}

/// Reads the file a batch at a time and inserts each batch. Returns how many rows went in.
async fn insert_all(
    worker: &mut Worker<'_>,
    import: &RowsImport,
    mut reader: Reader,
) -> Result<Result<u64, ImportFailure>, Disconnected> {
    if import.header {
        // On its own, so that every batch is a whole one.
        match read_records(reader, 1).await {
            Ok((returned, _)) => reader = returned,
            Err(failure) => return Ok(Err(failure)),
        }
    }
    let mut imported = 0;
    loop {
        let records = match read_records(reader, BATCH_ROWS).await {
            Ok((_, records)) if records.is_empty() => return Ok(Ok(imported)),
            Ok((returned, records)) => {
                reader = returned;
                records
            }
            Err(failure) => return Ok(Err(failure)),
        };
        let rows = match rows_of(&records, &import.mapping) {
            Ok(rows) => rows,
            Err(failure) => return Ok(Err(failure)),
        };
        let batch = Batch {
            import,
            records: &records,
            rows: &rows,
        };
        if let Err(failure) = insert_batch(worker, &batch).await? {
            return Ok(Err(failure));
        }
        imported += rows.len() as u64;
        worker.context.send(Event::ImportProgress {
            data_source: worker.context.data_source.id,
            rows: imported,
        });
    }
}

/// Up to `count` records, read on the blocking pool; the reader comes back with them.
async fn read_records(
    mut reader: Reader,
    count: usize,
) -> Result<(Reader, Vec<CsvRecord>), ImportFailure> {
    let (reader, records) = blocking(move || {
        let records = reader.next_records(count);
        (reader, records)
    })
    .await?;
    let records = records.map_err(|error| csv_failure(&error))?;
    Ok((reader, records))
}

/// Each record's values for the mapped columns, in mapping order: an empty field is `NULL`.
fn rows_of(
    records: &[CsvRecord],
    mapping: &[(String, usize)],
) -> Result<Vec<Vec<Cell>>, ImportFailure> {
    records
        .iter()
        .map(|record| {
            mapping
                .iter()
                .map(|(column, field)| match record.fields.get(*field) {
                    Some(value) if value.is_empty() => Ok(None),
                    Some(value) => Ok(Some(value.clone())),
                    None => Err(ImportFailure {
                        lines: Some((record.line, record.line)),
                        message: format!(
                            "this line has {} fields, and {column} comes from field {}",
                            record.fields.len(),
                            field + 1
                        ),
                    }),
                })
                .collect()
        })
        .collect()
}

/// A batch of the file's records and the rows they make.
struct Batch<'a> {
    import: &'a RowsImport,
    records: &'a [CsvRecord],
    rows: &'a [Vec<Cell>],
}

impl Batch<'_> {
    fn insert(&self, worker: &Worker<'_>, rows: &[Vec<Cell>]) -> String {
        let columns: Vec<&str> = self
            .import
            .mapping
            .iter()
            .map(|(column, _)| column.as_str())
            .collect();
        RowSql::new(worker.context.dialect(), &self.import.table).insert_rows(&columns, rows)
    }

    /// The batch's first and last lines.
    fn lines(&self) -> Option<(usize, usize)> {
        Some((self.records.first()?.line, self.records.last()?.line))
    }
}

/// Inserts a batch; on failure, finds the line the database refused, where savepoints allow.
async fn insert_batch(
    worker: &mut Worker<'_>,
    batch: &Batch<'_>,
) -> Result<Result<(), ImportFailure>, Disconnected> {
    let savepoint = worker.context.dialect().edits_savepoint();
    if let Some(savepoint) = &savepoint
        && let Err(error) = transaction::run_counting(worker, savepoint.open, None).await?
    {
        return Ok(Err(ImportFailure {
            lines: None,
            message: describe_error(&error),
        }));
    }
    let sql = batch.insert(worker, batch.rows);
    let error = match transaction::run_counting(worker, &sql, None).await? {
        Ok(_) => {
            for keep in savepoint.iter().flat_map(|savepoint| savepoint.keep) {
                if let Err(error) = transaction::run_counting(worker, keep, None).await? {
                    tracing::warn!(
                        "{keep} failed after a batch was inserted: {}",
                        describe_error(&error)
                    );
                }
            }
            return Ok(Ok(()));
        }
        Err(error) => error,
    };
    let refused = ImportFailure {
        lines: batch.lines(),
        message: statement_error(&error),
    };
    match savepoint {
        Some(savepoint) => Ok(Err(find_refused_line(worker, batch, &savepoint)
            .await?
            .unwrap_or(refused))),
        None => Ok(Err(refused)),
    }
}

/// Undoes the batch back to its savepoint and inserts its rows one at a time, to find the
/// first the database refuses. `None` if none is refused alone.
async fn find_refused_line(
    worker: &mut Worker<'_>,
    batch: &Batch<'_>,
    savepoint: &Savepoint,
) -> Result<Option<ImportFailure>, Disconnected> {
    for undo in savepoint.undo {
        if let Err(error) = transaction::run_counting(worker, undo, None).await? {
            tracing::warn!(
                "{undo} failed after a batch failed: {}",
                describe_error(&error)
            );
            return Ok(None);
        }
    }
    for (record, row) in batch.records.iter().zip(batch.rows) {
        let sql = batch.insert(worker, std::slice::from_ref(row));
        if let Err(error) = transaction::run_counting(worker, &sql, None).await? {
            return Ok(Some(ImportFailure {
                lines: Some((record.line, record.line)),
                message: statement_error(&error),
            }));
        }
    }
    Ok(None)
}

/// The database's own message, without the outer "the statement failed".
fn statement_error(error: &DriverError) -> String {
    match error {
        DriverError::Statement { source, .. } => describe_error(source.as_ref()),
        other => describe_error(other),
    }
}

fn csv_failure(error: &CsvError) -> ImportFailure {
    let line = match error {
        CsvError::NotText { line } | CsvError::UnterminatedQuote { line } => Some((*line, *line)),
        CsvError::Read { .. } => None,
    };
    ImportFailure {
        lines: line,
        message: describe_error(error),
    }
}

/// Runs file work on the blocking pool, so a slow disk never holds up the session's task.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ImportFailure> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| ImportFailure {
            lines: None,
            message: describe_error(&error),
        })
}
