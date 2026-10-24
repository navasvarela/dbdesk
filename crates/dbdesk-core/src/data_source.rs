use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::DriverError;
use crate::placeholder::{PlaceholderError, Setting, Variables, expand, expand_path};

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

/// Stable identity of a project, a named group of data sources. Like [`DataSourceId`], written
/// once, so renaming a project never breaks the data sources (or MCP clients) that refer to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectId(Uuid);

impl ProjectId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
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
    /// The project the data source is grouped under; `None` is "No project". Absent from
    /// files written before projects existed. An id `projects.toml` doesn't know also means
    /// no project, rather than an error, so deleting that file never loses a data source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectId>,
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
    /// MySQL and MariaDB: one driver serves both.
    Mysql(MysqlConfig),
    /// Microsoft SQL Server and Azure SQL.
    #[serde(rename = "sqlserver")]
    SqlServer(SqlServerConfig),
}

impl ConnectionConfig {
    pub fn driver(&self) -> DriverKind {
        match self {
            Self::Postgres(_) => DriverKind::Postgres,
            Self::Sqlite(_) => DriverKind::Sqlite,
            Self::Mysql(_) => DriverKind::Mysql,
            Self::SqlServer(_) => DriverKind::SqlServer,
        }
    }

    /// The database a connection opens, on engines whose catalog has databases above its
    /// schemas; the explorer lists that database's schemas. MySQL's databases are its
    /// schemas, so it has no such level.
    pub fn database(&self) -> Option<&str> {
        match self {
            Self::Postgres(config) => Some(&config.server.database),
            Self::SqlServer(config) => Some(&config.server.database),
            Self::Sqlite(_) | Self::Mysql(_) => None,
        }
    }

    /// The schema unqualified names resolve to, as a new connection sees it. A fact of the
    /// configuration rather than of the dialect: on MySQL it is the database connected to.
    /// On SQL Server it is each user's own setting, `dbo` unless an administrator chose
    /// another.
    pub fn default_schema(&self) -> &str {
        match self {
            Self::Postgres(_) => "public",
            Self::SqlServer(_) => "dbo",
            Self::Sqlite(_) => "main",
            Self::Mysql(config) => &config.server.database,
        }
    }

    /// These settings with every `${NAME}` placeholder replaced, and a leading `~` in paths
    /// replaced by the home folder: what a driver connects with.
    ///
    /// # Errors
    ///
    /// [`PlaceholderError`] naming the setting that uses an unset variable, or whose port
    /// doesn't resolve to a number.
    pub fn resolve(&self, variables: &dyn Variables) -> Result<Self, PlaceholderError> {
        Ok(match self {
            Self::Postgres(config) => Self::Postgres(PostgresConfig {
                server: config.server.resolve(variables)?,
            }),
            Self::Mysql(config) => Self::Mysql(MysqlConfig {
                server: config.server.resolve(variables)?,
            }),
            Self::SqlServer(config) => Self::SqlServer(SqlServerConfig {
                server: config.server.resolve(variables)?,
            }),
            Self::Sqlite(config) => Self::Sqlite(SqliteConfig {
                path: expand_path(&config.path, variables)
                    .map_err(PlaceholderError::in_setting(Setting::File))?,
                read_only: config.read_only,
            }),
        })
    }

    /// A name for a data source the user didn't name: `database@host`, or the file name.
    pub fn default_name(&self) -> String {
        match self {
            Self::Postgres(config) => config.server.default_name(),
            Self::Mysql(config) => config.server.default_name(),
            Self::SqlServer(config) => config.server.default_name(),
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
            Self::Postgres | Self::Mysql | Self::SqlServer => true,
            Self::Sqlite => false,
        }
    }
}

