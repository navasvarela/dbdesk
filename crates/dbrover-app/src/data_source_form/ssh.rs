//! The SSH section of the server form: whether to reach the database through an SSH server,
//! and how to sign in to it.

use std::path::PathBuf;

use dbrover_core::{SshConfig, SshLogin};

use super::{FormError, parse_port, required};

/// Text as the user typed it. Every field is kept while SSH is off, so turning it back on
/// doesn't lose them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshFields {
    pub enabled: bool,
    pub host: String,
    pub port: String,
    pub user: String,
    pub login: SshLoginChoice,
    /// Only used by [`SshLoginChoice::KeyFile`], but kept while another login is chosen.
    pub key_file: String,
    /// The SSH password, or the key file's passphrase. Empty keeps the saved one.
    pub secret: String,
}

impl Default for SshFields {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: SshConfig::DEFAULT_PORT.to_string(),
            user: String::new(),
            login: SshLoginChoice::Agent,
            key_file: String::new(),
            secret: String::new(),
        }
    }
}

impl SshFields {
    pub fn from_config(ssh: Option<&SshConfig>) -> Self {
        let Some(ssh) = ssh else {
            return Self::default();
        };
        let (login, key_file) = match &ssh.login {
            SshLogin::Agent => (SshLoginChoice::Agent, String::new()),
            SshLogin::KeyFile { key_file } => {
                (SshLoginChoice::KeyFile, key_file.display().to_string())
            }
            SshLogin::Password => (SshLoginChoice::Password, String::new()),
        };
        Self {
            enabled: true,
            host: ssh.host.clone(),
            port: ssh.port.to_string(),
            user: ssh.user.clone(),
            login,
            key_file,
            secret: String::new(),
        }
    }

    /// `None` while SSH is off.
    pub fn to_config(&self) -> Result<Option<SshConfig>, FormError> {
        if !self.enabled {
            return Ok(None);
        }
        let login = match self.login {
            SshLoginChoice::Agent => SshLogin::Agent,
            SshLoginChoice::KeyFile => SshLogin::KeyFile {
                key_file: PathBuf::from(required(&self.key_file, FormError::SshKeyFileMissing)?),
            },
            SshLoginChoice::Password => SshLogin::Password,
        };
        Ok(Some(SshConfig {
            host: required(&self.host, FormError::SshHostMissing)?,
            port: parse_port(&self.port, FormError::InvalidSshPort)?,
            user: required(&self.user, FormError::SshUserMissing)?,
            login,
        }))
    }

    /// The secret typed for the chosen login, if it uses one and one was typed.
    pub fn typed_secret(&self) -> Option<&str> {
        let uses_secret = self.enabled && self.login != SshLoginChoice::Agent;
        (uses_secret && !self.secret.is_empty()).then_some(self.secret.as_str())
    }
}

/// The ways to sign in, as the form lists them: [`SshLogin`] without its key file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshLoginChoice {
    Agent,
    KeyFile,
    Password,
}

impl SshLoginChoice {
    pub const ALL: [Self; 3] = [Self::Agent, Self::KeyFile, Self::Password];

    pub fn label(self) -> &'static str {
        match self {
            Self::Agent => "SSH agent",
            Self::KeyFile => "Key file",
            Self::Password => "Password",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Agent => "With the keys the running SSH agent holds.",
            Self::KeyFile => {
                "With a private key file. An encrypted key's passphrase is kept in the keychain."
            }
            Self::Password => "With the SSH user's password, kept in the keychain.",
        }
    }

    /// What the secret field is called for this login; `None` if it has no secret.
    pub fn secret_label(self) -> Option<&'static str> {
        match self {
            Self::Agent => None,
            Self::KeyFile => Some("Passphrase"),
            Self::Password => Some("SSH password"),
        }
    }
}

#[cfg(test)]
mod tests {
    use dbrover_core::Port;

    use super::*;

    fn enabled(edit: impl FnOnce(&mut SshFields)) -> SshFields {
        let mut fields = SshFields {
            enabled: true,
            host: "bastion.example.com".to_owned(),
            user: "deploy".to_owned(),
            ..SshFields::default()
        };
        edit(&mut fields);
        fields
    }

    type FieldEdit = fn(&mut SshFields);

    #[test]
    fn is_off_until_enabled_and_keeps_what_was_typed_meanwhile() {
        let mut fields = enabled(|f| f.host = "jump".to_owned());
        fields.enabled = false;

        assert_eq!(fields.to_config(), Ok(None));
        assert_eq!(fields.host, "jump");
        assert_eq!(fields.typed_secret(), None);
    }

    #[test]
    fn rejects_invalid_fields() {
        let cases: [(FieldEdit, FormError); 5] = [
            (|f| f.host = " ".to_owned(), FormError::SshHostMissing),
            (|f| f.user = String::new(), FormError::SshUserMissing),
            (|f| f.port = "ssh".to_owned(), FormError::InvalidSshPort),
            (|f| f.port = "0".to_owned(), FormError::InvalidSshPort),
            (
                |f| f.login = SshLoginChoice::KeyFile,
                FormError::SshKeyFileMissing,
            ),
        ];
        for (edit, expected) in cases {
            assert_eq!(enabled(edit).to_config(), Err(expected));
        }
    }

    #[test]
    fn every_login_round_trips_through_the_fields() {
        let logins = [
            SshLogin::Agent,
            SshLogin::KeyFile {
                key_file: "~/.ssh/id_ed25519".into(),
            },
            SshLogin::Password,
        ];
        for login in logins {
            let config = SshConfig {
                host: "${BASTION}".to_owned(),
                port: Port::Placeholder("${BASTION_PORT}".to_owned()),
                user: "deploy".to_owned(),
                login,
            };

            let fields = SshFields::from_config(Some(&config));

            assert_eq!(fields.to_config(), Ok(Some(config)));
        }
    }

    #[test]
    fn a_secret_counts_only_for_a_login_that_uses_one() {
        let cases = [
            (SshLoginChoice::Agent, None),
            (SshLoginChoice::KeyFile, Some("typed")),
            (SshLoginChoice::Password, Some("typed")),
        ];
        for (login, expected) in cases {
            let fields = enabled(|f| {
                f.login = login;
                f.secret = "typed".to_owned();
            });
            assert_eq!(fields.typed_secret(), expected, "{login:?}");
        }
    }
}
