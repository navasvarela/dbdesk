use std::collections::HashMap;
use std::sync::Arc;

use dbdesk_core::{DataSource, DataSourceId, describe_error};
use dbdesk_store::{SecretError, SecretStore};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::{JoinError, JoinSet};

use crate::events::EventSink;
use crate::open::test_connection;
use crate::registry::{Registry, RegistryError};
use crate::session::Session;
use crate::{Command, EngineConfig, Event, PasswordUpdate};

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
    let registry = Registry::load(config.data_sources).await;
    events.send(registry.loaded_event());

    let mut engine = Engine {
        registry,
        secrets: config.secrets,
        events,
        sessions: HashMap::new(),
        closing: JoinSet::new(),
    };
    while let Some(command) = commands.recv().await {
        engine.handle(command).await;
    }
    engine.shut_down().await;
    tracing::info!("engine stopped");
}

struct Engine {
    registry: Registry,
    secrets: Arc<dyn SecretStore>,
    events: EventSink,
    sessions: HashMap<DataSourceId, Session>,
    /// Sessions that were asked to disconnect and are still closing their connection.
    closing: JoinSet<()>,
}

impl Engine {
    /// Saves and deletes are awaited here, so they apply in the order the UI sent them.
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
            Arc::clone(&self.secrets),
            self.events.clone(),
        );
        self.sessions.insert(id, session);
    }

    fn disconnect(&mut self, id: DataSourceId) {
        if let Some(session) = self.sessions.remove(&id) {
            self.closing.spawn(session.stop());
        }
    }

    async fn shut_down(mut self) {
        for (_, session) in self.sessions.drain() {
            self.closing.spawn(session.stop());
        }
        while let Some(result) = self.closing.join_next().await {
            if let Err(error) = result {
                tracing::error!("session shutdown failed: {error}");
            }
        }
    }
}
