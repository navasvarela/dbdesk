use std::fmt;

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    CancelHandle, CatalogNode, CatalogPath, DriverError, RelationName, ResultCursor,
    RoleDescription, TableDescription, TransactionStatus,
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

/// Why the database would refuse a statement, found by preparing it without running it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    /// The database's message.
    pub message: String,
    /// Byte offset into the statement where the problem was found, when the database says.
    pub position: Option<usize>,
    pub kind: RejectionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionKind {
    /// The statement can't be read.
    Syntax,
    /// It names a table, column, function or schema that doesn't exist (now).
    UnknownName,
    /// Anything else: a type mismatch, an ambiguous column, …
    Other,
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

    /// Describes the role at `path`, a node the roles folder lists, on engines whose users
    /// are roles (see `dbdesk_sql::Capabilities::users`).
    ///
    /// # Errors
    ///
    /// [`DriverError::InvalidPath`] where there are no roles, for a path that isn't a role,
    /// or for a role that no longer exists; [`DriverError::Connection`] if the catalog query
    /// fails.
    async fn describe_role(&mut self, path: &CatalogPath) -> Result<RoleDescription, DriverError> {
        Err(DriverError::InvalidPath(path.clone()))
    }

    /// Asks the database whether it would accept `sql`, one data query or change (`SELECT`,
    /// `WITH`, `VALUES`, `INSERT`, `UPDATE`, `DELETE`), without running it: PostgreSQL parses
    /// and analyses it as an unnamed prepared statement, SQLite compiles it. Names resolve as
    /// they would for a run on this connection (search path, temporary tables). `None` if it
    /// would be accepted.
    ///
    /// Callers must not ask while a result cursor is open, nor inside an open transaction on
    /// PostgreSQL, where a rejected statement aborts the transaction. Other statements may run
    /// something while being prepared (some SQLite pragmas do), so they must not be passed.
    ///
    /// # Errors
    ///
    /// [`DriverError::Connection`] if the database can't be asked.
    async fn validate(&mut self, sql: &str) -> Result<Option<Rejection>, DriverError>;

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
