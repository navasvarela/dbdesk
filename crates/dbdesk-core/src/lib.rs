//! Domain types and the [`Connection`] trait shared by every dbdesk crate.
//!
//! Nothing here does I/O. Driver crates implement [`Connection`]; the engine owns the
//! connections and the UI only ever sees the plain data types.

mod catalog;
mod connection;
mod data_source;
mod error;
mod execution;
mod password;
mod table;

#[cfg(feature = "conformance")]
pub mod conformance;

pub use catalog::{CatalogNode, CatalogPath, FolderKind, NodeKind, PathSegment, quote_identifier};
pub use connection::{Connection, Rejection, RejectionKind, ServerInfo};
pub use data_source::{
    ConnectionConfig, DataSource, DataSourceId, DriverKind, PostgresConfig, SourceColor,
    SqliteConfig, TlsMode,
};
pub use error::{BoxError, DriverError, describe_error};
pub use execution::{CancelHandle, Cell, ColumnInfo, ResultCursor, Row, TransactionStatus};
pub use password::Password;
pub use table::{Identifier, RelationName, TableDescription, quote_always};
