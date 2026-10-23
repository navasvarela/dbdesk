//! Query console files: `consoles/<data-source-id>/<name>.sql`, plain SQL that any editor
//! can open.

use std::fmt;
use std::path::{Path, PathBuf};
use std::{fs, io};

use dbdesk_core::DataSourceId;
use serde::{Deserialize, Serialize};

use crate::atomic::write_atomically;

const EXTENSION: &str = "sql";

/// A console file's name, without `.sql`. Always a plain file name, so joining it to the
/// consoles folder can never point outside it, whatever a saved layout says.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ConsoleName(String);

#[derive(Debug, thiserror::Error)]
#[error("{0:?} can't be a console file name")]
pub struct InvalidConsoleName(String);

impl ConsoleName {
    /// The name the `n`th console of a data source gets: `console`, `console_2`, … as
    /// DataGrip names them.
    pub fn numbered(n: usize) -> Self {
        match n {
            0 | 1 => Self("console".to_owned()),
            n => Self(format!("console_{n}")),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The `n` of a [`ConsoleName::numbered`] name, if this is one.
    pub fn number(&self) -> Option<usize> {
        match self.0.strip_prefix("console") {
            Some("") => Some(1),
            Some(suffix) => suffix.strip_prefix('_')?.parse().ok().filter(|n| *n > 1),
            None => None,
        }
    }
}

impl TryFrom<String> for ConsoleName {
    type Error = InvalidConsoleName;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        let plain = !name.is_empty()
            && !name.starts_with('.')
            && !name.contains(['/', '\\', '\0'])
            && Path::new(&name).file_name() == Some(name.as_ref());
        if plain {
            Ok(Self(name))
        } else {
            Err(InvalidConsoleName(name))
        }
    }
}

impl From<ConsoleName> for String {
    fn from(name: ConsoleName) -> Self {
        name.0
    }
}

impl fmt::Display for ConsoleName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which console file: unique per data source and name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConsoleKey {
    pub data_source: DataSourceId,
    pub name: ConsoleName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedConsole {
    pub key: ConsoleKey,
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConsoleStoreError {
    #[error("could not read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// The `consoles/` folder.
#[derive(Debug, Clone)]
pub struct ConsoleFiles {
    root: PathBuf,
}

impl ConsoleFiles {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn path(&self, key: &ConsoleKey) -> PathBuf {
        self.root
            .join(key.data_source.to_string())
            .join(format!("{}.{EXTENSION}", key.name))
    }

    /// Reads every console file. A missing folder means none were saved yet. Folders that
    /// aren't a data source id and files that aren't `.sql` are skipped; each file that
    /// can't be read is an error of its own, so one bad file doesn't hide the others.
    ///
    /// # Errors
    ///
    /// [`ConsoleStoreError::Read`] if the folder exists but can't be listed.
    pub fn load_all(
        &self,
    ) -> Result<Vec<Result<SavedConsole, ConsoleStoreError>>, ConsoleStoreError> {
        let mut consoles = Vec::new();
        for (data_source, folder) in self.data_source_folders()? {
            for (name, path) in console_files(&folder)? {
                let key = ConsoleKey { data_source, name };
                consoles.push(
                    fs::read_to_string(&path)
                        .map(|text| SavedConsole { key, text })
                        .map_err(|source| ConsoleStoreError::Read { path, source }),
                );
            }
        }
        Ok(consoles)
    }

    fn data_source_folders(&self) -> Result<Vec<(DataSourceId, PathBuf)>, ConsoleStoreError> {
        Ok(list(&self.root)?
            .filter_map(|path| {
                let id = path.file_name()?.to_str()?.parse().ok()?;
                path.is_dir().then_some((id, path))
            })
            .collect())
    }

    /// Replaces the console's file with `text`, atomically.
    ///
    /// # Errors
    ///
    /// [`ConsoleStoreError::Write`] if the folder or file can't be written.
    pub fn save(&self, key: &ConsoleKey, text: &str) -> Result<(), ConsoleStoreError> {
        let path = self.path(key);
        write_atomically(&path, text).map_err(|source| ConsoleStoreError::Write { path, source })
    }

    /// Removes the console's file. Succeeds if there was none.
    ///
    /// # Errors
    ///
    /// [`ConsoleStoreError::Write`] if the file exists but can't be removed.
    pub fn delete(&self, key: &ConsoleKey) -> Result<(), ConsoleStoreError> {
        let path = self.path(key);
        match fs::remove_file(&path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                Err(ConsoleStoreError::Write {
                    path,
                    source: error,
                })
            }
            _ => Ok(()),
        }
    }
}

/// The `.sql` files directly in `folder`, by console name.
fn console_files(folder: &Path) -> Result<Vec<(ConsoleName, PathBuf)>, ConsoleStoreError> {
    Ok(list(folder)?
        .filter_map(|path| {
            if path.extension()? != EXTENSION || !path.is_file() {
                return None;
            }
            let stem = path.file_stem()?.to_str()?.to_owned();
            let name = ConsoleName::try_from(stem).ok()?;
            Some((name, path))
        })
        .collect())
}

/// The entries of `folder`, or none if it doesn't exist.
fn list(folder: &Path) -> Result<impl Iterator<Item = PathBuf>, ConsoleStoreError> {
    let read_error = |source| ConsoleStoreError::Read {
        path: folder.to_owned(),
        source,
    };
    let entries = match fs::read_dir(folder) {
        Ok(entries) => Some(entries),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(read_error(error)),
    };
    Ok(entries
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(data_source: DataSourceId, name: &str) -> ConsoleKey {
        ConsoleKey {
            data_source,
            name: ConsoleName::try_from(name.to_owned()).unwrap(),
        }
    }

    #[test]
    fn accepts_only_plain_file_names() {
        for name in ["console", "console_2", "monthly report", "ü"] {
            assert!(ConsoleName::try_from(name.to_owned()).is_ok(), "{name}");
        }
        for name in ["", ".", "..", ".hidden", "a/b", "../x", "a\\b", "nul\0"] {
            assert!(ConsoleName::try_from(name.to_owned()).is_err(), "{name:?}");
        }
    }

    #[test]
    fn numbers_names_like_datagrip() {
        let cases = [(1, "console", Some(1)), (2, "console_2", Some(2))];
        for (n, name, number) in cases {
            assert_eq!(ConsoleName::numbered(n).as_str(), name);
            assert_eq!(ConsoleName::numbered(n).number(), number);
        }
        for name in ["console_1", "console_x", "consoles", "report"] {
            let name = ConsoleName::try_from(name.to_owned()).unwrap();
            assert_eq!(name.number(), None, "{name}");
        }
    }

    #[test]
    fn saves_loads_and_deletes_per_data_source() {
        let dir = tempfile::tempdir().unwrap();
        let files = ConsoleFiles::new(dir.path().join("consoles"));
        assert!(files.load_all().unwrap().is_empty());
        let (shop, crm) = (DataSourceId::generate(), DataSourceId::generate());

        files.save(&key(shop, "console"), "select 1;").unwrap();
        files.save(&key(shop, "console"), "select 2;").unwrap();
        files.save(&key(crm, "console_2"), "").unwrap();
        fs::write(
            dir.path()
                .join("consoles")
                .join(shop.to_string())
                .join("notes.txt"),
            "",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("consoles/not-an-id")).unwrap();

        let mut loaded: Vec<SavedConsole> = files
            .load_all()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect();
        loaded.sort_by(|a, b| a.key.cmp(&b.key));
        let mut expected = vec![
            SavedConsole {
                key: key(shop, "console"),
                text: "select 2;".to_owned(),
            },
            SavedConsole {
                key: key(crm, "console_2"),
                text: String::new(),
            },
        ];
        expected.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(loaded, expected);

        files.delete(&key(shop, "console")).unwrap();
        files.delete(&key(shop, "console")).unwrap();
        assert_eq!(files.load_all().unwrap().len(), 1);
    }
}
