use std::sync::Arc;
use std::time::Instant;

use dbdesk_core::{Connection, DataSource, describe_error};
use dbdesk_store::SecretStore;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::events::EventSink;
use crate::open::open;
use crate::{Event, PasswordUpdate, SessionState};

/// A data source's live connection, owned by its own task so a slow connect or query never
/// holds up the engine's command loop.
pub(crate) struct Session {
    /// Dropping this is the disconnect signal; nothing is ever sent on it. That way a session
    /// also disconnects when the engine drops it during shutdown.
    disconnect: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Session {
    pub(crate) fn start(
        data_source: DataSource,
        secrets: Arc<dyn SecretStore>,
        events: EventSink,
    ) -> Self {
        let (disconnect, disconnect_signal) = oneshot::channel();
        let task = tokio::spawn(run(data_source, secrets, events, disconnect_signal));
        Self { disconnect, task }
    }

    /// Whether the session has ended, by disconnecting or by failing to connect.
    pub(crate) fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Disconnects and waits until the connection is closed.
    pub(crate) async fn stop(self) {
        drop(self.disconnect);
        if let Err(error) = self.task.await {
            tracing::error!("session task failed: {error}");
        }
    }
}

async fn run(
    data_source: DataSource,
    secrets: Arc<dyn SecretStore>,
    events: EventSink,
    mut disconnect_signal: oneshot::Receiver<()>,
) {
    let notify = |state| {
        events.send(Event::SessionChanged {
            id: data_source.id,
            state,
        });
    };
    let name = &data_source.name;

    notify(SessionState::Connecting);
    tracing::info!(data_source = %name, "connecting to {}", data_source.config);
    let started = Instant::now();
    let attempt = tokio::select! {
        attempt = open(&data_source, &secrets, &PasswordUpdate::Keep) => attempt,
        _ = &mut disconnect_signal => {
            tracing::info!(data_source = %name, "connection attempt cancelled");
            notify(SessionState::Disconnected);
            return;
        }
    };
    let connection = match attempt {
        Ok(connection) => connection,
        Err(error) => {
            let error = describe_error(&error);
            tracing::warn!(data_source = %name, "connection failed: {error}");
            notify(SessionState::Failed(error));
            return;
        }
    };

    let server = connection.server_info().clone();
    tracing::info!(
        data_source = %name,
        elapsed_ms = started.elapsed().as_millis(),
        "connected to {server}"
    );
    notify(SessionState::Connected(server));

    hold_until_disconnect(connection, disconnect_signal, name).await;
    notify(SessionState::Disconnected);
}

async fn hold_until_disconnect(
    connection: Box<dyn Connection>,
    disconnect_signal: oneshot::Receiver<()>,
    name: &str,
) {
    // Resolves with an error when the sender is dropped, which is the only way it resolves.
    let _sender_dropped = disconnect_signal.await;
    if let Err(error) = connection.close().await {
        tracing::warn!(data_source = %name, "error while disconnecting: {}", describe_error(&error));
    }
    tracing::info!(data_source = %name, "disconnected");
}
