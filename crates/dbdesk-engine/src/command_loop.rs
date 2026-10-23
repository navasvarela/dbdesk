use std::collections::HashMap;
use std::sync::Arc;

use dbdesk_core::{DataSource, DataSourceId, describe_error};
use dbdesk_store::{ConsoleFiles, KeymapFile, SecretError, SecretStore};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::{JoinError, JoinSet};

use crate::console_files;
use crate::events::EventSink;
use crate::export;
use crate::history::HistoryLog;
use crate::keymap;
use crate::open::test_connection;
use crate::registry::{Registry, RegistryError};
use crate::session::{Script, Services, Session, SessionOwner, SessionRequest};
use crate::{Command, ConsoleId, EngineConfig, Event, PasswordUpdate, RunOutcome};

#[derive(Debug, thiserror::Error)]
enum SaveError {
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error("could not store the password")]
    Password(#[source] SecretError),
    #[error("a background task failed")]
    Task(#[source] JoinError),
}

/// Handles commands until the UI closes the command channel, then disconnects everything.
pub(crate) async fn run(
    config: EngineConfig,
    mut commands: UnboundedReceiver<Command>,
    events: EventSink,
) {
    tracing::info!("engine started");
    events.send(Event::Started);
    events.send(keymap::load(&config.keymap).await);
    let registry = Registry::load(config.data_sources).await;
    events.send(registry.loaded_event());
    events.send(console_files::load(&config.consoles).await);
    let (history, history_task) = HistoryLog::start(config.history, events.clone());

    let mut engine = Engine {
        history,
        registry,
        secrets: config.secrets,
        console_files: config.consoles,
        keymap_file: config.keymap,
        events,
        sessions: HashMap::new(),
        consoles: HashMap::new(),
        closing: JoinSet::new(),
    };
    while let Some(command) = commands.recv().await {
        engine.handle(command).await;
    }
    // Sessions hold history handles too, so the history thread ends only once they have
    // stopped and their last statements are recorded.
    engine.shut_down().await;
    if let Err(error) = history_task.await {
        tracing::error!("the history thread failed: {error}");
    }
    tracing::info!("engine stopped");
}

struct ConsoleSession {
    data_source: DataSourceId,
    session: Session,
}

struct Engine {
    registry: Registry,
    history: HistoryLog,
    secrets: Arc<dyn SecretStore>,
    console_files: ConsoleFiles,
    keymap_file: KeymapFile,
    events: EventSink,
    /// The explorer's sessions, one per data source.
    sessions: HashMap<DataSourceId, Session>,
    /// Each query console's own session.
    consoles: HashMap<ConsoleId, ConsoleSession>,
    /// Sessions that were asked to disconnect and are still closing their connection.
    closing: JoinSet<()>,
}

impl Engine {
    /// Saves and deletes (of data sources and console files) are awaited here, so they apply
    /// in the order the UI sent them.
    /// Anything that waits on a network (connecting, testing) runs in its own task.
    async fn handle(&mut self, command: Command) {
        match command {
            Command::SaveDataSource {
                data_source,
                password,
            } => self.save(data_source, password).await,
            Command::DeleteDataSource(id) => self.delete(id).await,
            Command::TestConnection {
                data_source,
                password,
            } => {
                tokio::spawn(test_connection(
                    data_source,
                    password,
                    Arc::clone(&self.secrets),
                    self.events.clone(),
                ));
            }
            Command::Connect(id) => self.connect(id),
            Command::Disconnect(id) => self.disconnect(id),
            Command::LoadChildren { id, path } => {
                self.request(id, SessionRequest::Children(path));
            }
            Command::LoadDefinition { id, path } => {
                self.request(id, SessionRequest::Definition(path));
            }
            Command::Execute {
                console,
                data_source,
                run,
                statements,
                transactions,
            } => self.execute(
                console,
                data_source,
                Script {
                    run,
                    statements,
                    transactions,
                },
            ),
            Command::EndTransaction { console, end } => {
                if let Some(console) = self.consoles.get(&console) {
                    console.session.request(SessionRequest::EndTransaction(end));
                }
            }
            Command::FetchMore { console, run } => {
                if let Some(console) = self.consoles.get(&console) {
                    console.session.request(SessionRequest::FetchMore(run));
                }
            }
            Command::Cancel { console, run } => {
                if let Some(console) = self.consoles.get(&console) {
                    console.session.cancel(run);
                }
            }
            Command::CloseConsole(console) => self.close_console(console),
            Command::SaveConsole { key, text } => {
                if let Some(failed) = console_files::save(&self.console_files, key, text).await {
                    self.events.send(failed);
                }
            }
            Command::DeleteConsoleFile(key) => {
                console_files::delete(&self.console_files, key).await;
            }
            Command::ReloadKeymap => self.events.send(keymap::load(&self.keymap_file).await),
            Command::CreateKeymap { text } => {
                self.events
                    .send(keymap::create(&self.keymap_file, text).await);
            }
            Command::ExportRows { console, export } => {
                tokio::spawn(export::run(console, export, self.events.clone()));
            }
            Command::SearchHistory { data_source, text } => {
                self.history.search(data_source, text);
            }
        }
    }

    async fn save(&mut self, data_source: DataSource, password: PasswordUpdate) {
        let id = data_source.id;
        match self.try_save(data_source.clone(), password).await {
            Ok(()) => {
                tracing::info!(data_source = %data_source.name, "saved data source");
                self.events.send(Event::DataSourceSaved(data_source));
            }
            Err(error) => {
                let error = describe_error(&error);
                tracing::error!(data_source = %data_source.name, "could not save data source: {error}");
                self.events.send(Event::DataSourceSaveFailed { id, error });
            }
        }
    }

    async fn try_save(
        &mut self,
        data_source: DataSource,
        password: PasswordUpdate,
    ) -> Result<(), SaveError> {
        self.registry.ensure_available()?;
        if let PasswordUpdate::Set(password) = password {
            let secrets = Arc::clone(&self.secrets);
            let id = data_source.id;
            tokio::task::spawn_blocking(move || secrets.set_password(id, &password))
                .await
                .map_err(SaveError::Task)?
                .map_err(SaveError::Password)?;
        }
        self.registry.upsert(data_source).await?;
        Ok(())
    }

    async fn delete(&mut self, id: DataSourceId) {
        self.disconnect(id);
        match self.registry.remove(id).await {
            Ok(true) => {
                self.delete_password(id).await;
                tracing::info!(%id, "deleted data source");
                self.events.send(Event::DataSourceDeleted(id));
            }
            Ok(false) => tracing::warn!(%id, "delete requested for an unknown data source"),
            Err(error) => {
                tracing::error!(%id, "could not delete data source: {}", describe_error(&error));
            }
        }
    }

    /// A leftover keychain entry is harmless, so failing to remove it is only logged.
    async fn delete_password(&self, id: DataSourceId) {
        let secrets = Arc::clone(&self.secrets);
        let result = tokio::task::spawn_blocking(move || secrets.delete_password(id)).await;
        let error = match result {
            Ok(Ok(())) => return,
            Ok(Err(error)) => describe_error(&error),
            Err(error) => describe_error(&error),
        };
        tracing::warn!(%id, "could not remove the saved password: {error}");
    }

    fn connect(&mut self, id: DataSourceId) {
        if self
            .sessions
            .get(&id)
            .is_some_and(|session| !session.is_finished())
        {
            tracing::debug!(%id, "already connected or connecting");
            return;
        }
        let Some(data_source) = self.registry.get(id) else {
            tracing::warn!(%id, "connect requested for an unknown data source");
            return;
        };
        let session = Session::start(
            data_source.clone(),
            SessionOwner::Explorer(id),
            self.services(),
        );
        self.sessions.insert(id, session);
    }

    /// Queues a run on the console's session, (re)connecting it if it isn't connected to
    /// `data_source`.
    fn execute(&mut self, console: ConsoleId, data_source: DataSourceId, script: Script) {
        let reusable = self.consoles.get(&console).is_some_and(|existing| {
            existing.data_source == data_source && !existing.session.is_finished()
        });
        if !reusable {
            self.close_console(console);
            let Some(config) = self.registry.get(data_source) else {
                tracing::warn!(%data_source, "run requested for an unknown data source");
                self.events.send(Event::RunFinished {
                    run: script.run,
                    outcome: RunOutcome::Failed,
                    elapsed: std::time::Duration::ZERO,
                });
                return;
            };
            let session = Session::start(
                config.clone(),
                SessionOwner::Console(console),
                self.services(),
            );
            self.consoles.insert(
                console,
                ConsoleSession {
                    data_source,
                    session,
                },
            );
        }
        if let Some(console) = self.consoles.get(&console) {
            console.session.request(SessionRequest::Execute(script));
        }
    }

    fn services(&self) -> Services {
        Services {
            secrets: Arc::clone(&self.secrets),
            events: self.events.clone(),
            history: self.history.clone(),
        }
    }

    fn close_console(&mut self, console: ConsoleId) {
        if let Some(console) = self.consoles.remove(&console) {
            self.closing.spawn(console.session.stop());
        }
    }

    /// Queues work on the data source's session, starting one if it isn't connected, so
    /// expanding a disconnected data source in the explorer connects it.
    fn request(&mut self, id: DataSourceId, request: SessionRequest) {
        self.connect(id);
        if let Some(session) = self.sessions.get(&id) {
            session.request(request);
        }
    }

    fn disconnect(&mut self, id: DataSourceId) {
        if let Some(session) = self.sessions.remove(&id) {
            self.closing.spawn(session.stop());
        }
        let consoles: Vec<ConsoleId> = self
            .consoles
            .iter()
            .filter(|(_, console)| console.data_source == id)
            .map(|(console, _)| *console)
            .collect();
        for console in consoles {
            self.close_console(console);
        }
    }

    async fn shut_down(mut self) {
        drop(self.history);
        for (_, session) in self.sessions.drain() {
            self.closing.spawn(session.stop());
        }
        for (_, console) in self.consoles.drain() {
            self.closing.spawn(console.session.stop());
        }
        while let Some(result) = self.closing.join_next().await {
            if let Err(error) = result {
                tracing::error!("session shutdown failed: {error}");
            }
        }
    }
}
