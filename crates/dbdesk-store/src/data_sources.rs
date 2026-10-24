use std::path::{Path, PathBuf};
use std::{fs, io};

use dbdesk_core::DataSource;
use serde::{Deserialize, Serialize};

use crate::atomic::write_atomically;

const HEADER: &str = "\
# dbdesk data sources. Safe to edit by hand while dbdesk is closed.
# Passwords are stored in the OS keychain, never in this file.
# Comments are not preserved when dbdesk saves this file.

";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("could not read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{} is not a valid data sources file", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("could not serialize the data sources")]
    Serialize(#[source] toml::ser::Error),
    #[error("could not write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// The `data_sources.toml` file: one `[[data_source]]` table per data source.
#[derive(Debug, Clone)]
pub struct DataSourceFile {
    path: PathBuf,
}

#[derive(Deserialize)]
struct FileContents {
    #[serde(default, rename = "data_source")]
    data_sources: Vec<DataSource>,
}

#[derive(Serialize)]
struct FileContentsRef<'a> {
    #[serde(rename = "data_source")]
    data_sources: &'a [DataSource],
}

impl DataSourceFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads all data sources. A missing file means none have been saved yet.
    ///
    /// # Errors
    ///
    /// [`StoreError::Read`] or [`StoreError::Parse`] if the file exists but can't be used.
    pub fn load(&self) -> Result<Vec<DataSource>, StoreError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(StoreError::Read {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        let contents: FileContents = toml::from_str(&text).map_err(|source| StoreError::Parse {
            path: self.path.clone(),
            source,
        })?;
        Ok(contents.data_sources)
    }

    /// Replaces the file's contents with `data_sources`.
    ///
    /// Written atomically: a crash mid-write leaves either the old or the new file.
    ///
    /// # Errors
    ///
    /// [`StoreError::Write`] if the directory or file can't be written.
    pub fn save(&self, data_sources: &[DataSource]) -> Result<(), StoreError> {
        let body =
            toml::to_string(&FileContentsRef { data_sources }).map_err(StoreError::Serialize)?;
        write_atomically(&self.path, &format!("{HEADER}{body}")).map_err(|source| {
            StoreError::Write {
                path: self.path.clone(),
                source,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use dbdesk_core::{ConnectionConfig, DataSourceId, SqliteConfig};

    use super::*;

    fn sqlite(name: &str) -> DataSource {
        DataSource {
            id: DataSourceId::generate(),
            name: name.to_owned(),
            color: None,
            project: None,
            config: ConnectionConfig::Sqlite(SqliteConfig {
                path: format!("/data/{name}.db").into(),
                read_only: false,
            }),
        }
    }

    #[test]
    fn missing_file_has_no_data_sources() {
        let dir = tempfile::tempdir().unwrap();

        let loaded = DataSourceFile::new(dir.path().join("data_sources.toml"))
            .load()
            .unwrap();

        assert!(loaded.is_empty());
    }

    #[test]
    fn saves_and_loads_creating_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = DataSourceFile::new(dir.path().join("nested/data_sources.toml"));
        let data_sources = vec![sqlite("one"), sqlite("two")];

        file.save(&data_sources).unwrap();

        assert_eq!(file.load().unwrap(), data_sources);
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.starts_with("# dbdesk data sources."), "{text}");
        assert!(!file.path().with_extension("toml.tmp").exists());
    }

    #[test]
    fn invalid_file_is_a_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = DataSourceFile::new(dir.path().join("data_sources.toml"));
        fs::write(file.path(), "[[data_source]]\nname = 3").unwrap();

        assert!(matches!(file.load(), Err(StoreError::Parse { .. })));
    }
}
