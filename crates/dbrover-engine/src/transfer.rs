//! Exporting data sources to a file and importing one: File › Export data sources… and
//! File › Import data sources…. The file format is the store's (`dbrover_store::portable`).

use std::path::PathBuf;

use dbrover_core::{DataSource, DataSourceId, ProjectId, Variables, describe_error};
use dbrover_store::{ImportPreview, Project, Projects, read_export, write_export};

use crate::Event;
use crate::environment::ProcessEnvironment;
use crate::events::EventSink;

/// What to write, gathered from the registries before the file is written off the loop.
pub(crate) struct DataSourceExport {
    pub(crate) path: PathBuf,
    pub(crate) data_sources: Vec<DataSource>,
    pub(crate) projects: Vec<Project>,
}

impl DataSourceExport {
    /// The chosen data sources, in the order the file has them, and the chosen projects
    /// together with every project a chosen data source is in, so it lands in its project on
    /// the other machine.
    pub(crate) fn gather(
        path: PathBuf,
        all_data_sources: &[DataSource],
        projects: &Projects,
        chosen_data_sources: &[DataSourceId],
        chosen_projects: &[ProjectId],
    ) -> Self {
        let data_sources: Vec<DataSource> = all_data_sources
            .iter()
            .filter(|ds| chosen_data_sources.contains(&ds.id))
            .cloned()
            .collect();
        let projects = projects
            .projects
            .iter()
            .filter(|project| {
                chosen_projects.contains(&project.id)
                    || data_sources.iter().any(|ds| ds.project == Some(project.id))
            })
            .cloned()
            .collect();
        Self {
            path,
            data_sources,
            projects,
        }
    }
}

/// Writes the file, then reports [`Event::DataSourcesExported`].
pub(crate) async fn export(export: DataSourceExport, events: EventSink) {
    let DataSourceExport {
        path,
        data_sources,
        projects,
    } = export;
    let count = data_sources.len();
    let target = path.clone();
    let written = tokio::task::spawn_blocking(move || {
        let home = ProcessEnvironment.home();
        write_export(&target, &data_sources, &projects, home.as_deref())
    })
    .await;
    let result = match written {
        Ok(Ok(())) => Ok(count),
        Ok(Err(error)) => Err(describe_error(&error)),
        Err(error) => Err(describe_error(&error)),
    };
    match &result {
        Ok(count) => tracing::info!("exported {count} data sources to {}", path.display()),
        Err(error) => tracing::warn!("could not export data sources: {error}"),
    }
    events.send(Event::DataSourcesExported { path, result });
}

/// Reads the file and compares it with this machine's data sources and projects, then
/// reports [`Event::ImportPreviewed`]. Changes nothing.
pub(crate) async fn preview(
    path: PathBuf,
    data_sources: Vec<DataSource>,
    projects: Projects,
    events: EventSink,
) {
    let file = path.clone();
    let read = tokio::task::spawn_blocking(move || {
        let incoming = read_export(&file, ProcessEnvironment.home().as_deref())?;
        Ok::<_, dbrover_store::PortableError>(ImportPreview::new(
            incoming,
            &data_sources,
            &projects,
            &ProcessEnvironment,
        ))
    })
    .await;
    let result = match read {
        Ok(Ok(preview)) => Ok(preview),
        Ok(Err(error)) => Err(describe_error(&error)),
        Err(error) => Err(describe_error(&error)),
    };
    if let Err(error) = &result {
        tracing::warn!("could not read {} to import it: {error}", path.display());
    }
    events.send(Event::ImportPreviewed {
        path,
        result: result.map(Box::new),
    });
}
