//! Where dbdesk keeps its files and secrets. See `docs/persistence.md` for the full layout.
//!
//! Everything here blocks (file system, OS keychain). The engine calls it from tokio's
//! blocking pool, never from an async worker or the UI thread.

mod atomic;
mod consoles;
mod data_sources;
mod history;
mod paths;
mod secrets;

pub use consoles::{
    ConsoleFiles, ConsoleKey, ConsoleName, ConsoleStoreError, InvalidConsoleName, SavedConsole,
};
pub use data_sources::{DataSourceFile, StoreError};
pub use history::{History, HistoryEntry, HistoryError, HistoryFile, HistoryOutcome};
pub use paths::{APP_NAME, AppPaths};
pub use secrets::{Keychain, SecretError, SecretStore};
