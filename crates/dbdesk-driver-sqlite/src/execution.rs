//! Running user statements. A SQLite statement borrows its connection and every step blocks,
//! so each statement runs on its own thread from tokio's blocking pool, which steps rows only
//! when the cursor asks for a page. The thread holds the connection's lock until the cursor
//! is finished, which is what keeps the connection busy meanwhile.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use dbdesk_core::{
    CancelHandle, Cell, ColumnInfo, DriverError, Rejection, RejectionKind, ResultCursor, Row,
};
use rusqlite::types::ValueRef;
use rusqlite::{InterruptHandle, Statement};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::{connection_error, poisoned};

/// Blobs longer than this are shown as their size, not their bytes: a result grid is no place
/// for a megabyte of hex.
const MAX_BLOB_HEX_BYTES: usize = 64;

/// What the statement thread reports once the statement is prepared (and, for statements
/// without rows, run).
struct Started {
    columns: Vec<ColumnInfo>,
    rows_changed: Option<u64>,
}

struct Page {
    rows: Vec<Row>,
    exhausted: bool,
}

pub(crate) async fn execute(
    connection: &Arc<Mutex<rusqlite::Connection>>,
    cancel: &SqliteCancel,
    sql: &str,
) -> Result<Box<dyn ResultCursor>, DriverError> {
    // A cancel applies to the statement running when it arrives; a new statement starts
    // uncancelled. This runs before the statement thread exists, so a cancel can't slip in
    // between the reset and the statement's start.
    cancel.requested.store(false, Ordering::SeqCst);
    let cancelled = Arc::clone(&cancel.requested);
    let (started_sender, started) = oneshot::channel();
    // Capacity 1: the thread steps at most one page ahead of what was asked for.
    let (demand, demand_receiver) = mpsc::channel(1);
    let (page_sender, pages) = mpsc::channel(1);
    let connection = Arc::clone(connection);
    let sql = sql.to_owned();
    let thread = tokio::task::spawn_blocking(move || {
        let channels = Channels {
            started: started_sender,
            demand: demand_receiver,
            pages: page_sender,
        };
        run_statement(&connection, &sql, &cancelled, channels);
    });

    let started = started
        .await
        .map_err(|_| connection_error("the statement thread stopped unexpectedly"))??;
    let exhausted = started.columns.is_empty();
    Ok(Box::new(SqliteCursor {
        columns: started.columns,
        rows_changed: started.rows_changed,
        exhausted,
        demand: Some(demand),
        pages,
        thread,
    }))
}

/// How the statement thread talks to its cursor.
struct Channels {
    started: oneshot::Sender<Result<Started, DriverError>>,
    demand: mpsc::Receiver<usize>,
    pages: mpsc::Sender<Result<Page, DriverError>>,
}

/// The statement thread: prepares, then steps one page per demand until the result is
/// exhausted, the statement is cancelled, or the cursor goes away.
fn run_statement(
    connection: &Mutex<rusqlite::Connection>,
    sql: &str,
    cancelled: &AtomicBool,
    channels: Channels,
) {
    let Channels {
        started,
        mut demand,
        pages,
    } = channels;
    let Ok(connection) = connection.lock() else {
        let _ = started.send(Err(poisoned()));
        return;
    };
    let mut statement = match connection.prepare(sql) {
        Ok(statement) => statement,
        Err(error) => {
            // The receiver only disappears if the cursor's caller gave up; nothing to do.
            let _ = started.send(Err(statement_error(error)));
            return;
        }
    };
    let columns = column_info(&statement);
    if columns.is_empty() {
        let changes_before = connection.total_changes();
        let result = statement
            .raw_execute()
            .map(|_| Started {
                columns,
                rows_changed: Some(connection.total_changes() - changes_before),
            })
            .map_err(statement_error);
        let _ = started.send(result);
        return;
    }
    if started
        .send(Ok(Started {
            columns,
            rows_changed: None,
        }))
        .is_err()
    {
        return;
    }
    step_pages(&mut statement, cancelled, &mut demand, &pages);
}

fn step_pages(
    statement: &mut Statement<'_>,
    cancelled: &AtomicBool,
    demand: &mut mpsc::Receiver<usize>,
    pages: &mpsc::Sender<Result<Page, DriverError>>,
) {
    let column_count = statement.column_count();
    let mut rows = statement.raw_query();
    while let Some(max_rows) = demand.blocking_recv() {
        let mut page = Vec::new();
        let mut exhausted = false;
        while page.len() < max_rows {
            // `sqlite3_interrupt` only stops a step already in progress; this catches a
            // cancel that arrived between steps, or before the first one.
            if cancelled.load(Ordering::SeqCst) {
                let _ = pages.blocking_send(Err(interrupted()));
                return;
            }
            match rows.next() {
                Ok(Some(row)) => {
                    page.push((0..column_count).map(|i| cell(row.get_ref(i))).collect())
                }
                Ok(None) => {
                    exhausted = true;
                    break;
                }
                Err(error) => {
                    let _ = pages.blocking_send(Err(statement_error(error)));
                    return;
                }
            }
        }
        if pages
            .blocking_send(Ok(Page {
                rows: page,
                exhausted,
            }))
            .is_err()
            || exhausted
        {
            return;
        }
    }
}

