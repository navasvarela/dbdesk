use dbrover_core::{DataSource, DataSourceId, ProjectId, describe_error};
use dbrover_store::{DataSourceFile, StoreError};
use tokio::task::JoinError;

use crate::Event;

#[derive(Debug, thiserror::Error)]
pub(crate) enum RegistryError {
    #[error("data sources can't be changed until data_sources.toml is fixed: {0}")]
    Unavailable(String),
    #[error("there is no such data source")]
    Unknown,
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("a background task failed")]
    Task(#[source] JoinError),
}

/// The data sources the engine knows about, kept in sync with `data_sources.toml`.
pub(crate) enum Registry {
    Loaded {
        file: DataSourceFile,
        data_sources: Vec<DataSource>,
    },
    /// The file exists but couldn't be read. Every change is refused, so the file is never
    /// overwritten with an empty list and the user's data sources stay recoverable.
    Unavailable { reason: String },
}

impl Registry {
    pub(crate) async fn load(file: DataSourceFile) -> Self {
        let loaded = {
            let file = file.clone();
            tokio::task::spawn_blocking(move || file.load()).await
        };
        let reason = match loaded {
            Ok(Ok(data_sources)) => return Self::Loaded { file, data_sources },
            Ok(Err(error)) => describe_error(&error),
            Err(join_error) => describe_error(&join_error),
        };
        tracing::error!("could not load data sources: {reason}");
        Self::Unavailable { reason }
    }

    /// The event that tells the UI what was loaded.
    pub(crate) fn loaded_event(&self) -> Event {
        match self {
            Self::Loaded { data_sources, .. } => Event::DataSourcesLoaded(data_sources.clone()),
            Self::Unavailable { reason } => Event::DataSourcesUnavailable {
                error: reason.clone(),
            },
        }
    }

    pub(crate) fn get(&self, id: DataSourceId) -> Option<&DataSource> {
        match self {
            Self::Loaded { data_sources, .. } => data_sources.iter().find(|ds| ds.id == id),
            Self::Unavailable { .. } => None,
        }
    }

    /// Every data source, or why `data_sources.toml` can't be used.
    pub(crate) fn data_sources(&self) -> Result<&[DataSource], RegistryError> {
        match self {
            Self::Loaded { data_sources, .. } => Ok(data_sources),
            Self::Unavailable { reason } => Err(RegistryError::Unavailable(reason.clone())),
        }
    }

    pub(crate) fn ensure_available(&self) -> Result<(), RegistryError> {
        match self {
            Self::Loaded { .. } => Ok(()),
            Self::Unavailable { reason } => Err(RegistryError::Unavailable(reason.clone())),
        }
    }

    /// Adds the data source, or replaces the one with the same id, and saves the file.
    pub(crate) async fn upsert(&mut self, data_source: DataSource) -> Result<(), RegistryError> {
        self.update(|data_sources| {
            match data_sources.iter_mut().find(|ds| ds.id == data_source.id) {
                Some(existing) => *existing = data_source,
                None => data_sources.push(data_source),
            }
        })
        .await
    }

    /// Adds each data source, or replaces the one with its id, and saves the file once.
    pub(crate) async fn upsert_all(
        &mut self,
        imported: Vec<DataSource>,
    ) -> Result<(), RegistryError> {
        self.update(|data_sources| {
            for data_source in imported {
                match data_sources.iter_mut().find(|ds| ds.id == data_source.id) {
                    Some(existing) => *existing = data_source,
                    None => data_sources.push(data_source),
                }
            }
        })
        .await
    }

    /// Removes the data source and saves the file. Returns `false` if it wasn't there.
    pub(crate) async fn remove(&mut self, id: DataSourceId) -> Result<bool, RegistryError> {
        let mut removed = false;
        self.update(|data_sources| {
            let before = data_sources.len();
            data_sources.retain(|ds| ds.id != id);
            removed = data_sources.len() != before;
        })
        .await?;
        Ok(removed)
    }

    /// Puts the data source in `project` (`None` for no project) and saves the file.
    pub(crate) async fn move_to_project(
        &mut self,
        id: DataSourceId,
        project: Option<ProjectId>,
    ) -> Result<DataSource, RegistryError> {
        let mut moved = None;
        self.update(|data_sources| {
            if let Some(data_source) = data_sources.iter_mut().find(|ds| ds.id == id) {
                data_source.project = project;
                moved = Some(data_source.clone());
            }
        })
        .await?;
        moved.ok_or(RegistryError::Unknown)
    }

    /// Moves every data source in `project` to no project, saves the file, and returns the
    /// data sources it moved.
    pub(crate) async fn leave_project(
        &mut self,
        project: ProjectId,
    ) -> Result<Vec<DataSource>, RegistryError> {
        let mut moved = Vec::new();
        self.update(|data_sources| {
            for data_source in data_sources
                .iter_mut()
                .filter(|ds| ds.project == Some(project))
            {
                data_source.project = None;
                moved.push(data_source.clone());
            }
        })
        .await?;
        Ok(moved)
    }

    /// Applies `change` to a copy, saves the copy, and only then keeps it, so memory and
    /// file never disagree after a failed write.
    async fn update(
        &mut self,
        change: impl FnOnce(&mut Vec<DataSource>),
    ) -> Result<(), RegistryError> {
        let Self::Loaded { file, data_sources } = self else {
            self.ensure_available()?;
            return Ok(());
        };
        let mut updated = data_sources.clone();
        change(&mut updated);

        let file_to_write = file.clone();
        let to_write = updated.clone();
        tokio::task::spawn_blocking(move || file_to_write.save(&to_write))
            .await
            .map_err(RegistryError::Task)??;
        *data_sources = updated;
        Ok(())
    }
}
