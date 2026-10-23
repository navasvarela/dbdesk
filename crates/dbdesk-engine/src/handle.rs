use std::sync::Arc;
use std::time::Duration;

use dbdesk_store::{
    ConsoleFiles, DataSourceFile, HistoryFile, KeymapFile, SecretStore, SettingsFile,
};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender, error::TryRecvError};
use tokio::task::{JoinError, JoinHandle};

use crate::command_loop;
use crate::events::EventSink;
use crate::{Command, Event, Repaint};

/// How long [`EngineHandle::shutdown`] waits for in-flight work before abandoning it.
///
/// Chosen by feel, not measured: long enough for a cancel request to reach a database server
/// once queries exist (M3), short enough that quitting never feels stuck.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("could not start the engine runtime")]
    Runtime(#[source] std::io::Error),
    #[error("the engine is no longer running")]
    Stopped,
    #[error("the engine's command loop failed")]
    CommandLoopFailed(#[source] JoinError),
    #[error("the engine did not stop within {0:?}")]
    ShutdownTimedOut(Duration),
}

/// Where the engine keeps what it persists.
pub struct EngineConfig {
    pub data_sources: DataSourceFile,
    pub secrets: Arc<dyn SecretStore>,
    pub consoles: ConsoleFiles,
    pub history: HistoryFile,
    pub keymap: KeymapFile,
    pub settings: SettingsFile,
}

/// The UI's connection to a running engine.
///
/// Owns the engine's tokio runtime, so the engine lives exactly as long as this handle.
pub struct EngineHandle {
    runtime: Runtime,
    commands: UnboundedSender<Command>,
    events: UnboundedReceiver<Event>,
    command_loop: JoinHandle<()>,
}

impl EngineHandle {
    /// Starts the engine runtime and its command loop.
    ///
    /// The engine loads the data sources from `config` and reports them as its first events.
    /// `repaint` is called after every event so an idle UI wakes up to drain it.
    ///
    /// # Errors
    ///
    /// [`EngineError::Runtime`] if the OS refuses to create the runtime's worker threads.
    pub fn start(config: EngineConfig, repaint: impl Repaint) -> Result<Self, EngineError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .thread_name("dbdesk-engine")
            .enable_all()
            .build()
            .map_err(EngineError::Runtime)?;
        let (commands, command_receiver) = mpsc::unbounded_channel();
        let (event_sender, events) = mpsc::unbounded_channel();
        let command_loop = runtime.spawn(command_loop::run(
            config,
            command_receiver,
            EventSink::new(event_sender, repaint),
        ));

        Ok(Self {
            runtime,
            commands,
            events,
            command_loop,
        })
    }

    /// Queues a command for the engine. Never blocks.
    ///
    /// # Errors
    ///
    /// [`EngineError::Stopped`] if the command loop has exited.
    pub fn send(&self, command: Command) -> Result<(), EngineError> {
        self.commands
            .send(command)
            .map_err(|_| EngineError::Stopped)
    }

    /// Takes the next pending event, if any. Never blocks.
    ///
    /// # Errors
    ///
    /// [`EngineError::Stopped`] if the command loop has exited and every event it sent has
    /// already been received.
    pub fn try_recv(&mut self) -> Result<Option<Event>, EngineError> {
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(EngineError::Stopped),
        }
    }

    /// Stops the engine: closes the command channel so the command loop finishes, waits for
    /// it up to a short timeout, then shuts the runtime down.
    ///
    /// Must be called from outside the engine runtime (the UI thread), because it blocks.
    ///
    /// # Errors
    ///
    /// [`EngineError::CommandLoopFailed`] if the command loop panicked, or
    /// [`EngineError::ShutdownTimedOut`] if it did not finish in time.
    pub fn shutdown(self) -> Result<(), EngineError> {
        let Self {
            runtime,
            commands,
            events: _,
            command_loop,
        } = self;
        drop(commands);

        let finished =
            runtime.block_on(async { tokio::time::timeout(SHUTDOWN_TIMEOUT, command_loop).await });
        runtime.shutdown_background();

        match finished {
            Ok(Ok(())) => Ok(()),
            Ok(Err(join_error)) => Err(EngineError::CommandLoopFailed(join_error)),
            Err(_elapsed) => Err(EngineError::ShutdownTimedOut(SHUTDOWN_TIMEOUT)),
        }
    }
}
