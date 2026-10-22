use std::sync::Arc;
use std::time::Instant;

use dbdesk_core::{CatalogPath, Connection, DataSource, describe_error};
use dbdesk_store::SecretStore;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::events::EventSink;
use crate::open::open;
use crate::{Event, PasswordUpdate, SessionState};

/// Work for a session's connection. Answered with events, in the order received.
#[derive(Debug)]
pub(crate) enum SessionRequest {
    Children(CatalogPath),
    Definition(CatalogPath),
}

/// A data source's live connection, owned by its own task so a slow connect or query never
/// holds up the engine's command loop.
pub(crate) struct Session {
    /// Dropping this is the disconnect signal; nothing is ever sent on it. That way a session
    /// also disconnects when the engine drops it during shutdown.
    disconnect: oneshot::Sender<()>,
    /// Requests sent while still connecting wait here and run once connected.
    requests: UnboundedSender<SessionRequest>,
    task: JoinHandle<()>,
}

impl Session {
    pub(crate) fn start(
        data_source: DataSource,
        secrets: Arc<dyn SecretStore>,
        events: EventSink,
    ) -> Self {
        let (disconnect, disconnect_signal) = oneshot::channel();
        let (requests, request_receiver) = mpsc::unbounded_channel();
        let context = SessionContext {
            data_source,
            events,
        };
        let task = tokio::spawn(run(context, secrets, disconnect_signal, request_receiver));
        Self {
            disconnect,
            requests,
            task,
        }
    }

    /// Whether the session has ended, by disconnecting or by failing to connect.
    pub(crate) fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    pub(crate) fn request(&self, request: SessionRequest) {
        // Fails only if the session has ended; its Disconnected or Failed event tells the UI
        // to forget anything it was waiting for.
        if self.requests.send(request).is_err() {
            tracing::debug!("dropped a request for a session that has ended");
        }
    }

    /// Disconnects and waits until the connection is closed.
    pub(crate) async fn stop(self) {
        drop(self.disconnect);
        drop(self.requests);
        if let Err(error) = self.task.await {
            tracing::error!("session task failed: {error}");
        }
    }
}

struct SessionContext {
    data_source: DataSource,
    events: EventSink,
}

impl SessionContext {
    fn name(&self) -> &str {
        &self.data_source.name
    }

    fn notify(&self, state: SessionState) {
        self.events.send(Event::SessionChanged {
            id: self.data_source.id,
            state,
        });
    }
}

async fn run(
    context: SessionContext,
    secrets: Arc<dyn SecretStore>,
    mut disconnect_signal: oneshot::Receiver<()>,
    mut requests: UnboundedReceiver<SessionRequest>,
) {
    let Some(mut connection) = connect(&context, &secrets, &mut disconnect_signal).await else {
        return;
    };
    serve(
        &context,
        connection.as_mut(),
        &mut disconnect_signal,
        &mut requests,
    )
    .await;
    if let Err(error) = connection.close().await {
        tracing::warn!(data_source = %context.name(), "error while disconnecting: {}", describe_error(&error));
    }
    tracing::info!(data_source = %context.name(), "disconnected");
    context.notify(SessionState::Disconnected);
}

/// Opens the connection, reporting progress. `None` if it failed or was cancelled.
async fn connect(
    context: &SessionContext,
    secrets: &Arc<dyn SecretStore>,
    disconnect_signal: &mut oneshot::Receiver<()>,
) -> Option<Box<dyn Connection>> {
    let name = context.name();
    context.notify(SessionState::Connecting);
    tracing::info!(data_source = %name, "connecting to {}", context.data_source.config);
    let started = Instant::now();
    let attempt = tokio::select! {
        attempt = open(&context.data_source, secrets, &PasswordUpdate::Keep) => attempt,
        _ = disconnect_signal => {
            tracing::info!(data_source = %name, "connection attempt cancelled");
            context.notify(SessionState::Disconnected);
            return None;
        }
    };

    match attempt {
        Ok(connection) => {
            let server = connection.server_info().clone();
            tracing::info!(
                data_source = %name,
                elapsed_ms = started.elapsed().as_millis(),
                "connected to {server}"
            );
            context.notify(SessionState::Connected(server));
            Some(connection)
        }
        Err(error) => {
            let error = describe_error(&error);
            tracing::warn!(data_source = %name, "connection failed: {error}");
            context.notify(SessionState::Failed(error));
            None
        }
    }
}

/// Answers requests one at a time until told to disconnect. A disconnect also abandons the
/// request in progress, so a slow catalog query never delays it.
async fn serve(
    context: &SessionContext,
    connection: &mut dyn Connection,
    disconnect_signal: &mut oneshot::Receiver<()>,
    requests: &mut UnboundedReceiver<SessionRequest>,
) {
    loop {
        let request = tokio::select! {
            _ = &mut *disconnect_signal => return,
            request = requests.recv() => match request {
                Some(request) => request,
                None => return,
            },
        };
        tokio::select! {
            _ = &mut *disconnect_signal => return,
            () = handle(context, connection, request) => {}
        }
    }
}

async fn handle(
    context: &SessionContext,
    connection: &mut dyn Connection,
    request: SessionRequest,
) {
    let id = context.data_source.id;
    let name = context.name();
    let started = Instant::now();
    match request {
        SessionRequest::Children(path) => {
            let result = connection.children(&path).await;
            match &result {
                Ok(nodes) => tracing::debug!(
                    data_source = %name,
                    elapsed_ms = started.elapsed().as_millis(),
                    "loaded {} nodes under {path}",
                    nodes.len()
                ),
                Err(error) => tracing::warn!(
                    data_source = %name,
                    "could not load {path}: {}",
                    describe_error(error)
                ),
            }
            context.events.send(Event::ChildrenLoaded {
                id,
                path,
                result: result.map_err(|error| describe_error(&error)),
            });
        }
        SessionRequest::Definition(path) => {
            let result = connection.definition(&path).await;
            if let Err(error) = &result {
                tracing::warn!(
                    data_source = %name,
                    "could not load the definition of {path}: {}",
                    describe_error(error)
                );
            }
            context.events.send(Event::DefinitionLoaded {
                id,
                path,
                result: result.map_err(|error| describe_error(&error)),
            });
        }
    }
}