fn column_info(statement: &Statement<'_>) -> Vec<ColumnInfo> {
    statement
        .columns()
        .iter()
        .map(|column| ColumnInfo {
            name: column.name().to_owned(),
            type_name: column.decl_type().map(str::to_lowercase),
        })
        .collect()
}

fn cell(value: rusqlite::Result<ValueRef<'_>>) -> Cell {
    match value {
        Ok(ValueRef::Null) => None,
        Ok(ValueRef::Integer(integer)) => Some(integer.to_string()),
        // `{:?}` keeps the decimal point on whole numbers (`100.0`), as the sqlite3 shell does.
        Ok(ValueRef::Real(real)) => Some(format!("{real:?}")),
        Ok(ValueRef::Text(text)) => Some(String::from_utf8_lossy(text).into_owned()),
        Ok(ValueRef::Blob(blob)) if blob.len() > MAX_BLOB_HEX_BYTES => {
            Some(format!("<blob, {} bytes>", blob.len()))
        }
        Ok(ValueRef::Blob(blob)) => Some(format!(
            "x'{}'",
            blob.iter().map(|b| format!("{b:02x}")).collect::<String>()
        )),
        Err(error) => Some(format!("<unreadable: {error}>")),
    }
}

/// Compiles `sql` without stepping it, so nothing runs. Errors SQLite finds in the text (the
/// generic `SQLITE_ERROR`) are rejections; anything else, such as a locked database, means it
/// couldn't be asked.
pub(crate) fn validate(
    connection: &rusqlite::Connection,
    sql: &str,
) -> Result<Option<Rejection>, DriverError> {
    let error = match connection.prepare(sql) {
        Ok(_) => return Ok(None),
        Err(error) => error,
    };
    let (message, position) = match &error {
        rusqlite::Error::SqlInputError { msg, offset, .. } => {
            (msg.clone(), usize::try_from(*offset).ok())
        }
        rusqlite::Error::SqliteFailure(failure, Some(msg))
            if failure.code == rusqlite::ErrorCode::Unknown =>
        {
            (msg.clone(), None)
        }
        _ => return Err(DriverError::Connection(Box::new(error))),
    };
    let kind = if message.contains("syntax error") || message.contains("incomplete input") {
        RejectionKind::Syntax
    } else if message.starts_with("no such ") {
        RejectionKind::UnknownName
    } else {
        RejectionKind::Other
    };
    Ok(Some(Rejection {
        message,
        position,
        kind,
    }))
}

/// Maps a SQLite error to [`DriverError::Statement`], keeping the byte offset SQLite reports
/// for errors in the statement text (unknown tables and columns, syntax errors).
fn statement_error(error: rusqlite::Error) -> DriverError {
    let position = match &error {
        rusqlite::Error::SqlInputError { offset, .. } => usize::try_from(*offset).ok(),
        _ => None,
    };
    // SQLite has result codes but no SQLSTATEs.
    DriverError::Statement {
        source: Box::new(error),
        position,
        sqlstate: None,
    }
}

struct SqliteCursor {
    columns: Vec<ColumnInfo>,
    rows_changed: Option<u64>,
    exhausted: bool,
    /// Dropping it tells the statement thread to stop and release the connection.
    demand: Option<mpsc::Sender<usize>>,
    pages: mpsc::Receiver<Result<Page, DriverError>>,
    thread: JoinHandle<()>,
}

#[async_trait]
impl ResultCursor for SqliteCursor {
    fn columns(&self) -> &[ColumnInfo] {
        &self.columns
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>, DriverError> {
        let Some(demand) = self.demand.as_ref().filter(|_| !self.exhausted) else {
            return Ok(Vec::new());
        };
        if demand.send(max_rows).await.is_err() {
            self.exhausted = true;
            return Ok(Vec::new());
        }
        match self.pages.recv().await {
            Some(Ok(page)) => {
                self.exhausted = page.exhausted;
                Ok(page.rows)
            }
            Some(Err(error)) => {
                self.exhausted = true;
                Err(error)
            }
            None => {
                self.exhausted = true;
                Ok(Vec::new())
            }
        }
    }

    fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    /// Stops the statement thread and waits for it, so the connection is free again.
    async fn finish(mut self: Box<Self>) -> Result<Option<u64>, DriverError> {
        self.demand = None;
        self.pages.close();
        (&mut self.thread).await.map_err(connection_error)?;
        Ok(self.rows_changed)
    }
}

fn interrupted() -> DriverError {
    statement_error(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_INTERRUPT),
        Some("interrupted".to_owned()),
    ))
}

/// Stops the running statement: `sqlite3_interrupt` for a step in progress, plus a flag the
/// statement thread checks before every step, for a cancel that lands between steps.
pub(crate) struct SqliteCancel {
    interrupt: InterruptHandle,
    requested: Arc<AtomicBool>,
}

impl SqliteCancel {
    pub(crate) fn new(interrupt: InterruptHandle) -> Self {
        Self {
            interrupt,
            requested: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait]
impl CancelHandle for SqliteCancel {
    async fn cancel(&self) -> Result<(), DriverError> {
        self.requested.store(true, Ordering::SeqCst);
        self.interrupt.interrupt();
        Ok(())
    }
}
