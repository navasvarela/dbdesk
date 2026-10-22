//! The dbdesk engine: owns every piece of I/O on its own tokio runtime and talks to the UI
//! only through [`Command`]s and [`Event`]s.
//!
//! The UI thread must never wait on a database, so it never calls into a driver directly.
//! It sends commands with [`EngineHandle::send`] and drains events with
//! [`EngineHandle::try_recv`] once per frame. See `docs/adr/0003-ui-engine-message-passing.md`.

mod command_loop;
mod handle;
mod protocol;
mod repaint;

pub use handle::{EngineError, EngineHandle};
pub use protocol::{Command, Event};
pub use repaint::Repaint;
