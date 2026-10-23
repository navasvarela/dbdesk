//! Sessions: one task per live connection, owning it and answering requests in order.
//!
//! The explorer has one session per data source; each query console has its own, so a long
//! query never holds up browsing or another console.

mod catalog;
mod edits;
mod query;
mod transaction;
mod validate;

use std::future::Future;
use std::sync::Arc;
use std::time::Instant;

use dbdesk_core::{
    CancelHandle, CatalogPath, Connection, DataSource, DataSourceId, RelationName,
    TransactionStatus, describe_error,
};
use dbdesk_sql::Dialect;
use dbdesk_store::{HistoryEntry, SecretStore};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::events::EventSink;
use crate::history::HistoryLog;
use crate::open::open;
use crate::{
    ConsoleId, Event, PasswordUpdate, RunId, SessionState, TransactionEnd, TransactionMode,
};

/// Work for a session's connection. Answered with events, in the order received.
#[derive(Debug)]
pub(crate) enum SessionRequest {
    Children(CatalogPath),
    Definition(CatalogPath),
    Execute(Script),
    FetchMore(RunId),
    EndTransaction(TransactionEnd),
    DescribeTable(RelationName),
    ApplyEdits(Vec<String>),
    /// Prepares statements without running them; see [`crate::Command::ValidateStatements`].
    Validate(Vec<String>),
}

/// Statements to run in a console, in order.
#[derive(Debug)]
pub(crate) struct Script {
    pub(crate) run: RunId,
    pub(crate) statements: Vec<String>,
    pub(crate) transactions: TransactionMode,
}

/// Who a session works for, which decides how its state changes are reported.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SessionOwner {
    Explorer(DataSourceId),
    Console(ConsoleId),
}

/// What a session needs from the engine.
pub(crate) struct Services {
    pub(crate) secrets: Arc<dyn SecretStore>,
    pub(crate) events: EventSink,
    pub(crate) history: HistoryLog,
}

/// A live connection, owned by its own task so a slow connect or query never holds up the
/// engine's command loop.
pub(crate) struct Session {
    /// Dropping this is the disconnect signal; nothing is ever sent on it. That way a session
    /// also disconnects when the engine drops it during shutdown.
    disconnect: oneshot::Sender<()>,
    /// Requests sent while still connecting wait here and run once connected.
    requests: UnboundedSender<SessionRequest>,
    /// The run most recently asked to stop. A watch rather than a request, because it must
    /// reach the task while it is busy with that very run.
    cancel: watch::Sender<Option<RunId>>,
    task: JoinHandle<()>,
}

