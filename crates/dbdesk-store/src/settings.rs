//! `settings.toml`: preferences that aren't key bindings, such as how SQL is formatted. The
//! app's Settings window writes it; it can also be edited by hand.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic::write_atomically;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub format: FormatSettings,
    pub lint: LintSettings,
}

/// How the SQL formatter writes statements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FormatSettings {
    pub keyword_case: KeywordCase,
    /// Spaces per indentation level.
    pub indent_width: u8,
}

impl Default for FormatSettings {
    fn default() -> Self {
        Self {
            keyword_case: KeywordCase::Upper,
            indent_width: 4,
        }
    }
}

/// What the console checks while typing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LintSettings {
    /// Ids of the lint rules switched off. Kept as written; the app reports ids it doesn't
    /// know.
    pub disabled: Vec<String>,
    /// Whether a connected console asks its database to prepare (never run) its statements.
    pub ask_database: bool,
}

impl Default for LintSettings {
    fn default() -> Self {
        Self {
            disabled: Vec::new(),
            ask_database: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeywordCase {
    #[default]
    Upper,
    Lower,
    /// As typed.
    Preserve,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("could not read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{} is not a valid settings file", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("could not write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not write the settings as TOML")]
    Serialize(#[source] toml::ser::Error),
}

#[derive(Debug, Clone)]
pub struct SettingsFile {
    path: PathBuf,
}

impl SettingsFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The settings in the file; a missing file, or a setting it leaves out, means the
    /// default.
    ///
    /// # Errors
    ///
    /// [`SettingsError::Read`] or [`SettingsError::Parse`] if the file exists but can't be
    /// used.
    pub fn load(&self) -> Result<Settings, SettingsError> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Settings::default());
            }
            Err(source) => {
                return Err(SettingsError::Read {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        toml::from_str(&text).map_err(|source| SettingsError::Parse {
            path: self.path.clone(),
            source,
        })
    }

    /// Writes `settings`, replacing the file. Comments in a hand-edited file are lost.
    ///
    /// # Errors
    ///
    /// [`SettingsError::Write`] if the file can't be written.
    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        let text = toml::to_string(settings).map_err(SettingsError::Serialize)?;
        write_atomically(&self.path, &text).map_err(|source| SettingsError::Write {
            path: self.path.clone(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_or_setting_means_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(dir.path().join("settings.toml"));
        assert_eq!(file.load().unwrap(), Settings::default());

        std::fs::write(file.path(), "[format]\nkeyword_case = \"lower\"\n").unwrap();

        let settings = file.load().unwrap();
        assert_eq!(settings.format.keyword_case, KeywordCase::Lower);
        assert_eq!(settings.format.indent_width, 4);
    }

    #[test]
    fn saves_and_reads_back_and_rejects_unknown_settings() {
        let dir = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(dir.path().join("settings.toml"));
        let settings = Settings {
            format: FormatSettings {
                keyword_case: KeywordCase::Preserve,
                indent_width: 2,
            },
            lint: LintSettings {
                disabled: vec!["null-comparison".to_owned()],
                ask_database: false,
            },
        };

        file.save(&settings).unwrap();
        assert_eq!(file.load().unwrap(), settings);

        std::fs::write(file.path(), "[format]\nindent = 2\n").unwrap();
        assert!(matches!(file.load(), Err(SettingsError::Parse { .. })));
    }
}
