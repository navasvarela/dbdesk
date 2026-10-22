//! The editable fields behind the data source dialog, and their validation. No egui here, so
//! the rules are testable without a UI.

use dbdesk_core::{
    ConnectionConfig, DataSource, DataSourceId, DriverKind, Password, PostgresConfig, SqliteConfig,
};
use dbdesk_engine::PasswordUpdate;

const DEFAULT_POSTGRES_HOST: &str = "localhost";
const DEFAULT_POSTGRES_DATABASE: &str = "postgres";
const DEFAULT_POSTGRES_USER: &str = "postgres";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FormError {
    #[error("Host is required.")]
    HostMissing,
    #[error("Port must be a number from 1 to 65535.")]
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
    pub fields: DriverFields,
    /// Empty means "keep the saved password" (or none, for a new data source).
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverFields {
    Postgres(PostgresFields),
    Sqlite(SqliteFields),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresFields {
    pub host: String,
    pub port: String,
    pub database: String,
    pub user: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteFields {
    pub path: String,
    pub read_only: bool,
}

impl DataSourceForm {
    pub fn new(driver: DriverKind) -> Self {
        let fields = match driver {
            DriverKind::Postgres => DriverFields::Postgres(PostgresFields {
                host: DEFAULT_POSTGRES_HOST.to_owned(),
                port: PostgresConfig::DEFAULT_PORT.to_string(),
                database: DEFAULT_POSTGRES_DATABASE.to_owned(),
                user: DEFAULT_POSTGRES_USER.to_owned(),
            }),
            DriverKind::Sqlite => DriverFields::Sqlite(SqliteFields {
                path: String::new(),
                read_only: false,
            }),
        };
        Self {
            name: String::new(),
            fields,
            password: String::new(),
        }
    }

    pub fn from_data_source(data_source: &DataSource) -> Self {
        let fields = match &data_source.config {
            ConnectionConfig::Postgres(config) => DriverFields::Postgres(PostgresFields {
                host: config.host.clone(),
                port: config.port.to_string(),
                database: config.database.clone(),
                user: config.user.clone(),
            }),
            ConnectionConfig::Sqlite(config) => DriverFields::Sqlite(SqliteFields {
                path: config.path.display().to_string(),
                read_only: config.read_only,
            }),
        };
        Self {
            name: data_source.name.clone(),
            fields,
            password: String::new(),
        }
    }

    pub fn driver(&self) -> DriverKind {
        match self.fields {
            DriverFields::Postgres(_) => DriverKind::Postgres,
            DriverFields::Sqlite(_) => DriverKind::Sqlite,
        }
    }

    pub fn to_data_source(&self, id: DataSourceId) -> Result<DataSource, FormError> {
        let config = match &self.fields {
            DriverFields::Postgres(fields) => ConnectionConfig::Postgres(fields.to_config()?),
            DriverFields::Sqlite(fields) => ConnectionConfig::Sqlite(fields.to_config()?),
        };
        let name = match self.name.trim() {
            "" => config.default_name(),
            name => name.to_owned(),
        };
        Ok(DataSource { id, name, config })
    }

    pub fn password_update(&self) -> PasswordUpdate {
        if self.password.is_empty() || !self.driver().uses_password() {
            PasswordUpdate::Keep
        } else {
            PasswordUpdate::Set(Password::new(self.password.clone()))
        }
    }
}

impl PostgresFields {
    fn to_config(&self) -> Result<PostgresConfig, FormError> {
        let port = self
            .port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or(FormError::InvalidPort)?;
        Ok(PostgresConfig {
            host: required(&self.host, FormError::HostMissing)?,
            port,
            database: required(&self.database, FormError::DatabaseMissing)?,
            user: required(&self.user, FormError::UserMissing)?,
        })
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

fn required(value: &str, missing: FormError) -> Result<String, FormError> {
    match value.trim() {
        "" => Err(missing),
        value => Ok(value.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn postgres_form(edit: impl FnOnce(&mut PostgresFields)) -> DataSourceForm {
        let mut form = DataSourceForm::new(DriverKind::Postgres);
        if let DriverFields::Postgres(fields) = &mut form.fields {
            edit(fields);
        }
        form
    }

    type FieldEdit = fn(&mut PostgresFields);

    #[test]
    fn rejects_invalid_postgres_fields() {
        let cases: [(FieldEdit, FormError); 5] = [
            (|f| f.host = "  ".to_owned(), FormError::HostMissing),
            (|f| f.port = "abc".to_owned(), FormError::InvalidPort),
            (|f| f.port = "0".to_owned(), FormError::InvalidPort),
            (|f| f.port = "70000".to_owned(), FormError::InvalidPort),
            (|f| f.user = String::new(), FormError::UserMissing),
        ];

        for (edit, expected) in cases {
            let form = postgres_form(edit);
            assert_eq!(form.to_data_source(DataSourceId::generate()), Err(expected));
        }
    }

    #[test]
    fn trims_fields_and_names_unnamed_data_sources() {
        let form = postgres_form(|f| {
            f.host = " db.internal ".to_owned();
            f.port = " 6432 ".to_owned();
        });

        let data_source = form.to_data_source(DataSourceId::generate()).unwrap();

        assert_eq!(data_source.name, "postgres@db.internal");
        assert_eq!(
            data_source.config,
            ConnectionConfig::Postgres(PostgresConfig {
                host: "db.internal".to_owned(),
                port: 6432,
                database: "postgres".to_owned(),
                user: "postgres".to_owned(),
            })
        );
    }

    #[test]
    fn round_trips_an_existing_data_source() {
        let data_source = DataSource {
            id: DataSourceId::generate(),
            name: "chinook".to_owned(),
            config: ConnectionConfig::Sqlite(SqliteConfig {
                path: "/data/chinook.db".into(),
                read_only: true,
            }),
        };

        let form = DataSourceForm::from_data_source(&data_source);

        assert_eq!(form.to_data_source(data_source.id), Ok(data_source));
    }

    #[test]
    fn only_a_typed_password_replaces_the_saved_one() {
        let mut form = DataSourceForm::new(DriverKind::Postgres);
        assert_eq!(form.password_update(), PasswordUpdate::Keep);

        form.password = "s3cret".to_owned();

        assert_eq!(
            form.password_update(),
            PasswordUpdate::Set(Password::new("s3cret"))
        );
    }
}
