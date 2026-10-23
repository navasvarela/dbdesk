use std::fmt;

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    CancelHandle, CatalogNode, CatalogPath, DriverError, RelationName, ResultCursor,
    TableDescription, TransactionStatus,
};

/// What a server reported about itself when the connection was opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    /// e.g. "PostgreSQL" or "SQLite".
    pub product: String,
    pub version: String,
}

impl fmt::Display for ServerInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.product, self.version)
    }
}

/// A live connection to a database, implemented once per driver.
///
/// Used as `Box<dyn Connection>`, hence `async-trait`: native `async fn` in traits is not
/// dyn-compatible yet. See `docs/adr/0002-native-drivers-behind-a-trait.md`.
#[async_trait]
pub trait Connection: Send {
    fn server_info(&self) -> &ServerInfo;

    /// Round-trips a trivial statement to prove the connection can run queries, not just
    /// that the handshake succeeded.
    async fn ping(&mut self) -> Result<(), DriverError>;

    /// The children of the catalog node at `path`, in display order. The root path lists the
    /// top level (databases for PostgreSQL, schemas for SQLite).
    ///
    /// # Errors
    ///
    /// [`DriverError::InvalidPath`] for a path this driver never returns, or
    /// [`DriverError::Connection`] if the catalog query fails.
    async fn children(&mut self, path: &CatalogPath) -> Result<Vec<CatalogNode>, DriverError>;

    /// The source of the object at `path` (a `CREATE VIEW`, function body, index definition,
    /// …), for nodes whose [`CatalogNode::has_definition`] is set.
    ///
    /// # Errors
    ///
    /// As for [`Connection::children`]. A node without a definition is an `InvalidPath`.
    async fn definition(&mut self, path: &CatalogPath) -> Result<String, DriverError>;

    /// Starts running one statement and returns a cursor over its result. Rows are read
    /// lazily, so a huge result costs nothing until it is paged through.
    ///
    /// # Errors
    ///
    /// [`DriverError::Statement`] if the statement is rejected or fails before producing a
    /// result, with the error's position when known.
    async fn execute(&mut self, sql: &str) -> Result<Box<dyn ResultCursor>, DriverError>;

    /// Resolves a table the way a statement on this connection would (search path, case
    /// folding) and describes it, for editing its rows. `None` if no such table exists, or it
    /// isn't a plain table (a view, say).
    ///
    /// # Errors
    ///
    /// [`DriverError::Connection`] if the catalog query fails.
    async fn describe_table(
        &mut self,
        table: &RelationName,
    ) -> Result<Option<TableDescription>, DriverError>;

    /// Whether a transaction is open on this connection, as the database sees it, so a
    /// transaction the user started with their own `BEGIN` counts too.
    ///
    /// # Errors
    ///
    /// [`DriverError::Connection`] if the database can't be asked.
    async fn transaction_status(&mut self) -> Result<TransactionStatus, DriverError>;

    /// A handle for cancelling this connection's running statement from another task. Valid
    /// for the connection's whole lifetime.
    fn cancel_handle(&self) -> Arc<dyn CancelHandle>;

    async fn close(self: Box<Self>) -> Result<(), DriverError>;
}
