//! The server section of the data source form: host, port, database, user, password and TLS,
//! shared by every network engine. An engine embeds [`ServerFields`] in its own fields and
//! adds only what is particular to it.

use std::path::PathBuf;

use dbdesk_core::{ServerConfig, TlsMode};

use super::{FormError, required};

/// What a new data source's server section starts with, and the hints its fields show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerDefaults {
    pub host: &'static str,
    pub port: u16,
    pub database: &'static str,
    pub user: &'static str,
}

/// Text as the user typed it, plus the TLS choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFields {
    pub host: String,
    pub port: String,
    pub database: String,
    pub user: String,
    /// Empty means "keep the saved password" (or none, for a new data source).
    pub password: String,
    pub tls: TlsChoice,
    /// Only used by [`TlsChoice::VerifyFull`], but kept while another mode is chosen so
    /// switching back doesn't lose it. Empty trusts the public web authorities.
    pub ca_file: String,
    pub defaults: ServerDefaults,
}

impl ServerFields {
    pub fn new(defaults: ServerDefaults) -> Self {
        Self {
            host: defaults.host.to_owned(),
            port: defaults.port.to_string(),
            database: defaults.database.to_owned(),
            user: defaults.user.to_owned(),
            password: String::new(),
            tls: TlsChoice::from(&TlsMode::default()),
            ca_file: String::new(),
            defaults,
        }
    }

    pub fn from_config(config: &ServerConfig, defaults: ServerDefaults) -> Self {
        let ca_file = match &config.tls {
            TlsMode::VerifyFull {
                ca_file: Some(path),
            } => path.display().to_string(),
            _ => String::new(),
        };
        Self {
            host: config.host.clone(),
            port: config.port.to_string(),
            database: config.database.clone(),
            user: config.user.clone(),
            password: String::new(),
            tls: TlsChoice::from(&config.tls),
            ca_file,
            defaults,
        }
    }

    pub fn to_config(&self) -> Result<ServerConfig, FormError> {
        let port = self
            .port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or(FormError::InvalidPort)?;
        Ok(ServerConfig {
            host: required(&self.host, FormError::HostMissing)?,
            port,
            database: required(&self.database, FormError::DatabaseMissing)?,
            user: required(&self.user, FormError::UserMissing)?,
            tls: self.tls_mode(),
        })
    }

    fn tls_mode(&self) -> TlsMode {
        match self.tls {
            TlsChoice::Disable => TlsMode::Disable,
            TlsChoice::Prefer => TlsMode::Prefer,
            TlsChoice::Require => TlsMode::Require,
            TlsChoice::VerifyFull => TlsMode::VerifyFull {
                ca_file: Some(self.ca_file.trim())
                    .filter(|path| !path.is_empty())
                    .map(PathBuf::from),
            },
        }
    }
}

/// The TLS modes, as the form lists them: [`TlsMode`] without its CA file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsChoice {
    Disable,
    Prefer,
    Require,
    VerifyFull,
}

impl TlsChoice {
    pub const ALL: [Self; 4] = [Self::Disable, Self::Prefer, Self::Require, Self::VerifyFull];

    /// libpq's `sslmode` names, which PostgreSQL users and hosting providers' guides use.
    pub fn label(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyFull => "verify-full",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Disable => "Never encrypt.",
            Self::Prefer => "Encrypt if the server offers it, without checking its certificate.",
            Self::Require => "Always encrypt, without checking the server's certificate.",
            Self::VerifyFull => {
                "Always encrypt, and check that the certificate is trusted and names the host."
            }
        }
    }
}

impl From<&TlsMode> for TlsChoice {
    fn from(tls: &TlsMode) -> Self {
        match tls {
            TlsMode::Disable => Self::Disable,
            TlsMode::Prefer => Self::Prefer,
            TlsMode::Require => Self::Require,
            TlsMode::VerifyFull { .. } => Self::VerifyFull,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULTS: ServerDefaults = ServerDefaults {
        host: "localhost",
        port: 5432,
        database: "postgres",
        user: "postgres",
    };

    fn fields(edit: impl FnOnce(&mut ServerFields)) -> ServerFields {
        let mut fields = ServerFields::new(DEFAULTS);
        edit(&mut fields);
        fields
    }

    type FieldEdit = fn(&mut ServerFields);

    #[test]
    fn rejects_invalid_fields() {
        let cases: [(FieldEdit, FormError); 6] = [
            (|f| f.host = "  ".to_owned(), FormError::HostMissing),
            (|f| f.port = "abc".to_owned(), FormError::InvalidPort),
            (|f| f.port = "0".to_owned(), FormError::InvalidPort),
            (|f| f.port = "70000".to_owned(), FormError::InvalidPort),
            (|f| f.database = String::new(), FormError::DatabaseMissing),
            (|f| f.user = String::new(), FormError::UserMissing),
        ];

        for (edit, expected) in cases {
            assert_eq!(fields(edit).to_config(), Err(expected));
        }
    }

    #[test]
    fn starts_from_the_engine_defaults_and_trims_what_is_typed() {
        let config = fields(|f| {
            f.host = " db.internal ".to_owned();
            f.port = " 6432 ".to_owned();
        })
        .to_config()
        .unwrap();

        assert_eq!(
            config,
            ServerConfig {
                host: "db.internal".to_owned(),
                port: 6432,
                database: "postgres".to_owned(),
                user: "postgres".to_owned(),
                tls: TlsMode::Prefer,
            }
        );
    }

    #[test]
    fn keeps_the_ca_file_for_verify_full_only() {
        let cases = [
            (TlsChoice::Require, " /etc/ca.pem ", TlsMode::Require),
            (
                TlsChoice::VerifyFull,
                " /etc/ca.pem ",
                TlsMode::VerifyFull {
                    ca_file: Some("/etc/ca.pem".into()),
                },
            ),
            (
                TlsChoice::VerifyFull,
                "  ",
                TlsMode::VerifyFull { ca_file: None },
            ),
        ];
        for (choice, ca_file, expected) in cases {
            let config = fields(|f| {
                f.tls = choice;
                f.ca_file = ca_file.to_owned();
            })
            .to_config()
            .unwrap();

            assert_eq!(config.tls, expected, "{choice:?} {ca_file:?}");
        }
    }

    #[test]
    fn every_tls_mode_round_trips_through_the_fields() {
        let modes = [
            TlsMode::Disable,
            TlsMode::Prefer,
            TlsMode::Require,
            TlsMode::VerifyFull { ca_file: None },
            TlsMode::VerifyFull {
                ca_file: Some("/etc/ca.pem".into()),
            },
        ];
        for tls in modes {
            let config = ServerConfig {
                host: "db.internal".to_owned(),
                port: 6432,
                database: "app".to_owned(),
                user: "juan".to_owned(),
                tls,
            };

            let fields = ServerFields::from_config(&config, DEFAULTS);

            assert_eq!(fields.to_config(), Ok(config));
        }
    }
}
