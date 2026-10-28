//! Running statements: result columns, rows as text, cursors and cancellation.

use async_trait::async_trait;

use crate::DriverError;

/// A result column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    /// The declared type, when the driver knows it cheaply (SQLite does; PostgreSQL's text
    /// protocol doesn't report it).
    pub type_name: Option<String>,
}

/// One value as the database formats it for display, or `None` for SQL `NULL`.
///
/// Text rather than typed values: PostgreSQL's text protocol formats every type, including
/// ones DbRover has never heard of, exactly as `psql` shows them. Typed values can come later,
/// when editing data needs them.
pub type Cell = Option<String>;

pub type Row = Vec<Cell>;

/// The rows of one running statement, read a page at a time.
///
/// While a cursor is open its connection is busy: finish the cursor before using the
/// connection for anything else.
#[async_trait]
pub trait ResultCursor: Send {
    /// The result's columns; empty for statements that return no rows.
    fn columns(&self) -> &[ColumnInfo];

    /// Reads up to `max_rows` more rows. Returns fewer, possibly none, once the result is
    /// exhausted.
    ///
    /// # Errors
    ///
    /// [`DriverError::Statement`] if the statement fails while producing rows.
    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>, DriverError>;

    /// Whether every row has been read.
    fn is_exhausted(&self) -> bool;

    /// Ends the statement and discards any rows not yet read. To stop the server producing
    /// them, call [`CancelHandle::cancel`] first.
    ///
    /// Returns the number of rows the statement changed (or returned), when the database
    /// reports it.
    ///
    /// # Errors
    ///
    /// [`DriverError::Statement`] if the statement fails while finishing, including because
    /// it was cancelled.
    async fn finish(self: Box<Self>) -> Result<Option<u64>, DriverError>;
}

/// Whether a connection is inside a transaction block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TransactionStatus {
    /// Each statement commits on its own.
    #[default]
    Idle,
    /// Changes stay pending until `COMMIT` or `ROLLBACK`.
    InTransaction,
    /// A statement failed inside the transaction, and the database accepts nothing but
    /// `ROLLBACK` (PostgreSQL only; SQLite keeps the transaction usable).
    Failed,
}

/// Interrupts whatever a connection is running, from outside the task that runs it.
#[async_trait]
pub trait CancelHandle: Send + Sync {
    /// Asks the database to stop the current statement. Best effort: the statement may
    /// finish anyway, and nothing happens if none is running.
    ///
    /// # Errors
    ///
    /// [`DriverError::Connection`] if the cancel request couldn't be delivered.
    async fn cancel(&self) -> Result<(), DriverError>;
}
