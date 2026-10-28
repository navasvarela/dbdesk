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
mod placeholder;
mod quoting;
mod role;
mod table;

#[cfg(feature = "conformance")]
pub mod conformance;

pub use catalog::{CatalogNode, CatalogPath, FolderKind, NodeKind, PathSegment};
pub use connection::{Connection, Rejection, RejectionKind, ServerInfo};
pub use data_source::{
    ClickHouseConfig, ConnectionConfig, DataSource, DataSourceId, DriverKind, DuckDbConfig,
    DuckDbDatabase, McpLogin, MysqlConfig, Port, PostgresConfig, ProjectId, Route, SafetyLevel,
    ServerConfig, SourceColor, SqlServerConfig, SqliteConfig, SshConfig, SshLogin, TlsMode,
};
pub use error::{BoxError, DriverError, describe_error};
pub use execution::{CancelHandle, Cell, ColumnInfo, ResultCursor, Row, TransactionStatus};
pub use password::Password;
pub use placeholder::{
    ExpandError, PlaceholderError, Setting, Variables, expand, expand_path, has_placeholder,
};
pub use quoting::Quoting;
pub use role::{RoleAttribute, RoleDescription};
pub use table::{
    CheckConstraint, ColumnDescription, ForeignKey, ForeignKeyConstraint, Identifier, Identity,
    IdentityGeneration, ReferentialAction, RelationName, TableConstraints, TableDescription,
    TableName, UniqueConstraint,
};
