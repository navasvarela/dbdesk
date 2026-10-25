use dbdesk_core::{ProjectId, describe_error};
use dbdesk_store::{McpOverrides, Project, ProjectFile, ProjectStoreError, Projects};
use tokio::task::JoinError;

use crate::Event;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProjectError {
    #[error("projects can't be changed until projects.toml is fixed: {0}")]
    Unavailable(String),
    #[error("there is no such project")]
    Unknown,
    #[error("a project's name can't be empty")]
    EmptyName,
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Store(#[from] ProjectStoreError),
    #[error("a background task failed")]
    Task(#[source] JoinError),
}

/// The projects the engine knows about, kept in sync with `projects.toml`. The same shape as
/// the data sources' `Registry`, for the same reason: a file that can't be read is never
/// written over.
pub(crate) enum ProjectRegistry {
    Loaded {
        file: ProjectFile,
        projects: Projects,
    },
    Unavailable {
        reason: String,
    },
}

impl ProjectRegistry {
    pub(crate) async fn load(file: ProjectFile) -> Self {
        let loaded = {
            let file = file.clone();
            tokio::task::spawn_blocking(move || file.load()).await
        };
        let reason = match loaded {
            Ok(Ok(projects)) => return Self::Loaded { file, projects },
            Ok(Err(error)) => describe_error(&error),
            Err(join_error) => describe_error(&join_error),
        };
        tracing::error!("could not load projects: {reason}");
        Self::Unavailable { reason }
    }

    pub(crate) fn loaded_event(&self) -> Event {
        match self {
            Self::Loaded { projects, .. } => Event::ProjectsLoaded(projects.clone()),
            Self::Unavailable { reason } => Event::ProjectsUnavailable {
                error: reason.clone(),
            },
        }
    }

    pub(crate) fn contains(&self, id: ProjectId) -> bool {
        match self {
            Self::Loaded { projects, .. } => projects.get(id).is_some(),
            Self::Unavailable { .. } => false,
        }
    }

    /// Adds a project called `name`, with a slug no other project has, and saves the file.
    pub(crate) async fn create(
        &mut self,
        id: ProjectId,
        name: &str,
    ) -> Result<Project, ProjectError> {
        let name = non_empty(name)?;
        self.update(|projects| {
            let project = Project::new(id, name.to_owned(), projects.unused_slug(name));
            projects.projects.push(project.clone());
            Ok(project)
        })
        .await
    }

    /// Renames the project and saves the file. The slug stays, so MCP clients configured with
    /// it keep working.
    pub(crate) async fn rename(
        &mut self,
        id: ProjectId,
        name: &str,
    ) -> Result<Project, ProjectError> {
        let name = non_empty(name)?;
        self.update(|projects| {
            let project = projects
                .projects
                .iter_mut()
                .find(|project| project.id == id)
                .ok_or(ProjectError::Unknown)?;
            name.clone_into(&mut project.name);
            Ok(project.clone())
        })
        .await
    }

    /// What `projects.toml` holds, or why it can't be used.
    pub(crate) fn projects(&self) -> Result<&Projects, ProjectError> {
        match self {
            Self::Loaded { projects, .. } => Ok(projects),
            Self::Unavailable { reason } => Err(ProjectError::Unavailable(reason.clone())),
        }
    }

    /// Adds each project, or replaces the one with its id, and saves the file once.
    pub(crate) async fn import(&mut self, imported: Vec<Project>) -> Result<(), ProjectError> {
        self.update(|projects| {
            for project in imported {
                match projects.projects.iter_mut().find(|p| p.id == project.id) {
                    Some(existing) => *existing = project,
                    None => projects.projects.push(project),
                }
            }
            projects.check_unique().map_err(ProjectError::Conflict)
        })
        .await
    }

    /// Replaces the project's MCP settings and saves the file.
    pub(crate) async fn save_mcp(
        &mut self,
        id: ProjectId,
        mcp_enabled: bool,
        overrides: McpOverrides,
    ) -> Result<Project, ProjectError> {
        self.update(|projects| {
            let project = projects
                .projects
                .iter_mut()
                .find(|project| project.id == id)
                .ok_or(ProjectError::Unknown)?;
            project.mcp_enabled = mcp_enabled;
            project.mcp = overrides;
            Ok(project.clone())
        })
        .await
    }

    /// Removes the project and saves the file.
    pub(crate) async fn remove(&mut self, id: ProjectId) -> Result<(), ProjectError> {
        self.update(|projects| {
            let before = projects.projects.len();
            projects.projects.retain(|project| project.id != id);
            if projects.projects.len() == before {
                return Err(ProjectError::Unknown);
            }
            Ok(())
        })
        .await
    }

    /// Applies `change` to a copy, saves the copy, and only then keeps it, so memory and file
    /// never disagree after a failed write.
    async fn update<T>(
        &mut self,
        change: impl FnOnce(&mut Projects) -> Result<T, ProjectError>,
    ) -> Result<T, ProjectError> {
        let (file, projects) = match self {
            Self::Loaded { file, projects } => (file, projects),
            Self::Unavailable { reason } => return Err(ProjectError::Unavailable(reason.clone())),
        };
        let mut updated = projects.clone();
        let result = change(&mut updated)?;

        let file_to_write = file.clone();
        let to_write = updated.clone();
        tokio::task::spawn_blocking(move || file_to_write.save(&to_write))
            .await
            .map_err(ProjectError::Task)??;
        *projects = updated;
        Ok(result)
    }
}

fn non_empty(name: &str) -> Result<&str, ProjectError> {
    match name.trim() {
        "" => Err(ProjectError::EmptyName),
        name => Ok(name),
    }
}
