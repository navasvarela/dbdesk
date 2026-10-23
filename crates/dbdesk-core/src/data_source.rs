use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identity of a data source. Written once when the data source is created, so renaming
/// it never breaks the keychain entry, consoles or history that refer to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DataSourceId(Uuid);

impl DataSourceId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for DataSourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Parses the form [`Display`](fmt::Display) writes, as found in folder names under `consoles/`.
impl FromStr for DataSourceId {
    type Err = uuid::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(text).map(Self)
    }
}

/// A saved connection configuration. Never holds a password; those live in the OS keychain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSource {
    pub id: DataSourceId,
    pub name: String,
    /// Tints the data source's explorer row and its consoles, so production stands out from
    /// a local copy at a glance. Optional, and absent from files written before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<SourceColor>,
    #[serde(flatten)]
    pub config: ConnectionConfig,
}

/// The colours a data source can be marked with.
///
/// A fixed set of names rather than free-form RGB: the app picks each one's shade per theme,
/// so a colour chosen in the dark theme still reads in the light one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceColor {
    Red,
    Orange,
    Yellow,
    Green,
    Teal,
    Blue,
    Violet,
    Pink,
}

impl SourceColor {
    pub const ALL: [Self; 8] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Teal,
        Self::Blue,
        Self::Violet,
        Self::Pink,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Teal => "Teal",
            Self::Blue => "Blue",
            Self::Violet => "Violet",
            Self::Pink => "Pink",
        }
    }
}

/// Engine-specific connection settings.
///
/// A closed enum rather than an open driver registry: each variant needs its own form in the
/// UI and its own driver crate anyway, and the compiler then points at every place a new
/// engine must be handled. See `docs/adr/0002-native-drivers-behind-a-trait.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "driver", rename_all = "lowercase")]
pub enum ConnectionConfig {
    Postgres(PostgresConfig),
    Sqlite(SqliteConfig),
}

impl ConnectionConfig {
    pub fn driver(&self) -> DriverKind {
        match self {
            Self::Postgres(_) => DriverKind::Postgres,
            Self::Sqlite(_) => DriverKind::Sqlite,
        }
    }

    /// The database a connection opens, on engines whose catalog has databases above its
    /// schemas; the explorer lists that database's schemas.
    pub fn database(&self) -> Option<&str> {
        match self {
            Self::Postgres(config) => Some(&config.database),
            Self::Sqlite(_) => None,
        }
    }

    /// A name for a data source the user didn't name: `database@host`, or the file name.
    pub fn default_name(&self) -> String {
        match self {
            Self::Postgres(config) => format!("{}@{}", config.database, config.host),
            Self::Sqlite(config) => config.path.file_name().map_or_else(
                || config.path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
        }
    }
}

impl DriverKind {
    /// Whether connections authenticate with a password, so the keychain is worth asking.
    /// Skipping the lookup for other drivers avoids pointless keychain prompts.
    pub fn uses_password(self) -> bool {
        match self {
            Self::Postgres => true,
            Self::Sqlite => false,
        }
    }
}

/// Describes where the connection goes, for logs and tooltips. Never includes secrets.
impl fmt::Display for ConnectionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postgres(config) => write!(
                f,
                "{}@{}:{}/{}",
                config.user, config.host, config.port, config.database
            ),
            Self::Sqlite(config) if config.read_only => {
                write!(f, "{} (read-only)", config.path.display())
            }
            Self::Sqlite(config) => write!(f, "{}", config.path.display()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
}

impl PostgresConfig {
    pub const DEFAULT_PORT: u16 = 5432;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqliteConfig {
    pub path: PathBuf,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DriverKind {
    Postgres,
    Sqlite,
}

impl DriverKind {
    pub const ALL: [Self; 2] = [Self::Postgres, Self::Sqlite];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::Sqlite => "SQLite",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct File {
        data_source: Vec<DataSource>,
    }

    #[test]
    fn round_trips_through_toml_with_a_driver_tag() {
        let file = File {
            data_source: vec![
                DataSource {
                    id: DataSourceId::generate(),
                    name: "local".to_owned(),
                    color: None,
                    config: ConnectionConfig::Postgres(PostgresConfig {
                        host: "localhost".to_owned(),
                        port: 5432,
                        database: "app".to_owned(),
                        user: "juan".to_owned(),
                    }),
                },
                DataSource {
                    id: DataSourceId::generate(),
                    name: "chinook".to_owned(),
                    color: None,
                    config: ConnectionConfig::Sqlite(SqliteConfig {
                        path: "/tmp/chinook.db".into(),
                        read_only: true,
                    }),
                },
            ],
        };

        let text = toml::to_string(&file).unwrap();
        assert!(text.contains("driver = \"postgres\""), "{text}");
        assert!(text.contains("driver = \"sqlite\""), "{text}");
        assert_eq!(toml::from_str::<File>(&text).unwrap(), file);
    }

    #[test]
    fn colour_is_optional_and_written_only_when_set() {
        let without = r#"
            [[data_source]]
            id = "4b5f9a3e-8a52-4a7e-9c6b-1f0f2b0c9d11"
            name = "chinook"
            driver = "sqlite"
            path = "/tmp/chinook.db"
        "#;
        let mut file: File = toml::from_str(without).unwrap();
        assert_eq!(file.data_source[0].color, None);
        assert!(!toml::to_string(&file).unwrap().contains("color"));

        file.data_source[0].color = Some(SourceColor::Violet);
        let text = toml::to_string(&file).unwrap();

        assert!(text.contains("color = \"violet\""), "{text}");
        assert_eq!(toml::from_str::<File>(&text).unwrap(), file);
    }

    #[test]
    fn describes_the_target_without_secrets() {
        let postgres = ConnectionConfig::Postgres(PostgresConfig {
            host: "db.internal".to_owned(),
            port: 6432,
            database: "app".to_owned(),
            user: "juan".to_owned(),
        });
        let sqlite = ConnectionConfig::Sqlite(SqliteConfig {
            path: "/data/app.db".into(),
            read_only: true,
        });

        assert_eq!(postgres.to_string(), "juan@db.internal:6432/app");
        assert_eq!(sqlite.to_string(), "/data/app.db (read-only)");
    }
}
