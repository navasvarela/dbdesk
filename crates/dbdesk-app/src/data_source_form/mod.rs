//! The editable fields behind the data source dialog, and their validation. No egui here, so
//! the rules are testable without a UI.

mod server;

use dbdesk_core::{
    ClickHouseConfig, ConnectionConfig, DataSource, DataSourceId, DriverKind, DuckDbConfig,
    DuckDbDatabase, McpLogin, MysqlConfig, Password, PostgresConfig, ProjectId, SafetyLevel,
    SourceColor, SqlServerConfig, SqliteConfig,
};
use dbdesk_engine::PasswordUpdate;

pub use server::{ServerDefaults, ServerFields, TlsChoice, TlsOffer};

const POSTGRES_DEFAULTS: ServerDefaults = ServerDefaults {
    host: "localhost",
    port: PostgresConfig::DEFAULT_PORT,
    database: "postgres",
    user: "postgres",
    tls: TlsOffer::Negotiated,
};

/// `mysql` exists on every server, as `postgres` does on PostgreSQL; `root` is the account
/// every installation starts with.
const MYSQL_DEFAULTS: ServerDefaults = ServerDefaults {
    host: "localhost",
    port: MysqlConfig::DEFAULT_PORT,
    database: "mysql",
    user: "root",
    tls: TlsOffer::Negotiated,
};

/// `master` exists on every server and every login may open it; `sa` is the administrator
/// every installation (and the official container) starts with.
const SQL_SERVER_DEFAULTS: ServerDefaults = ServerDefaults {
    host: "localhost",
    port: SqlServerConfig::DEFAULT_PORT,
    database: "master",
    user: "sa",
    tls: TlsOffer::Negotiated,
};

/// `default` exists on every server, and is the user every installation (and the official
/// container) starts with.
const CLICKHOUSE_DEFAULTS: ServerDefaults = ServerDefaults {
    host: "localhost",
    port: ClickHouseConfig::DEFAULT_PORT,
    database: "default",
    user: "default",
    tls: TlsOffer::OwnPort(ClickHouseConfig::DEFAULT_TLS_PORT),
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FormError {
    #[error("Host is required.")]
    HostMissing,
    #[error("Port must be a number from 1 to 65535, or a ${{VARIABLE}}.")]
    InvalidPort,
    #[error("Database is required.")]
    DatabaseMissing,
    #[error("User is required.")]
    UserMissing,
    #[error("File is required.")]
    PathMissing,
}

/// Text as the user typed it. Converted to a [`DataSource`] only on Test or Save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSourceForm {
    /// Optional: an empty name becomes [`ConnectionConfig::default_name`].
    pub name: String,
    pub color: Option<SourceColor>,
    pub safety: SafetyLevel,
    /// Not edited in the dialog, only carried through it, so saving an edit keeps the data
    /// source in its project.
    pub project: Option<ProjectId>,
    /// Carried through the dialog like `project`.
    pub mcp_login: Option<McpLogin>,
    pub fields: DriverFields,
}

