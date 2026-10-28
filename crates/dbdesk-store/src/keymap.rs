//! `keymap.toml`: which keys trigger which actions. Only the text is read here; the app knows
//! the actions and how to parse key names.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config_text::read_if_exists;

/// Action ids mapped to the shortcuts written for them, as written: `"cmd+enter"`.
pub type KeymapBindings = BTreeMap<String, Vec<String>>;

#[derive(Debug, thiserror::Error)]
pub enum KeymapError {
    #[error("could not read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{} is not a valid keymap file", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("could not create {}", path.display())]
    Create {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileContents {
    #[serde(default)]
    bindings: BTreeMap<String, OneOrMany>,
}

/// An action takes one shortcut, or a list of them (empty to unbind it).
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl From<OneOrMany> for Vec<String> {
    fn from(value: OneOrMany) -> Self {
        match value {
            OneOrMany::One(shortcut) => vec![shortcut],
            OneOrMany::Many(shortcuts) => shortcuts,
        }
    }
}

#[derive(Debug, Clone)]
pub struct KeymapFile {
    path: PathBuf,
}

impl KeymapFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The bindings the file sets. A missing file sets none, so every default applies.
    ///
    /// # Errors
    ///
    /// [`KeymapError::Read`] or [`KeymapError::Parse`] if the file exists but can't be used.
    pub fn load(&self) -> Result<KeymapBindings, KeymapError> {
        let read = read_if_exists(&self.path).map_err(|source| KeymapError::Read {
            path: self.path.clone(),
            source,
        })?;
        let Some(text) = read else {
            return Ok(KeymapBindings::new());
        };
        let contents: FileContents =
            toml::from_str(&text).map_err(|source| KeymapError::Parse {
                path: self.path.clone(),
                source,
            })?;
        Ok(contents
            .bindings
            .into_iter()
            .map(|(action, shortcuts)| (action, shortcuts.into()))
            .collect())
    }

    /// Writes `text` as a new keymap file. Never replaces an existing one, which holds the
    /// user's own bindings.
    ///
    /// # Errors
    ///
    /// [`KeymapError::Create`] if the file exists or can't be written.
    pub fn create(&self, text: &str) -> Result<(), KeymapError> {
        let create_error = |source| KeymapError::Create {
            path: self.path.clone(),
            source,
        };
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(create_error)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
            .map_err(create_error)?;
        file.write_all(text.as_bytes()).map_err(create_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_sets_no_bindings() {
        let dir = tempfile::tempdir().unwrap();

        let bindings = KeymapFile::new(dir.path().join("keymap.toml"))
            .load()
            .unwrap();

        assert!(bindings.is_empty());
    }

    #[test]
    fn reads_one_or_many_shortcuts_per_action() {
        let dir = tempfile::tempdir().unwrap();
        let file = KeymapFile::new(dir.path().join("keymap.toml"));
        file.create(
            r#"
            [bindings]
            "console.run" = "cmd+r"
            "console.explain" = ["cmd+alt+e", "f5"]
            "console.cancel" = []
            "#,
        )
        .unwrap();

        let bindings = file.load().unwrap();

        assert_eq!(bindings["console.run"], ["cmd+r"]);
        assert_eq!(bindings["console.explain"], ["cmd+alt+e", "f5"]);
        assert!(bindings["console.cancel"].is_empty());
    }

    #[test]
    fn rejects_unknown_sections_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let file = KeymapFile::new(dir.path().join("keymap.toml"));
        file.create("[bindigns]\n").unwrap();

        assert!(matches!(file.load(), Err(KeymapError::Parse { .. })));
        assert!(matches!(
            file.create("[bindings]\n"),
            Err(KeymapError::Create { .. })
        ));
    }
}
