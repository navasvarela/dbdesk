//! Domain types and the [`Connection`] trait shared by every dbdesk crate.
//!
//! Nothing here does I/O. Driver crates implement [`Connection`]; the engine owns the
//! connections and the UI only ever sees the plain data types.

mod connection;
mod data_source;
mod error;
mod password;

#[cfg(feature = "conformance")]
pub mod conformance;

pub use connection::{Connection, ServerInfo};
pub use data_source::{
    ConnectionConfig, DataSource, DataSourceId, DriverKind, PostgresConfig, SqliteConfig,
};
pub use error::{BoxError, DriverError, describe_error};
pub use password::Password;
