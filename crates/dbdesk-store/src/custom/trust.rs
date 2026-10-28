//! `trusted_commands.toml`: the command actions the user chose to run, each by its exact
//! definition, so a command that changes asks again. See `docs/hackability.md`.
//!
//! It lives in the data folder, not beside `actions.toml`: a config folder synced from another
//! machine, or copied from a colleague, brings its actions but none of this file's trust.

use std::path::{Path, PathBuf};
use std::{fs, io};

use serde::{Deserialize, Serialize};

use super::actions::ActionId;
use super::commands::{CommandAction, StdinFormat};
use crate::atomic::write_atomically;

const HEADER: &str = "\
# Commands from actions.toml that dbdesk runs without asking first, each as it was when you
# chose Run. dbdesk writes this file; remove an entry to be asked again.

";

/// What the user saw and agreed to run: which action, the command as written, and what it
/// reads on stdin. Where its output goes isn't part of it, since that can't do harm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedCommand {
    id: String,
    command: Vec<String>,
    stdin: StdinFormat,
}

impl TrustedCommand {
    pub fn of(id: &ActionId, action: &CommandAction) -> Self {
        Self {
            id: id.as_str().to_owned(),
            command: action.command.source().to_vec(),
            stdin: action.stdin,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedCommands(Vec<TrustedCommand>);

impl TrustedCommands {
    pub fn contains(&self, command: &TrustedCommand) -> bool {
        self.0.contains(command)
    }

    /// Trusts `command` from now on, in place of whatever its action was trusted to run
    /// before, which no longer exists in that form.
    pub fn trust(&mut self, command: TrustedCommand) {
        self.0.retain(|trusted| trusted.id != command.id);
        self.0.push(command);
    }
}

#[derive(Serialize, Deserialize)]
struct TrustFile {
    #[serde(default, rename = "command")]
    commands: Vec<TrustedCommand>,
}

#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    #[error("could not read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{} is not a list of trusted commands", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("could not serialize the trusted commands")]
    Serialize(#[source] toml::ser::Error),
    #[error("could not write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Where the trusted commands are kept.
#[derive(Debug, Clone)]
pub struct TrustedCommandsFile {
    path: PathBuf,
}

impl TrustedCommandsFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// No commands if there is no file yet.
    ///
    /// # Errors
    ///
    /// [`TrustError`] if the file exists but can't be read or isn't in this format.
    pub fn load(&self) -> Result<TrustedCommands, TrustError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(TrustedCommands::default());
            }
            Err(source) => {
                return Err(TrustError::Read {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        let file: TrustFile = toml::from_str(&text).map_err(|source| TrustError::Parse {
            path: self.path.clone(),
            source,
        })?;
        Ok(TrustedCommands(file.commands))
    }

    /// Replaces the file, atomically, creating its folder.
    ///
    /// # Errors
    ///
    /// [`TrustError`] if the file can't be written.
    pub fn save(&self, commands: &TrustedCommands) -> Result<(), TrustError> {
        let file = TrustFile {
            commands: commands.0.clone(),
        };
        let text = toml::to_string(&file).map_err(TrustError::Serialize)?;
        let failed = |source| TrustError::Write {
            path: self.path.clone(),
            source,
        };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(failed)?;
        }
        write_atomically(&self.path, &format!("{HEADER}{text}")).map_err(failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionBody, parse_actions};

    fn command(id: &str, command: &str, stdin: &str) -> TrustedCommand {
        let checked = parse_actions(&format!(
            "[[action]]\nid = {id:?}\nname = \"N\"\napplies_to = [\"grid.selection\"]\n\
             command = {command}\nstdin = {stdin:?}\n"
        ));
        let Some(action) = checked.value.first() else {
            panic!("{:?}", checked.problems);
        };
        let ActionBody::Command(body) = &action.body else {
            panic!("{action:?}");
        };
        TrustedCommand::of(&action.id, body)
    }

    #[test]
    fn keeps_what_was_trusted_and_forgets_an_older_definition() {
        let dir = tempfile::tempdir().unwrap();
        let file = TrustedCommandsFile::new(dir.path().join("data").join("trusted_commands.toml"));
        assert_eq!(file.load().unwrap(), TrustedCommands::default());

        let jq = command("jq", r#"["jq", "."]"#, "json");
        let mut trusted = TrustedCommands::default();
        trusted.trust(jq.clone());
        trusted.trust(command("wc", r#"["wc", "-l"]"#, "csv"));
        file.save(&trusted).unwrap();
        let loaded = file.load().unwrap();
        assert_eq!(loaded, trusted);
        assert!(loaded.contains(&jq));

        let changed = [
            command("jq", r#"["jq", "-r", "."]"#, "json"),
            command("jq", r#"["jq", "."]"#, "csv"),
            command("other", r#"["jq", "."]"#, "json"),
        ];
        for definition in &changed {
            assert!(!loaded.contains(definition), "{definition:?}");
        }

        let mut retrusted = loaded;
        retrusted.trust(changed[0].clone());
        assert!(!retrusted.contains(&jq));
        assert!(retrusted.contains(&changed[0]));
    }

    #[test]
    fn says_which_file_it_cant_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trusted_commands.toml");
        fs::write(&path, "[[command]]\nid = 3\n").unwrap();
        let error = TrustedCommandsFile::new(&path).load().unwrap_err();
        assert!(
            error
                .to_string()
                .ends_with("is not a list of trusted commands"),
            "{error}"
        );
    }
}
