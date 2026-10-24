//! Running user statements, and cancelling them.
//!
//! `tiberius` borrows the client for as long as a result is open, so each statement runs on
//! its own task that owns the client's lock and reads rows only when the cursor asks for a
//! page, as the MySQL driver's statement task does. The task keeps the lock until the cursor
//! is finished, which is what keeps the connection busy meanwhile.
//!
//! Cancelling uses TDS's own attention message, which any login may send, rather than `KILL`
//! from a second connection, which needs a permission most logins lack. Sending it needs the
//! client itself, which the running statement's result stream borrows, so a cancel is a
//! signal to the statement task: the task stops reading, drops the stream, and sends the
//! attention (`Client::cancel_query`), which also reads the server's acknowledgement and
//! leaves the connection ready for the next statement.

use std::future::Future;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use dbdesk_core::{CancelHandle, ColumnInfo, DriverError, Rejection, ResultCursor, Row};
use futures_util::TryStreamExt;
use tiberius::{QueryItem, QueryStream};
use tokio::sync::{Mutex, OwnedMutexGuard, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::errors::{rejection, statement_error};
use crate::{SQL_SERVER_QUOTING, SqlClient, connection_error, values};

/// How long a read already under way may go on after a cancel before the stream is dropped.
///
/// Dropping the result stream while it is halfway through a row (waiting for the packet with
/// the rest of it) would lose the half already read, and the attention's acknowledgement
/// could then not be found in what follows. Rows that are arriving complete in far less than
/// this, so a cancel during a fast result stops between two rows; a read still waiting after
/// it is almost always waiting for a server that is busy computing, between tokens. 100 ms
/// was chosen as long against a network round trip and short against a person waiting.
const CANCEL_GRACE: Duration = Duration::from_millis(100);

/// Where a cancel reaches the running statement: the sender of a fresh channel per
/// statement, so a cancel never outlives the statement it was meant for.
#[derive(Default)]
pub(crate) struct CancelSlot {
    running: std::sync::Mutex<Option<watch::Sender<bool>>>,
}

impl CancelSlot {
    fn arm(&self) -> Cancellation {
        let (sender, receiver) = watch::channel(false);
        *self.running.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
        Cancellation(receiver)
    }

    fn disarm(&self) {
        *self.running.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

#[async_trait]
impl CancelHandle for CancelSlot {
    /// Signals the statement task, which sends the attention itself: returns at once, and
    /// does nothing when no statement is running.
    async fn cancel(&self) -> Result<(), DriverError> {
        let running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sender) = running.as_ref() {
            sender.send_replace(true);
        }
        Ok(())
    }
}

/// The statement task's end of a [`CancelSlot`].
struct Cancellation(watch::Receiver<bool>);

impl Cancellation {
    /// Completes once a cancel is requested; never if the slot is disarmed first.
    async fn requested(&mut self) {
        if self.0.wait_for(|cancelled| *cancelled).await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Why a statement stopped early, when it was cancelled.
#[derive(Debug, thiserror::Error)]
enum CancelError {
    #[error("the statement was cancelled")]
    Cancelled,
    #[error("the statement was cancelled, but the server's acknowledgement was lost")]
    Unacknowledged(#[source] tiberius::error::Error),
}

/// What the statement task reports once the server has answered: the result's columns, or,
/// for a statement without rows, how many it changed.
struct Started {
    columns: Vec<ColumnInfo>,
    rows_changed: Option<u64>,
}

struct Page {
    rows: Vec<Row>,
    exhausted: bool,
}

/// How the statement task talks to its cursor.
struct Channels {
    started: oneshot::Sender<Result<Started, DriverError>>,
    demand: mpsc::Receiver<usize>,
    pages: mpsc::Sender<Result<Page, DriverError>>,
}

pub(crate) async fn execute(
    client: &Arc<Mutex<SqlClient>>,
    cancel: &Arc<CancelSlot>,
    sql: &str,
) -> Result<Box<dyn ResultCursor>, DriverError> {
    let client = Arc::clone(client).lock_owned().await;
    // Armed before the statement is sent, so a cancel that arrives as soon as it is running
    // isn't lost.
    let cancellation = cancel.arm();
    let (started_sender, started) = oneshot::channel();
    // Capacity 1: the task reads at most one page ahead of what was asked for.
    let (demand, demand_receiver) = mpsc::channel(1);
    let (page_sender, pages) = mpsc::channel(1);
    let channels = Channels {
        started: started_sender,
        demand: demand_receiver,
        pages: page_sender,
    };
    let task = tokio::spawn(run_statement(
        client,
        sql.to_owned(),
        channels,
        Arc::clone(cancel),
        cancellation,
    ));

    let started = started
        .await
        .map_err(|_| connection_error("the statement task stopped unexpectedly"))??;
    let exhausted = started.columns.is_empty();
    Ok(Box::new(SqlServerCursor {
        columns: started.columns,
        rows_changed: started.rows_changed,
        exhausted,
        demand: Some(demand),
        pages,
        task,
    }))
}

/// Where the error a cancel causes is reported: to whoever is waiting on the statement at
/// that point.
enum Report {
    Started(oneshot::Sender<Result<Started, DriverError>>),
    Page(mpsc::Sender<Result<Page, DriverError>>),
    /// The cursor's [`finish`](ResultCursor::finish), through the task's result.
    Finish,
}

enum Outcome {
    /// The statement ran to its end, or failed; what is left to report is the task's result.
    Ended(Result<(), DriverError>),
    Cancelled(Report),
}

/// The statement task: runs `sql`, then reads one page per demand until the result is
/// exhausted, fails, is cancelled, or the cursor goes away. Whatever is left is read and
/// discarded, so the connection is ready for the next statement, and how that went is the
/// task's result, which [`SqlServerCursor::finish`] reports.
async fn run_statement(
    mut client: OwnedMutexGuard<SqlClient>,
    sql: String,
    channels: Channels,
    slot: Arc<CancelSlot>,
    mut cancellation: Cancellation,
) -> Result<(), DriverError> {
    let outcome = read_statement(&mut client, &sql, channels, &mut cancellation).await;
    slot.disarm();
    let report = match outcome {
        Outcome::Ended(result) => return result,
        Outcome::Cancelled(report) => report,
    };
    let source = match client.cancel_query().await {
        Ok(()) => CancelError::Cancelled,
        Err(error) => CancelError::Unacknowledged(error),
    };
    let error = DriverError::Statement {
        source: Box::new(source),
        position: None,
    };
    // The receivers only disappear if the cursor's caller gave up; nothing to do then.
    match report {
        Report::Started(started) => {
            let _ = started.send(Err(error));
            Ok(())
        }
        Report::Page(pages) => {
            let _ = pages.send(Err(error)).await;
            Ok(())
        }
        Report::Finish => Err(error),
    }
}

async fn read_statement(
    client: &mut SqlClient,
    sql: &str,
    channels: Channels,
    cancellation: &mut Cancellation,
) -> Outcome {
    let Channels {
        started,
        demand,
        pages,
    } = channels;
    let channels = PageChannels { demand, pages };
    // `simple_query` sends the batch and then waits for its first result set, or the end of
    // its reply: a statement that takes long before it answers is cancelled here.
    let mut stream = match until_cancelled(client.simple_query(sql), cancellation).await {
        Next::Cancelled => return Outcome::Cancelled(Report::Started(started)),
        Next::Done(Ok(stream)) => stream,
        Next::Done(Err(error)) => {
            let _ = started.send(Err(statement_error(error, sql)));
            return Outcome::Ended(Ok(()));
        }
    };
    let columns = match until_cancelled(stream.columns(), cancellation).await {
        Next::Cancelled => return Outcome::Cancelled(Report::Started(started)),
        Next::Done(Err(error)) => {
            let _ = started.send(Err(statement_error(error, sql)));
            return Outcome::Ended(Ok(()));
        }
        Next::Done(Ok(columns)) => columns.map(<[tiberius::Column]>::to_vec),
    };
    let Some(columns) = columns else {
        // Without a result set, the reply has been read to its end, so the client is free to
        // ask how many rows the statement changed.
        drop(stream);
        let finished = rows_changed(client).await.map(|rows_changed| Started {
            columns: Vec::new(),
            rows_changed,
        });
        let _ = started.send(finished);
        return Outcome::Ended(Ok(()));
    };
    let column_info = columns.iter().map(values::column_info).collect();
    let cursor_waits = started
        .send(Ok(Started {
            columns: column_info,
            rows_changed: None,
        }))
        .is_ok();
    if cursor_waits && let Some(ended) = serve_pages(&mut stream, sql, channels, cancellation).await
    {
        return ended;
    }
    discard_rest(&mut stream, sql, cancellation).await
}

/// The cursor's half of [`Channels`], once the statement has started.
struct PageChannels {
    demand: mpsc::Receiver<usize>,
    pages: mpsc::Sender<Result<Page, DriverError>>,
}

/// Reads one page per demand, until the first result set is exhausted or the cursor stops
/// asking. Returns how the statement ended if it ended here, by failing or being cancelled.
async fn serve_pages(
    stream: &mut QueryStream<'_>,
    sql: &str,
    channels: PageChannels,
    cancellation: &mut Cancellation,
) -> Option<Outcome> {
    let PageChannels { mut demand, pages } = channels;
    // `columns` only peeked at the first result set's metadata, which the stream yields
    // before its rows.
    let mut in_first_result = false;
    loop {
        let max_rows = tokio::select! {
            biased;
            () = cancellation.requested() => return Some(Outcome::Cancelled(Report::Finish)),
            demand = demand.recv() => demand?,
        };
        let mut page = Vec::new();
        let mut exhausted = false;
        while page.len() < max_rows {
            match until_cancelled(stream.try_next(), cancellation).await {
                Next::Cancelled => return Some(Outcome::Cancelled(Report::Page(pages))),
                Next::Done(Ok(Some(QueryItem::Row(row)))) => page.push(values::cells(row)),
                Next::Done(Ok(Some(QueryItem::Metadata(_)))) if !in_first_result => {
                    in_first_result = true;
                }
                // The first result set ended: a batch's further results aren't shown.
                Next::Done(Ok(Some(QueryItem::Metadata(_)) | None)) => {
                    exhausted = true;
                    break;
                }
                Next::Done(Err(error)) => {
                    let _ = pages.send(Err(statement_error(error, sql))).await;
                    return Some(Outcome::Ended(Ok(())));
                }
            }
        }
        let sent = pages
            .send(Ok(Page {
                rows: page,
                exhausted,
            }))
            .await;
        if sent.is_err() || exhausted {
            return None;
        }
    }
}

/// Reads and drops the rows the cursor never asked for, and any further result sets, so the
/// connection is ready for the next statement.
async fn discard_rest(
    stream: &mut QueryStream<'_>,
    sql: &str,
    cancellation: &mut Cancellation,
) -> Outcome {
    loop {
        match until_cancelled(stream.try_next(), cancellation).await {
            Next::Cancelled => return Outcome::Cancelled(Report::Finish),
            Next::Done(Ok(Some(_))) => {}
            Next::Done(Ok(None)) => return Outcome::Ended(Ok(())),
            Next::Done(Err(error)) => return Outcome::Ended(Err(statement_error(error, sql))),
        }
    }
}

enum Next<T> {
    Done(T),
    Cancelled,
}

/// Waits for `work`, unless a cancel is requested first: then it is given
/// [`CANCEL_GRACE`] to finish what it is reading, and dropped either way.
async fn until_cancelled<F: Future>(work: F, cancellation: &mut Cancellation) -> Next<F::Output> {
    tokio::pin!(work);
    // The cancel first: rows already buffered keep `work` ready, which would otherwise win
    // every time.
    tokio::select! {
        biased;
        () = cancellation.requested() => {}
        output = &mut work => return Next::Done(output),
    }
    // Whatever arrives in the grace period is discarded: the point is only to stop between
    // tokens.
    let _ = tokio::time::timeout(CANCEL_GRACE, work).await;
    Next::Cancelled
}

/// How many rows the batch just run changed (or selected, for statements that only assign
/// variables). `@@ROWCOUNT` survives into the next batch, which TDS's own row counts would
/// make unnecessary, but `tiberius` keeps those to itself for a batch read as a stream.
async fn rows_changed(client: &mut SqlClient) -> Result<Option<u64>, DriverError> {
    let row = client
        .simple_query("SELECT CAST(@@ROWCOUNT AS bigint)")
        .await
        .map_err(connection_error)?
        .into_row()
        .await
        .map_err(connection_error)?;
    Ok(row
        .and_then(|row| row.get::<i64, _>(0))
        .and_then(|count| u64::try_from(count).ok()))
}

/// Runs `sql` for its effect alone, reading and discarding whatever it returns.
pub(crate) async fn run_quietly(client: &mut SqlClient, sql: &str) -> Result<(), DriverError> {
    client
        .simple_query(sql)
        .await
        .map_err(connection_error)?
        .into_results()
        .await
        .map_err(connection_error)?;
    Ok(())
}

/// Has the server describe the first result `sql` would return, which compiles it and
/// resolves its names without running it, with the session's temporary tables in view.
///
/// Not `SET NOEXEC ON`, which compiles as well but defers resolving the names of tables that
/// don't exist (yet), so a misspelt table would pass.
pub(crate) async fn validate(
    client: &mut SqlClient,
    sql: &str,
) -> Result<Option<Rejection>, DriverError> {
    let probe = format!(
        "EXEC sp_describe_first_result_set @tsql = {}",
        SQL_SERVER_QUOTING.string_literal(sql)
    );
    let described = match client.simple_query(probe).await {
        Ok(stream) => stream.into_results().await,
        Err(error) => Err(error),
    };
    match described {
        Ok(_) => Ok(None),
        Err(tiberius::error::Error::Server(error)) => Ok(Some(rejection(&error, sql))),
        Err(error) => Err(connection_error(error)),
    }
}

struct SqlServerCursor {
    columns: Vec<ColumnInfo>,
    rows_changed: Option<u64>,
    exhausted: bool,
    /// Dropping it tells the statement task to discard the rest and release the connection.
    demand: Option<mpsc::Sender<usize>>,
    pages: mpsc::Receiver<Result<Page, DriverError>>,
    task: JoinHandle<Result<(), DriverError>>,
}

#[async_trait]
impl ResultCursor for SqlServerCursor {
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

    /// Stops the statement task and waits for it, so the connection is free again. Reports
    /// an error the server sent while the rest of the result was discarded, a cancel's
    /// included.
    async fn finish(mut self: Box<Self>) -> Result<Option<u64>, DriverError> {
        self.demand = None;
        self.pages.close();
        (&mut self.task).await.map_err(connection_error)??;
        Ok(self.rows_changed)
    }
}
