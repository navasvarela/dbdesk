//! The two ways a command's result is read: a server cursor followed a page at a time, in a
//! task that owns it, or a table already in memory.

use std::collections::VecDeque;

use async_trait::async_trait;
use bson::{Bson, Document};
use dbrover_core::{ColumnInfo, DriverError, ResultCursor, ResultShape, Row};
use mongodb::{Cursor, Database};
use tokio::sync::{mpsc, oneshot};

use crate::convert::{cell, document_text};
use crate::errors;
use crate::plan::CursorRows;

/// How many documents a batch from the server holds: a grid page, so each page the grid asks
/// for is about one round trip.
pub(crate) const BATCH_SIZE: u32 = 500;

/// The column a result of documents has: each row is one document's text (ADR 0006).
const DOCUMENT_COLUMN: &str = "document";

fn columns(names: &[&str]) -> Vec<ColumnInfo> {
    names
        .iter()
        .map(|name| ColumnInfo {
            name: (*name).to_owned(),
            type_name: None,
        })
        .collect()
}

/// A result in memory: a command's whole answer.
pub(crate) struct Table {
    pub(crate) columns: Vec<ColumnInfo>,
    pub(crate) rows: VecDeque<Row>,
    pub(crate) shape: ResultShape,
    /// The documents the command changed, for a write.
    pub(crate) changed: Option<u64>,
}

impl Table {
    pub(crate) fn rows(names: &[&str], rows: Vec<Row>) -> Self {
        Self {
            columns: columns(names),
            rows: rows.into(),
            shape: ResultShape::Rows,
            changed: None,
        }
    }

    pub(crate) fn documents(documents: Vec<Document>) -> Self {
        Self {
            columns: columns(&[DOCUMENT_COLUMN]),
            rows: documents
                .into_iter()
                .map(|document| vec![Some(document_text(document))])
                .collect(),
            shape: ResultShape::Documents,
            changed: None,
        }
    }

    /// A result with no rows, for a command that answers nothing, as `use`.
    pub(crate) fn empty() -> Self {
        Self {
            columns: Vec::new(),
            rows: VecDeque::new(),
            shape: ResultShape::Rows,
            changed: None,
        }
    }

    pub(crate) fn changing(mut self, changed: u64) -> Self {
        self.changed = Some(changed);
        self
    }
}

#[async_trait]
impl ResultCursor for Table {
    fn columns(&self) -> &[ColumnInfo] {
        &self.columns
    }

    fn shape(&self) -> ResultShape {
        self.shape
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>, DriverError> {
        let count = max_rows.min(self.rows.len());
        Ok(self.rows.drain(..count).collect())
    }

    fn is_exhausted(&self) -> bool {
        self.rows.is_empty()
    }

    async fn finish(self: Box<Self>) -> Result<Option<u64>, DriverError> {
        Ok(self.changed)
    }
}

/// What the task that owns a server cursor answers for a page.
type Page = Result<(Vec<Document>, bool), mongodb::error::Error>;

struct PageRequest {
    max: usize,
    reply: oneshot::Sender<Page>,
}

/// A server cursor, read a page at a time by a task of its own.
///
/// The driver warns that dropping one of its futures part-way can leave a connection in an
/// unknown state, and the engine drops a page's future when a console disconnects. So the
/// cursor lives in a task that always finishes the page it started; this side only asks and
/// waits. Dropping this side ends the task, and the driver closes a cursor dropped before its
/// end with `killCursors`.
pub(crate) struct DocumentCursor {
    columns: Vec<ColumnInfo>,
    rows: CursorRows,
    requests: mpsc::Sender<PageRequest>,
    exhausted: bool,
}

impl DocumentCursor {
    /// Runs `command`, which answers with a cursor, and waits for its first batch.
    ///
    /// # Errors
    ///
    /// The server refused the command, or can't be reached.
    pub(crate) async fn start(
        database: Database,
        command: Document,
        rows: CursorRows,
        comment: Bson,
    ) -> Result<Self, DriverError> {
        let (requests, mut pending) = mpsc::channel::<PageRequest>(1);
        let (started, opened) = oneshot::channel();
        tokio::spawn(async move {
            let cursor = database
                .run_cursor_command(command)
                .batch_size(BATCH_SIZE)
                .comment(comment)
                .await;
            let mut cursor = match cursor {
                Ok(cursor) => {
                    let _ = started.send(Ok(()));
                    cursor
                }
                Err(error) => {
                    let _ = started.send(Err(error));
                    return;
                }
            };
            let mut ahead = None;
            while let Some(request) = pending.recv().await {
                let page = read_page(&mut cursor, &mut ahead, request.max).await;
                let _ = request.reply.send(page);
            }
        });
        opened.await.map_err(ended)?.map_err(errors::command)?;
        let columns = match rows {
            CursorRows::Documents => columns(&[DOCUMENT_COLUMN]),
            CursorRows::Names => columns(&["name"]),
        };
        Ok(Self {
            columns,
            rows,
            requests,
            exhausted: false,
        })
    }
}

/// The cursor's task is gone, which only a panic in the driver would do.
fn ended<E>(_: E) -> DriverError {
    errors::statement("the command's task ended early", None)
}

/// Reads up to `max` documents, and whether the cursor ended with them. One document past the
/// page is read ahead and kept in `ahead` for the next page, so the grid knows exactly when a
/// result has no more.
async fn read_page(
    cursor: &mut Cursor<Document>,
    ahead: &mut Option<Document>,
    max: usize,
) -> Page {
    let mut documents: Vec<Document> = ahead.take().into_iter().collect();
    while documents.len() < max {
        if !cursor.advance().await? {
            return Ok((documents, true));
        }
        documents.push(cursor.deserialize_current()?);
    }
    if !cursor.advance().await? {
        return Ok((documents, true));
    }
    *ahead = Some(cursor.deserialize_current()?);
    Ok((documents, false))
}

#[async_trait]
impl ResultCursor for DocumentCursor {
    fn columns(&self) -> &[ColumnInfo] {
        &self.columns
    }

    fn shape(&self) -> ResultShape {
        match self.rows {
            CursorRows::Documents => ResultShape::Documents,
            CursorRows::Names => ResultShape::Rows,
        }
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>, DriverError> {
        if self.exhausted {
            return Ok(Vec::new());
        }
        let (reply, answer) = oneshot::channel();
        let request = PageRequest {
            max: max_rows,
            reply,
        };
        self.requests.send(request).await.map_err(ended)?;
        let (documents, exhausted) = answer.await.map_err(ended)?.map_err(errors::command)?;
        self.exhausted = exhausted;
        Ok(match self.rows {
            CursorRows::Documents => documents
                .into_iter()
                .map(|document| vec![Some(document_text(document))])
                .collect(),
            CursorRows::Names => documents
                .into_iter()
                .map(|mut document| vec![document.remove("name").and_then(cell)])
                .collect(),
        })
    }

    fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    async fn finish(self: Box<Self>) -> Result<Option<u64>, DriverError> {
        Ok(None)
    }
}
