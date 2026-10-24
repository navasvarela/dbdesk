//! The dbdesk engine: owns every piece of I/O on its own tokio runtime and talks to the UI
//! only through [`Command`]s and [`Event`]s.
//!
//! The UI thread must never wait on a database, so it never calls into a driver directly.
//! It sends commands with [`EngineHandle::send`] and drains events with
//! [`EngineHandle::try_recv`] once per frame. See `docs/adr/0003-ui-engine-message-passing.md`.

mod command_loop;
mod console_files;
mod events;
mod export;
mod handle;
mod history;
mod keymap;
mod open;
mod protocol;
mod registry;
mod releases;
mod repaint;
mod session;
mod settings;

pub use export::Export;
pub use handle::{EngineConfig, EngineError, EngineHandle};
pub use protocol::{
    Command, ConsoleId, EditFailure, Event, PasswordUpdate, RunId, RunOutcome, RunPurpose,
    SessionState, StatementCheck, StatementFailure, StatementSummary, TransactionEnd,
    TransactionMode,
};
pub use releases::{GithubReleases, Release, ReleaseError, ReleaseFeed};
pub use repaint::Repaint;
