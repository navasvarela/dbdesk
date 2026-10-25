//! The dbdesk engine: owns every piece of I/O on its own tokio runtime and talks to the UI
//! only through [`Command`]s and [`Event`]s.
//!
//! The UI thread must never wait on a database, so it never calls into a driver directly.
//! It sends commands with [`EngineHandle::send`] and drains events with
//! [`EngineHandle::try_recv`] once per frame. See `docs/adr/0003-ui-engine-message-passing.md`.

mod command_loop;
mod console_files;
mod environment;
mod events;
mod export;
mod handle;
mod history;
mod keymap;
mod open;
mod passwords;
mod projects;
mod protocol;
pub mod read_only;
pub mod read_write;
mod registry;
mod releases;
mod repaint;
mod session;
mod settings;
mod transfer;
mod tunnels;
mod updater;

pub use dbdesk_ssh::HostKey;
pub use environment::ProcessEnvironment;
pub use export::Export;
pub use handle::{EngineConfig, EngineError, EngineHandle};
pub use open::{OpenError, open_data_source, open_data_source_for_mcp};
pub use passwords::PasswordNeeded;
pub use protocol::{
    Command, ConsoleId, DatabaseFile, EditFailure, Event, PasswordKept, PasswordUpdate, RunId,
    RunOutcome, RunPurpose, SessionState, StatementCheck, StatementFailure, StatementSummary,
    TransactionEnd, TransactionMode,
};
pub use releases::{GithubReleases, Release, ReleaseError, ReleaseFeed};
pub use repaint::Repaint;
pub use tunnels::Tunnels;
pub use updater::{InstallError, InstallSupport, PackagerInstaller, UpdateInstaller};
