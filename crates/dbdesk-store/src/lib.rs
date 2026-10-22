//! Where dbdesk keeps its files and secrets. See `docs/persistence.md` for the full layout.
//!
//! Everything here blocks (file system, OS keychain). The engine calls it from tokio's
//! blocking pool, never from an async worker or the UI thread.

mod data_sources;
mod paths;
mod secrets;

pub use data_sources::{DataSourceFile, StoreError};
pub use paths::{APP_NAME, AppPaths};
pub use secrets::{Keychain, SecretError, SecretStore};
