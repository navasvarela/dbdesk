//! Exporting data sources and their projects to one file, to recreate them on another
//! machine or share them with a team, and importing such a file. See
//! `docs/well-rounded.md` (W1) and `docs/persistence.md`.
//!
//! An export never holds a password: those stay in the keychain, and an imported data source
//! asks for its own on first connect.

mod file;
mod paths;
mod preview;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::{fs, io};

use dbrover_core::DataSource;

use crate::atomic::write_atomically;
use crate::projects::{Project, Projects};
use file::{FORMAT_VERSION, PortableDataSource, PortableFile, PortableProject};
use paths::{ExportPaths, ImportPaths};

pub use preview::{
    DataSourceImport, Difference, EntryStatus, ImportChoice, ImportChoices, ImportPlan,
    ImportPreview, MissingPath, ProjectImport,
};

const HEADER: &str = "\
# DbRover data sources, exported to recreate them on another machine:
# File › Import data sources… in dbrover. No passwords: each data source asks for its own
# on first connect. Paths starting with ~ are in the home folder, and relative ones are
# next to this file.

";

#[derive(Debug, thiserror::Error)]
pub enum PortableError {
    #[error("could not read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{} is not a DbRover data sources export", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error(
        "{} was written by a newer DbRover (format version {found}, this one reads up to \
         {FORMAT_VERSION}); update DbRover to import it",
        path.display()
    )]
    NewerVersion { path: PathBuf, found: u32 },
    #[error("{} is not a valid export: {reason}", path.display())]
    Invalid { path: PathBuf, reason: String },
    #[error("could not serialize the export")]
    Serialize(#[source] toml::ser::Error),
    #[error("could not write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// What an imported file holds, with its paths resolved for this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub file: PathBuf,
    pub projects: Vec<Project>,
    pub data_sources: Vec<DataSource>,
}

/// Writes `data_sources` and `projects` to `path`, atomically.
///
/// `home` is the home folder paths under which are written as `~/…`.
///
/// # Errors
///
/// [`PortableError::Write`] if the file can't be written.
pub fn write_export(
    path: &Path,
    data_sources: &[DataSource],
    projects: &[Project],
    home: Option<&Path>,
) -> Result<(), PortableError> {
    let text = export_text(path, data_sources, projects, home)?;
    write_atomically(path, &text).map_err(|source| PortableError::Write {
        path: path.to_owned(),
        source,
    })
}

fn export_text(
    path: &Path,
    data_sources: &[DataSource],
    projects: &[Project],
    home: Option<&Path>,
) -> Result<String, PortableError> {
    let paths = ExportPaths {
        dir: path.parent().map(Path::to_path_buf).unwrap_or_default(),
        home: home.map(Path::to_path_buf),
    };
    let file = PortableFile {
        format_version: FORMAT_VERSION,
        projects: projects.iter().map(PortableProject::from).collect(),
        data_sources: data_sources
            .iter()
            .map(|data_source| PortableDataSource::export(data_source, |p| paths.portable(p)))
            .collect(),
    };
    let body = toml::to_string(&file).map_err(PortableError::Serialize)?;
    Ok(format!("{HEADER}{body}"))
}

/// Reads an exported file, resolving its paths for this machine: `~/…` under `home`, and
/// relative paths next to the file.
///
/// # Errors
///
/// [`PortableError`] if the file can't be read, isn't an export, was written by a newer
/// DbRover, or names an id or slug twice.
pub fn read_export(path: &Path, home: Option<&Path>) -> Result<Incoming, PortableError> {
    let text = fs::read_to_string(path).map_err(|source| PortableError::Read {
        path: path.to_owned(),
        source,
    })?;
    parse(path, &text, home)
}

fn parse(path: &Path, text: &str, home: Option<&Path>) -> Result<Incoming, PortableError> {
    let file: PortableFile = toml::from_str(text).map_err(|source| PortableError::Parse {
        path: path.to_owned(),
        source,
    })?;
    if file.format_version > FORMAT_VERSION {
        return Err(PortableError::NewerVersion {
            path: path.to_owned(),
            found: file.format_version,
        });
    }
    let invalid = |reason| PortableError::Invalid {
        path: path.to_owned(),
        reason,
    };
    let mut ids = HashSet::new();
    if let Some(twice) = file.data_sources.iter().find(|ds| !ids.insert(ds.id)) {
        return Err(invalid(format!(
            "two data sources have the id {}",
            twice.id
        )));
    }
    let projects = Projects {
        projects: file.projects.into_iter().map(Project::from).collect(),
        ..Projects::default()
    };
    projects.check_unique().map_err(invalid)?;
    let paths = ImportPaths {
        dir: path.parent().map(Path::to_path_buf).unwrap_or_default(),
        home: home.map(Path::to_path_buf),
    };
    Ok(Incoming {
        file: path.to_owned(),
        projects: projects.projects,
        data_sources: file
            .data_sources
            .into_iter()
            .map(|data_source| data_source.import(|p| paths.local(p)))
            .collect(),
    })
}

#[cfg(test)]
mod tests;