/// Describes where the connection goes, for logs and tooltips. Never includes secrets.
impl fmt::Display for ConnectionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postgres(config) => config.server.fmt(f),
            Self::Mysql(config) => config.server.fmt(f),
            Self::SqlServer(config) => config.server.fmt(f),
            Self::Sqlite(config) if config.read_only => {
                write!(f, "{} (read-only)", config.path.display())
            }
            Self::Sqlite(config) => write!(f, "{}", config.path.display()),
        }
    }
}

/// Where a database server is and whom to connect as: what every network engine's settings
/// start with, and what the data source dialog's server section edits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: Port,
    pub database: String,
    pub user: String,
    /// Absent from files written before TLS existed, which then get the default.
    #[serde(default)]
    pub tls: TlsMode,
}

impl ServerConfig {
    fn default_name(&self) -> String {
        format!("{}@{}", self.database, self.host)
    }

    /// The port to connect to, for drivers.
    ///
    /// # Errors
    ///
    /// [`DriverError::Connect`] if the port is still a placeholder: the engine resolves
    /// every setting before it connects, so this points at a caller that skipped that.
    pub fn port_number(&self) -> Result<u16, DriverError> {
        self.port.number().ok_or_else(|| {
            DriverError::Connect(
                format!("the port {} has not been resolved to a number", self.port).into(),
            )
        })
    }

    fn resolve(&self, variables: &dyn Variables) -> Result<Self, PlaceholderError> {
        let text = |text: &str, setting| {
            expand(text, variables).map_err(PlaceholderError::in_setting(setting))
        };
        Ok(Self {
            host: text(&self.host, Setting::Host)?,
            port: self.port.resolve(variables)?,
            database: text(&self.database, Setting::Database)?,
            user: text(&self.user, Setting::User)?,
            tls: match &self.tls {
                TlsMode::VerifyFull {
                    ca_file: Some(path),
                } => TlsMode::VerifyFull {
                    ca_file: Some(
                        expand_path(path, variables)
                            .map_err(PlaceholderError::in_setting(Setting::CaFile))?,
                    ),
                },
                tls => tls.clone(),
            },
        })
    }
}

/// A server's port: a number, or text with a `${NAME}` placeholder that resolves to one when
/// connecting. Written to the file as a number, or as a string when it has a placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Port {
    Number(u16),
    Placeholder(String),
}

impl Port {
    /// The port number, once resolved. `None` for a placeholder: drivers only see ports that
    /// [`ConnectionConfig::resolve`] has turned into numbers.
    pub fn number(&self) -> Option<u16> {
        match self {
            Self::Number(port) => Some(*port),
            Self::Placeholder(_) => None,
        }
    }

    fn resolve(&self, variables: &dyn Variables) -> Result<Self, PlaceholderError> {
        let Self::Placeholder(text) = self else {
            return Ok(self.clone());
        };
        let value = expand(text, variables).map_err(PlaceholderError::in_setting(Setting::Port))?;
        value
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .map(Self::Number)
            .ok_or(PlaceholderError::InvalidPort {
                setting: Setting::Port,
                value,
            })
    }
}

impl From<u16> for Port {
    fn from(port: u16) -> Self {
        Self::Number(port)
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(port) => port.fmt(f),
            Self::Placeholder(text) => f.write_str(text),
        }
    }
}

/// `user@host:port/database`.
impl fmt::Display for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}@{}:{}/{}",
            self.user, self.host, self.port, self.database
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresConfig {
    /// Flattened, so the file keeps `host`, `port` and the rest next to `driver`, as it did
    /// before the server settings were shared between engines.
    #[serde(flatten)]
    pub server: ServerConfig,
}

impl PostgresConfig {
    pub const DEFAULT_PORT: u16 = 5432;
}

/// A MySQL or MariaDB server. Its `database` is the one connected to, which unqualified
/// names resolve in, and the explorer lists every database the user can see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MysqlConfig {
    #[serde(flatten)]
    pub server: ServerConfig,
}

impl MysqlConfig {
    pub const DEFAULT_PORT: u16 = 3306;
}

