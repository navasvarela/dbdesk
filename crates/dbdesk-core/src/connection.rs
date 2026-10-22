use std::fmt;

use async_trait::async_trait;

use crate::{CatalogNode, CatalogPath, DriverError};

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

    async fn close(self: Box<Self>) -> Result<(), DriverError>;
}