impl Session {
    pub(crate) fn start(data_source: DataSource, owner: SessionOwner, services: Services) -> Self {
        let Services {
            secrets,
            events,
            history,
        } = services;
        let (disconnect, disconnect_signal) = oneshot::channel();
        let (requests, request_receiver) = mpsc::unbounded_channel();
        let (cancel, cancel_signal) = watch::channel(None);
        let context = SessionContext {
            data_source,
            owner,
            events,
            // Only what the user runs in a console belongs in the history, not the
            // explorer's catalog queries.
            history: matches!(owner, SessionOwner::Console(_)).then_some(history),
        };
        let signals = Signals {
            disconnect: disconnect_signal,
            cancel: cancel_signal,
        };
        let task = tokio::spawn(run(context, secrets, signals, request_receiver));
        Self {
            disconnect,
            requests,
            cancel,
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

    pub(crate) fn cancel(&self, run: RunId) {
        self.cancel.send_replace(Some(run));
    }

    /// Disconnects and waits until the connection is closed.
    pub(crate) async fn stop(self) {
        drop(self.disconnect);
        drop(self.requests);
        drop(self.cancel);
        if let Err(error) = self.task.await {
            tracing::error!("session task failed: {error}");
        }
    }
}

struct SessionContext {
    data_source: DataSource,
    owner: SessionOwner,
    events: EventSink,
    history: Option<HistoryLog>,
}

impl SessionContext {
    fn name(&self) -> &str {
        &self.data_source.name
    }

    fn send(&self, event: Event) {
        self.events.send(event);
    }

    fn record(&self, entry: HistoryEntry) {
        if let Some(history) = &self.history {
            history.record(entry);
        }
    }

    /// The dialect of the SQL this session runs, for reading statements' first words.
    fn dialect(&self) -> Dialect {
        Dialect::from(self.data_source.config.driver())
    }

    /// Tells a console about its connection's transaction. The explorer never opens one.
    fn notify_transaction(&self, status: TransactionStatus) {
        if let SessionOwner::Console(console) = self.owner {
            self.send(Event::TransactionChanged { console, status });
        }
    }

    fn notify(&self, state: SessionState) {
        self.send(match self.owner {
            SessionOwner::Explorer(id) => Event::SessionChanged { id, state },
            SessionOwner::Console(console) => Event::ConsoleSessionChanged { console, state },
        });
    }
}

/// What can interrupt a session's work from outside.
struct Signals {
    disconnect: oneshot::Receiver<()>,
    cancel: watch::Receiver<Option<RunId>>,
}

/// The session was told to disconnect while working; its loop must stop.
struct Disconnected;

impl Signals {
    /// Awaits `work` unless the session is told to disconnect first. If `run` is cancelled
    /// meanwhile, asks the database to stop it and keeps waiting: the work then fails
    /// promptly, and the connection is left in a known state instead of mid-result.
    async fn guard<T>(
        &mut self,
        work: impl Future<Output = T>,
        run: Option<RunId>,
        cancel_handle: &dyn CancelHandle,
    ) -> Result<T, Disconnected> {
        tokio::pin!(work);
        loop {
            tokio::select! {
                // Polling `work` first guarantees the statement has been started before a
                // cancel is acted on. Otherwise a cancel could reach the database before the
                // statement does, find nothing running, and be lost.
                biased;
                output = &mut work => return Ok(output),
                _ = &mut self.disconnect => return Err(Disconnected),
                changed = self.cancel.changed() => {
                    if changed.is_err() {
                        return Err(Disconnected);
                    }
                    if run.is_some() && *self.cancel.borrow_and_update() == run {
                        cancel_running_statement(cancel_handle).await;
                    }
                }
            }
        }
    }

    /// Whether `run` has been asked to stop.
    fn is_cancelled(&self, run: RunId) -> bool {
        *self.cancel.borrow() == Some(run)
    }
}

async fn cancel_running_statement(cancel_handle: &dyn CancelHandle) {
    if let Err(error) = cancel_handle.cancel().await {
        tracing::warn!(
            "could not cancel the running statement: {}",
            describe_error(&error)
        );
    }
}

async fn run(
    context: SessionContext,
    secrets: Arc<dyn SecretStore>,
    mut signals: Signals,
    mut requests: UnboundedReceiver<SessionRequest>,
) {
    let Some(mut connection) = connect(&context, &secrets, &mut signals.disconnect).await else {
        return;
    };
    let cancel_handle = connection.cancel_handle();
    let mut worker = Worker {
        context: &context,
        connection: connection.as_mut(),
        cancel_handle: cancel_handle.as_ref(),
        signals: &mut signals,
        parked: None,
        transaction: TransactionStatus::Idle,
    };
    worker.serve(&mut requests).await;
    if let Some(parked) = worker.parked.take() {
        query::discard(&context, parked, cancel_handle.as_ref()).await;
    }
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

/// A connected session's state while it serves requests.
struct Worker<'a> {
    context: &'a SessionContext,
    connection: &'a mut dyn Connection,
    cancel_handle: &'a dyn CancelHandle,
    signals: &'a mut Signals,
    /// The last run's final statement, with rows still unread.
    parked: Option<query::Parked>,
    /// The connection's transaction status as last reported to the UI.
    transaction: TransactionStatus,
}

impl Worker<'_> {
    /// Answers requests until told to disconnect.
    async fn serve(&mut self, requests: &mut UnboundedReceiver<SessionRequest>) {
        while let Some(request) = self.next_request(requests).await {
            if self.handle(request).await.is_err() {
                return;
            }
        }
    }

    /// The next request, meanwhile discarding parked rows whose run gets cancelled.
    async fn next_request(
        &mut self,
        requests: &mut UnboundedReceiver<SessionRequest>,
    ) -> Option<SessionRequest> {
        loop {
            tokio::select! {
                _ = &mut self.signals.disconnect => return None,
                changed = self.signals.cancel.changed() => {
                    changed.ok()?;
                    let cancelled = *self.signals.cancel.borrow_and_update();
                    if let Some(parked) = self.parked.take_if(|parked| Some(parked.run()) == cancelled) {
                        query::discard(self.context, parked, self.cancel_handle).await;
                    }
                }
                request = requests.recv() => return request,
            }
        }
    }

    async fn handle(&mut self, request: SessionRequest) -> Result<(), Disconnected> {
        if let SessionRequest::FetchMore(run) = request {
            if let Some(parked) = self.parked.take_if(|parked| parked.run() == run) {
                self.parked = query::fetch_more(self, parked).await?;
            }
            return Ok(());
        }
        // Validation is only a check, so it never costs the user their unread rows.
        if let SessionRequest::Validate(statements) = request {
            return validate::validate(self, statements).await;
        }
        // Anything else needs the connection, which parked rows are still holding.
        if let Some(parked) = self.parked.take() {
            query::discard(self.context, parked, self.cancel_handle).await;
        }
        match request {
            SessionRequest::Children(path) => {
                let work = catalog::children(self.context, &mut *self.connection, path);
                self.signals.guard(work, None, self.cancel_handle).await
            }
            SessionRequest::Definition(path) => {
                let work = catalog::definition(self.context, &mut *self.connection, path);
                self.signals.guard(work, None, self.cancel_handle).await
            }
            SessionRequest::Execute(script) => {
                self.parked = query::run_script(self, &script).await?;
                Ok(())
            }
            SessionRequest::EndTransaction(end) => transaction::end(self, end).await,
            SessionRequest::DescribeTable(table) => {
                let work = catalog::describe_table(self.context, &mut *self.connection, table);
                self.signals.guard(work, None, self.cancel_handle).await
            }
            SessionRequest::ApplyEdits(statements) => edits::apply(self, &statements).await,
            SessionRequest::FetchMore(_) | SessionRequest::Validate(_) => Ok(()),
        }
    }
}