/// A SQL Server or Azure SQL database, signed in to with a SQL Server login. Its `database` is
/// the one connected to; the explorer lists every database the login may open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlServerConfig {
    #[serde(flatten)]
    pub server: ServerConfig,
}

impl SqlServerConfig {
    pub const DEFAULT_PORT: u16 = 1433;
}

/// Whether and how a network connection is encrypted. The modes are libpq's `sslmode` values
/// of the same names, so they mean what PostgreSQL users already expect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum TlsMode {
    /// Never encrypt.
    Disable,
    /// Encrypt if the server offers it, without checking its certificate; otherwise connect
    /// unencrypted. The default, as in libpq: it changes nothing for servers without TLS.
    #[default]
    Prefer,
    /// Always encrypt, without checking the server's certificate. Protects against someone
    /// reading the traffic, not against someone impersonating the server.
    Require,
    /// Always encrypt, and check that the certificate is signed by a trusted authority and
    /// names the host connected to.
    VerifyFull {
        /// A PEM file with the authorities to trust instead of the public web ones, for
        /// servers with a private or self-signed certificate.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ca_file: Option<PathBuf>,
    },
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
    Mysql,
    SqlServer,
}

impl DriverKind {
    pub const ALL: [Self; 4] = [Self::Postgres, Self::Sqlite, Self::Mysql, Self::SqlServer];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::Sqlite => "SQLite",
            Self::Mysql => "MySQL / MariaDB",
            Self::SqlServer => "SQL Server",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placeholder::tests::Fixed;

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
                    project: None,
                    config: ConnectionConfig::Postgres(PostgresConfig {
                        server: ServerConfig {
                            host: "localhost".to_owned(),
                            port: 5432.into(),
                            database: "app".to_owned(),
                            user: "juan".to_owned(),
                            tls: TlsMode::VerifyFull {
                                ca_file: Some("/etc/ssl/internal-ca.pem".into()),
                            },
                        },
                    }),
                },
                DataSource {
                    id: DataSourceId::generate(),
                    name: "chinook".to_owned(),
                    color: None,
                    project: None,
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
    fn tls_defaults_to_prefer_in_files_written_before_it_existed() {
        let without = r#"
            [[data_source]]
            id = "4b5f9a3e-8a52-4a7e-9c6b-1f0f2b0c9d11"
            name = "local"
            driver = "postgres"
            host = "localhost"
            port = 5432
            database = "app"
            user = "juan"
        "#;

        let file: File = toml::from_str(without).unwrap();

        let ConnectionConfig::Postgres(config) = &file.data_source[0].config else {
            panic!("expected a PostgreSQL data source");
        };
        assert_eq!(config.server.tls, TlsMode::Prefer);
    }

    #[test]
    fn writes_tls_as_a_mode_with_its_ca_file() {
        let modes = [
            (TlsMode::Disable, "mode = \"disable\""),
            (TlsMode::Require, "mode = \"require\""),
            (
                TlsMode::VerifyFull {
                    ca_file: Some("/ca.pem".into()),
                },
                "ca_file = \"/ca.pem\"",
            ),
            (
                TlsMode::VerifyFull { ca_file: None },
                "mode = \"verify-full\"",
            ),
        ];
        for (tls, expected) in modes {
            let file = File {
                data_source: vec![DataSource {
                    id: DataSourceId::generate(),
                    name: "local".to_owned(),
                    color: None,
                    project: None,
                    config: ConnectionConfig::Postgres(PostgresConfig {
                        server: ServerConfig {
                            host: "localhost".to_owned(),
                            port: 5432.into(),
                            database: "app".to_owned(),
                            user: "juan".to_owned(),
                            tls,
                        },
                    }),
                }],
            };

            let text = toml::to_string(&file).unwrap();

            assert!(text.contains(expected), "{text}");
            assert_eq!(toml::from_str::<File>(&text).unwrap(), file);
        }
    }

    #[test]
    fn reads_a_mysql_data_source_with_its_driver_tag() {
        let text = r#"
            [[data_source]]
            id = "4b5f9a3e-8a52-4a7e-9c6b-1f0f2b0c9d11"
            name = "shop"
            driver = "mysql"
            host = "localhost"
            port = 3306
            database = "shop"
            user = "root"
        "#;

        let file: File = toml::from_str(text).unwrap();

        let config = &file.data_source[0].config;
        assert_eq!(config.driver(), DriverKind::Mysql);
        assert_eq!(config.default_schema(), "shop");
        assert_eq!(config.database(), None);
        assert_eq!(config.to_string(), "root@localhost:3306/shop");
        assert_eq!(
            toml::from_str::<File>(&toml::to_string(&file).unwrap()).unwrap(),
            file
        );
    }

    #[test]
    fn reads_a_sql_server_data_source_with_its_driver_tag() {
        let text = r#"
            [[data_source]]
            id = "4b5f9a3e-8a52-4a7e-9c6b-1f0f2b0c9d11"
            name = "sales"
            driver = "sqlserver"
            host = "localhost"
            port = 1433
            database = "sales"
            user = "sa"

            [data_source.tls]
            mode = "require"
        "#;

        let file: File = toml::from_str(text).unwrap();

        let config = &file.data_source[0].config;
        assert_eq!(config.driver(), DriverKind::SqlServer);
        assert_eq!(config.default_schema(), "dbo");
        assert_eq!(config.database(), Some("sales"));
        assert_eq!(config.to_string(), "sa@localhost:1433/sales");
        assert_eq!(
            toml::from_str::<File>(&toml::to_string(&file).unwrap()).unwrap(),
            file
        );
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

    /// A file as dbdesk 0.2.0 wrote it, before projects existed.
    const FILE_FROM_0_2_0: &str = r#"
        # dbdesk data sources. Safe to edit by hand while dbdesk is closed.
        # Passwords are stored in the OS keychain, never in this file.
        # Comments are not preserved when dbdesk saves this file.

        [[data_source]]
        id = "4b5f9a3e-8a52-4a7e-9c6b-1f0f2b0c9d11"
        name = "production"
        color = "red"
        driver = "postgres"
        host = "db.internal"
        port = 5432
        database = "app"
        user = "juan"

        [data_source.tls]
        mode = "verify-full"
        ca_file = "/etc/ssl/internal-ca.pem"

        [[data_source]]
        id = "0d6c2f4e-1b8a-4c3d-9e7f-5a6b7c8d9e0f"
        name = "shop"
        driver = "mysql"
        host = "localhost"
        port = 3306
        database = "shop"
        user = "root"

        [data_source.tls]
        mode = "prefer"

        [[data_source]]
        id = "9a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d"
        name = "chinook.db"
        driver = "sqlite"
        path = "/data/chinook.db"
        read_only = true
    "#;

    #[test]
    fn files_from_0_2_0_load_unchanged_with_every_data_source_in_no_project() {
        let file: File = toml::from_str(FILE_FROM_0_2_0).unwrap();

        assert_eq!(file.data_source.len(), 3);
        assert!(file.data_source.iter().all(|ds| ds.project.is_none()));
        assert_eq!(file.data_source[0].color, Some(SourceColor::Red));
        let text = toml::to_string(&file).unwrap();
        assert!(!text.contains("project"), "{text}");
        assert_eq!(toml::from_str::<File>(&text).unwrap(), file);
    }

    #[test]
    fn writes_the_project_only_when_set() {
        let mut file: File = toml::from_str(FILE_FROM_0_2_0).unwrap();
        let project = ProjectId::generate();
        file.data_source[2].project = Some(project);

        let text = toml::to_string(&file).unwrap();

        assert_eq!(text.matches("project = ").count(), 1, "{text}");
        assert!(text.contains(&format!("project = \"{project}\"")), "{text}");
        assert_eq!(toml::from_str::<File>(&text).unwrap(), file);
    }

    #[test]
    fn describes_the_target_without_secrets() {
        let postgres = ConnectionConfig::Postgres(PostgresConfig {
            server: ServerConfig {
                host: "db.internal".to_owned(),
                port: 6432.into(),
                database: "app".to_owned(),
                user: "juan".to_owned(),
                tls: TlsMode::Require,
            },
        });
        let sqlite = ConnectionConfig::Sqlite(SqliteConfig {
            path: "/data/app.db".into(),
            read_only: true,
        });

        assert_eq!(postgres.to_string(), "juan@db.internal:6432/app");
        assert_eq!(sqlite.to_string(), "/data/app.db (read-only)");
    }

    #[test]
    fn resolves_placeholders_in_every_setting_that_may_hold_one() {
        let text = r#"
            [[data_source]]
            id = "4b5f9a3e-8a52-4a7e-9c6b-1f0f2b0c9d11"
            name = "shared"
            driver = "postgres"
            host = "${PGHOST}"
            port = "${PGPORT}"
            database = "app_${ENV}"
            user = "${PGUSER}"

            [data_source.tls]
            mode = "verify-full"
            ca_file = "~/certs/${ENV}.pem"

            [[data_source]]
            id = "0d6c2f4e-1b8a-4c3d-9e7f-5a6b7c8d9e0f"
            name = "local"
            driver = "sqlite"
            path = "${DATA}/shop.db"
        "#;
        let variables = Fixed::new(&[
            ("PGHOST", "db.internal"),
            ("PGPORT", "6432"),
            ("ENV", "staging"),
            ("PGUSER", "juan"),
            ("DATA", "/srv"),
        ]);

        let file: File = toml::from_str(text).unwrap();
        let resolved: Vec<ConnectionConfig> = file
            .data_source
            .iter()
            .map(|ds| ds.config.resolve(&variables).unwrap())
            .collect();

        let ConnectionConfig::Postgres(postgres) = &resolved[0] else {
            panic!("expected a PostgreSQL data source");
        };
        assert_eq!(
            postgres.server.to_string(),
            "juan@db.internal:6432/app_staging"
        );
        assert_eq!(postgres.server.port.number(), Some(6432));
        assert_eq!(
            postgres.server.tls,
            TlsMode::VerifyFull {
                ca_file: Some("/home/juan/certs/staging.pem".into())
            }
        );
        let ConnectionConfig::Sqlite(sqlite) = &resolved[1] else {
            panic!("expected a SQLite data source");
        };
        assert_eq!(sqlite.path, PathBuf::from("/srv/shop.db"));
        let written = toml::to_string(&file).unwrap();
        assert!(written.contains(r#"port = "${PGPORT}""#), "{written}");
        assert_eq!(toml::from_str::<File>(&written).unwrap(), file);
    }

    #[test]
    fn names_the_setting_an_unset_or_wrong_variable_is_in() {
        let config = |host: &str, port: Port| {
            ConnectionConfig::Mysql(MysqlConfig {
                server: ServerConfig {
                    host: host.to_owned(),
                    port,
                    database: "shop".to_owned(),
                    user: "root".to_owned(),
                    tls: TlsMode::Prefer,
                },
            })
        };
        let variables = Fixed::new(&[("WORD", "abc")]);
        let cases = [
            (
                config("${DBHOST}", 3306.into()),
                "could not resolve the host: ${DBHOST} is not set",
            ),
            (
                config("localhost", Port::Placeholder("${WORD}".to_owned())),
                "the port resolves to \"abc\", which is not a port number from 1 to 65535",
            ),
        ];
        for (config, expected) in cases {
            let error = config.resolve(&variables).unwrap_err();
            assert_eq!(crate::describe_error(&error), expected);
        }
    }
}