/// Each engine's fields: the shared [`ServerFields`] for a network engine, plus whatever is
/// particular to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverFields {
    Postgres(PostgresFields),
    Sqlite(SqliteFields),
    Mysql(MysqlFields),
    SqlServer(SqlServerFields),
    DuckDb(DuckDbFields),
    ClickHouse(ClickHouseFields),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresFields {
    pub server: ServerFields,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MysqlFields {
    pub server: ServerFields,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlServerFields {
    pub server: ServerFields,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClickHouseFields {
    pub server: ServerFields,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuckDbFields {
    pub location: DuckDbLocation,
    /// Kept while In memory is chosen, so switching back to File doesn't lose it.
    pub path: String,
    pub read_only: bool,
}

/// Where a DuckDB data source's database is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuckDbLocation {
    File,
    InMemory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteFields {
    pub path: String,
    pub read_only: bool,
}

impl DriverFields {
    fn new(driver: DriverKind) -> Self {
        match driver {
            DriverKind::Postgres => Self::Postgres(PostgresFields {
                server: ServerFields::new(POSTGRES_DEFAULTS),
            }),
            DriverKind::Sqlite => Self::Sqlite(SqliteFields {
                path: String::new(),
                read_only: false,
            }),
            DriverKind::Mysql => Self::Mysql(MysqlFields {
                server: ServerFields::new(MYSQL_DEFAULTS),
            }),
            DriverKind::SqlServer => Self::SqlServer(SqlServerFields {
                server: ServerFields::new(SQL_SERVER_DEFAULTS),
            }),
            DriverKind::DuckDb => Self::DuckDb(DuckDbFields {
                location: DuckDbLocation::File,
                path: String::new(),
                read_only: false,
            }),
            DriverKind::ClickHouse => Self::ClickHouse(ClickHouseFields {
                server: ServerFields::new(CLICKHOUSE_DEFAULTS),
            }),
        }
    }

    fn from_config(config: &ConnectionConfig) -> Self {
        match config {
            ConnectionConfig::Postgres(config) => Self::Postgres(PostgresFields {
                server: ServerFields::from_config(&config.server, POSTGRES_DEFAULTS),
            }),
            ConnectionConfig::Sqlite(config) => Self::Sqlite(SqliteFields {
                path: config.path.display().to_string(),
                read_only: config.read_only,
            }),
            ConnectionConfig::Mysql(config) => Self::Mysql(MysqlFields {
                server: ServerFields::from_config(&config.server, MYSQL_DEFAULTS),
            }),
            ConnectionConfig::SqlServer(config) => Self::SqlServer(SqlServerFields {
                server: ServerFields::from_config(&config.server, SQL_SERVER_DEFAULTS),
            }),
            ConnectionConfig::DuckDb(config) => Self::DuckDb(match &config.database {
                DuckDbDatabase::File { path, read_only } => DuckDbFields {
                    location: DuckDbLocation::File,
                    path: path.display().to_string(),
                    read_only: *read_only,
                },
                DuckDbDatabase::InMemory => DuckDbFields {
                    location: DuckDbLocation::InMemory,
                    path: String::new(),
                    read_only: false,
                },
            }),
            ConnectionConfig::ClickHouse(config) => Self::ClickHouse(ClickHouseFields {
                server: ServerFields::from_config(&config.server, CLICKHOUSE_DEFAULTS),
            }),
        }
    }

    fn driver(&self) -> DriverKind {
        match self {
            Self::Postgres(_) => DriverKind::Postgres,
            Self::Sqlite(_) => DriverKind::Sqlite,
            Self::Mysql(_) => DriverKind::Mysql,
            Self::SqlServer(_) => DriverKind::SqlServer,
            Self::DuckDb(_) => DriverKind::DuckDb,
            Self::ClickHouse(_) => DriverKind::ClickHouse,
        }
    }

    /// The server section, for engines that connect over the network.
    fn server(&self) -> Option<&ServerFields> {
        match self {
            Self::Postgres(fields) => Some(&fields.server),
            Self::Mysql(fields) => Some(&fields.server),
            Self::SqlServer(fields) => Some(&fields.server),
            Self::ClickHouse(fields) => Some(&fields.server),
            Self::Sqlite(_) | Self::DuckDb(_) => None,
        }
    }

    fn to_config(&self) -> Result<ConnectionConfig, FormError> {
        Ok(match self {
            Self::Postgres(fields) => ConnectionConfig::Postgres(PostgresConfig {
                server: fields.server.to_config()?,
            }),
            Self::Sqlite(fields) => ConnectionConfig::Sqlite(fields.to_config()?),
            Self::Mysql(fields) => ConnectionConfig::Mysql(MysqlConfig {
                server: fields.server.to_config()?,
            }),
            Self::SqlServer(fields) => ConnectionConfig::SqlServer(SqlServerConfig {
                server: fields.server.to_config()?,
            }),
            Self::DuckDb(fields) => ConnectionConfig::DuckDb(fields.to_config()?),
            Self::ClickHouse(fields) => ConnectionConfig::ClickHouse(ClickHouseConfig {
                server: fields.server.to_config()?,
            }),
        })
    }
}

impl DataSourceForm {
    pub fn new(driver: DriverKind) -> Self {
        Self {
            name: String::new(),
            color: None,
            safety: SafetyLevel::Normal,
            project: None,
            mcp_login: None,
            fields: DriverFields::new(driver),
        }
    }

    pub fn from_data_source(data_source: &DataSource) -> Self {
        Self {
            name: data_source.name.clone(),
            color: data_source.color,
            safety: data_source.safety,
            project: data_source.project,
            mcp_login: data_source.mcp_login.clone(),
            fields: DriverFields::from_config(&data_source.config),
        }
    }

    pub fn driver(&self) -> DriverKind {
        self.fields.driver()
    }

    pub fn to_data_source(&self, id: DataSourceId) -> Result<DataSource, FormError> {
        let config = self.fields.to_config()?;
        let name = match self.name.trim() {
            "" => config.default_name(),
            name => name.to_owned(),
        };
        Ok(DataSource {
            id,
            name,
            color: self.color,
            safety: self.safety,
            project: self.project,
            mcp_login: self.mcp_login.clone(),
            config,
        })
    }

    pub fn password_update(&self) -> PasswordUpdate {
        match self.fields.server() {
            Some(server) if !server.password.is_empty() => {
                PasswordUpdate::Set(Password::new(server.password.clone()))
            }
            _ => PasswordUpdate::Keep,
        }
    }
}

impl SqliteFields {
    fn to_config(&self) -> Result<SqliteConfig, FormError> {
        Ok(SqliteConfig {
            path: required(&self.path, FormError::PathMissing)?.into(),
            read_only: self.read_only,
        })
    }
}

impl DuckDbFields {
    fn to_config(&self) -> Result<DuckDbConfig, FormError> {
        let database = match self.location {
            DuckDbLocation::File => DuckDbDatabase::File {
                path: required(&self.path, FormError::PathMissing)?.into(),
                read_only: self.read_only,
            },
            DuckDbLocation::InMemory => DuckDbDatabase::InMemory,
        };
        Ok(DuckDbConfig { database })
    }
}

fn required(value: &str, missing: FormError) -> Result<String, FormError> {
    match value.trim() {
        "" => Err(missing),
        value => Ok(value.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use dbdesk_core::{ServerConfig, TlsMode};

    use super::*;

    fn postgres_form(edit: impl FnOnce(&mut ServerFields)) -> DataSourceForm {
        let mut form = DataSourceForm::new(DriverKind::Postgres);
        if let DriverFields::Postgres(fields) = &mut form.fields {
            edit(&mut fields.server);
        }
        form
    }

    #[test]
    fn names_unnamed_data_sources_and_reports_invalid_server_fields() {
        let named = postgres_form(|f| f.host = " db.internal ".to_owned())
            .to_data_source(DataSourceId::generate())
            .unwrap();
        let invalid = postgres_form(|f| f.port = "abc".to_owned());

        assert_eq!(named.name, "postgres@db.internal");
        assert_eq!(
            invalid.to_data_source(DataSourceId::generate()),
            Err(FormError::InvalidPort)
        );
    }

    #[test]
    fn round_trips_existing_data_sources() {
        let configs = [
            ConnectionConfig::Sqlite(SqliteConfig {
                path: "/data/chinook.db".into(),
                read_only: true,
            }),
            ConnectionConfig::Postgres(PostgresConfig {
                server: ServerConfig {
                    host: "db.internal".to_owned(),
                    port: 6432.into(),
                    database: "app".to_owned(),
                    user: "juan".to_owned(),
                    tls: TlsMode::VerifyFull {
                        ca_file: Some("/etc/ca.pem".into()),
                    },
                },
            }),
        ];
        for config in configs {
            let data_source = DataSource {
                id: DataSourceId::generate(),
                name: "saved".to_owned(),
                color: Some(SourceColor::Teal),
                safety: SafetyLevel::ReadOnly,
                project: Some(ProjectId::generate()),
                mcp_login: Some(McpLogin {
                    user: "assistant".to_owned(),
                }),
                config,
            };

            let form = DataSourceForm::from_data_source(&data_source);

            assert_eq!(form.to_data_source(data_source.id), Ok(data_source));
        }
    }

    #[test]
    fn only_a_typed_password_replaces_the_saved_one() {
        let mut form = DataSourceForm::new(DriverKind::Postgres);
        assert_eq!(form.password_update(), PasswordUpdate::Keep);

        form = postgres_form(|f| f.password = "s3cret".to_owned());

        assert_eq!(
            form.password_update(),
            PasswordUpdate::Set(Password::new("s3cret"))
        );
        assert_eq!(
            DataSourceForm::new(DriverKind::Sqlite).password_update(),
            PasswordUpdate::Keep
        );
    }
}
