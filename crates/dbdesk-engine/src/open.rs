//! Opening connections: resolving the password, then dispatching to the right driver.

use std::sync::Arc;
use std::time::Duration;

use dbdesk_core::{
    Connection, ConnectionConfig, DataSource, DriverError, Password, PlaceholderError, ServerInfo,
    describe_error,
};
use dbdesk_store::{SecretError, SecretStore};
use tokio::task::JoinError;

use crate::environment::ProcessEnvironment;
use crate::events::EventSink;
use crate::{DatabaseFile, Event, PasswordUpdate};

/// Gives up on a connection attempt after this long. Long enough for a slow VPN or a cold
/// cloud database to answer, short enough that a wrong host doesn't leave the UI showing
/// "connecting" for minutes (TCP's own timeout can exceed two minutes).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("could not read the saved password")]
    Password(#[source] SecretError),
    #[error(transparent)]
    Placeholder(#[from] PlaceholderError),
    #[error(transparent)]
    Driver(#[from] DriverError),
    #[error("could not connect within {0:?}")]
    TimedOut(Duration),
    #[error("a background task failed")]
    Task(#[source] JoinError),
}

pub(crate) async fn open(
    data_source: &DataSource,
    secrets: &Arc<dyn SecretStore>,
    password: &PasswordUpdate,
) -> Result<Box<dyn Connection>, OpenError> {
    let config = data_source.config.resolve(&ProcessEnvironment)?;
    let password = resolve_password(data_source, secrets, password).await?;
    tokio::time::timeout(CONNECT_TIMEOUT, connect(&config, password.as_ref()))
        .await
        .map_err(|_| OpenError::TimedOut(CONNECT_TIMEOUT))?
        .map_err(OpenError::Driver)
}

async fn resolve_password(
    data_source: &DataSource,
    secrets: &Arc<dyn SecretStore>,
    password: &PasswordUpdate,
) -> Result<Option<Password>, OpenError> {
    match password {
        PasswordUpdate::Set(password) => Ok(Some(password.clone())),
        PasswordUpdate::Keep if !data_source.config.driver().uses_password() => Ok(None),
        PasswordUpdate::Keep => {
            let secrets = Arc::clone(secrets);
            let id = data_source.id;
            tokio::task::spawn_blocking(move || secrets.password(id))
                .await
                .map_err(OpenError::Task)?
                .map_err(OpenError::Password)
        }
    }
}

/// Opens a connection to a saved data source with its saved password, if it needs one.
///
/// For other front ends than the app's UI, such as `dbdesk mcp`, so every way in connects
/// through the same drivers, keychain and timeout.
///
/// # Errors
///
/// [`OpenError`] if the password can't be read, the driver fails, or connecting takes longer
/// than the connect timeout.
pub async fn open_data_source(
    data_source: &DataSource,
    secrets: &Arc<dyn SecretStore>,
) -> Result<Box<dyn Connection>, OpenError> {
    open(data_source, secrets, &PasswordUpdate::Keep).await
}

/// The one place that maps a configuration to its driver.
async fn connect(
    config: &ConnectionConfig,
    password: Option<&Password>,
) -> Result<Box<dyn Connection>, DriverError> {
    Ok(match config {
        ConnectionConfig::Postgres(config) => {
            Box::new(dbdesk_driver_postgres::connect(config, password).await?)
        }
        ConnectionConfig::Sqlite(config) => Box::new(dbdesk_driver_sqlite::connect(config).await?),
        ConnectionConfig::Mysql(config) => {
            Box::new(dbdesk_driver_mysql::connect(config, password).await?)
        }
        ConnectionConfig::SqlServer(config) => {
            Box::new(dbdesk_driver_sqlserver::connect(config, password).await?)
        }
        ConnectionConfig::DuckDb(config) => Box::new(dbdesk_driver_duckdb::connect(config).await?),
        ConnectionConfig::ClickHouse(config) => {
            Box::new(dbdesk_driver_clickhouse::connect(config, password).await?)
        }
    })
}

/// Connects, pings and disconnects, then reports the outcome as [`Event::ConnectionTested`].
pub(crate) async fn test_connection(
    data_source: DataSource,
    password: PasswordUpdate,
    secrets: Arc<dyn SecretStore>,
    events: EventSink,
) {
    tracing::info!(data_source = %data_source.name, "testing connection to {}", data_source.config);
    let result = open_ping_close(&data_source, &secrets, &password).await;
    match &result {
        Ok(server) => tracing::info!(
            data_source = %data_source.name,
            "test connection succeeded: {server}"
        ),
        Err(error) => tracing::warn!(
            data_source = %data_source.name,
            "test connection failed: {}",
            describe_error(error)
        ),
    }
    events.send(Event::ConnectionTested {
        id: data_source.id,
        result: result.map_err(|error| describe_error(&error)),
    });
}

async fn open_ping_close(
    data_source: &DataSource,
    secrets: &Arc<dyn SecretStore>,
    password: &PasswordUpdate,
) -> Result<ServerInfo, OpenError> {
    let mut connection = open(data_source, secrets, password).await?;
    connection.ping().await?;
    let server = connection.server_info().clone();
    connection.close().await?;
    Ok(server)
}

/// Creates an empty database file, then reports it as [`Event::DatabaseFileCreated`].
pub(crate) async fn create_database_file(file: DatabaseFile, events: EventSink) {
    let result = match &file {
        DatabaseFile::Sqlite(path) => dbdesk_driver_sqlite::create_database(path).await,
        DatabaseFile::DuckDb(path) => dbdesk_driver_duckdb::create_database(path).await,
    };
    let engine = file.driver().display_name();
    let path = file.path().display();
    match &result {
        Ok(()) => tracing::info!("created {engine} database {path}"),
        Err(error) => tracing::warn!(
            "could not create {engine} database {path}: {}",
            describe_error(error)
        ),
    }
    events.send(Event::DatabaseFileCreated {
        file,
        result: result.map_err(|error| describe_error(&error)),
    });
}
