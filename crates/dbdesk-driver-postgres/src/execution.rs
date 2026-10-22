//! Running user statements over the simple query protocol.
//!
//! The simple protocol returns every value as text, formatted by the server exactly as `psql`
//! shows it, so no type needs decoding here. Rows stream from the socket as the cursor reads
//! them: tokio-postgres stops reading the socket while nobody consumes the stream, and TCP
//! backpressure then pauses the server.

use std::pin::Pin;

use async_trait::async_trait;
use dbdesk_core::{CancelHandle, ColumnInfo, DriverError, ResultCursor, Row};
use futures_util::StreamExt;
use tokio_postgres::error::ErrorPosition;
use tokio_postgres::{CancelToken, Client, NoTls, SimpleQueryMessage, SimpleQueryStream};

use crate::connection_error;

pub(crate) async fn execute(
    client: &Client,
    sql: &str,
) -> Result<Box<dyn ResultCursor>, DriverError> {
    let to_error = |error| statement_error(sql, error);
    let stream = client.simple_query_raw(sql).await.map_err(to_error)?;
    let mut cursor = PostgresCursor {
        stream: Box::pin(stream),
        columns: Vec::new(),
        rows_reported: None,
        exhausted: false,
        sql: sql.to_owned(),
    };
    cursor.read_header().await?;
    Ok(Box::new(cursor))
}

/// Maps a server error to [`DriverError::Statement`], converting PostgreSQL's 1-based
/// character position into a byte offset into `sql`.
fn statement_error(sql: &str, error: tokio_postgres::Error) -> DriverError {
    let position = error
        .as_db_error()
        .and_then(|db_error| db_error.position())
        .and_then(|position| match position {
            ErrorPosition::Original(characters) => {
                let index = usize::try_from(*characters).ok()?.checked_sub(1)?;
                sql.char_indices().nth(index).map(|(byte, _)| byte)
            }
            ErrorPosition::Internal { .. } => None,
        });
    DriverError::Statement {
        source: Box::new(error),
        position,
    }
}

struct PostgresCursor {
    stream: Pin<Box<SimpleQueryStream>>,
    columns: Vec<ColumnInfo>,
    /// The server's row count from `CommandComplete`, once the statement has completed.
    rows_reported: Option<u64>,
    exhausted: bool,
    sql: String,
}

impl PostgresCursor {
    async fn next_message(&mut self) -> Result<Option<SimpleQueryMessage>, DriverError> {
        match self.stream.next().await {
            Some(Ok(message)) => Ok(Some(message)),
            Some(Err(error)) => Err(statement_error(&self.sql, error)),
            None => Ok(None),
        }
    }

    /// Reads until the column list or the completion of a statement without rows.
    async fn read_header(&mut self) -> Result<(), DriverError> {
        loop {
            match self.next_message().await? {
                Some(SimpleQueryMessage::RowDescription(columns)) => {
                    self.columns = columns
                        .iter()
                        .map(|column| ColumnInfo {
                            name: column.name().to_owned(),
                            type_name: None,
                        })
                        .collect();
                    return Ok(());
                }
                Some(SimpleQueryMessage::CommandComplete(rows)) => {
                    self.complete(rows);
                    return Ok(());
                }
                // Rows can't precede their description; skip anything else.
                Some(_) => {}
                None => {
                    // An empty query (only comments) completes with no messages at all.
                    self.exhausted = true;
                    return Ok(());
                }
            }
        }
    }

    fn complete(&mut self, rows: u64) {
        self.rows_reported = Some(rows);
        self.exhausted = true;
    }
}

#[async_trait]
impl ResultCursor for PostgresCursor {
    fn columns(&self) -> &[ColumnInfo] {
        &self.columns
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>, DriverError> {
        let mut page = Vec::new();
        while !self.exhausted && page.len() < max_rows {
            match self.next_message().await? {
                Some(SimpleQueryMessage::Row(row)) => {
                    page.push(
                        (0..row.len())
                            .map(|i| row.get(i).map(str::to_owned))
                            .collect(),
                    );
                }
                Some(SimpleQueryMessage::CommandComplete(rows)) => self.complete(rows),
                // A second result set means the text held more than one statement, which the
                // console's splitter is meant to prevent. Show the first; `finish` drains
                // the rest.
                Some(_) | None => self.exhausted = true,
            }
        }
        Ok(page)
    }

    fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    /// Reads and discards whatever the server still sends, so the connection is ready for
    /// the next statement. After a cancel, that is the "canceling statement" error.
    async fn finish(mut self: Box<Self>) -> Result<Option<u64>, DriverError> {
        while let Some(message) = self.next_message().await? {
            if let SimpleQueryMessage::CommandComplete(rows) = message {
                self.rows_reported.get_or_insert(rows);
            }
        }
        Ok(self.rows_reported)
    }
}

/// Cancels through a separate connection to the server, as `psql`'s Ctrl+C does.
pub(crate) struct PostgresCancel(pub(crate) CancelToken);

#[async_trait]
impl CancelHandle for PostgresCancel {
    async fn cancel(&self) -> Result<(), DriverError> {
        self.0.cancel_query(NoTls).await.map_err(connection_error)
    }
}
