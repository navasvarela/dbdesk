//! Opening connections: resolving the password, then dispatching to the right driver.

use std::sync::Arc;
use std::time::Duration;

use dbdesk_core::{
    Connection, ConnectionConfig, DataSource, DriverError, Password, PlaceholderError, ServerInfo,
    describe_error,
};
use dbdesk_store::{Secret, SecretError, SecretStore};
use tokio::task::JoinError;

use crate::environment::ProcessEnvironment;
use crate::events::EventSink;
use crate::passwords::{self, PasswordNeeded, Prompting};
use crate::{DatabaseFile, Event, PasswordUpdate};

/// Gives up on a connection attempt after this long. Long enough for a slow VPN or a cold
/// cloud database to answer, short enough that a wrong host doesn't leave the UI showing
/// "connecting" for minutes (TCP's own timeout can exceed two minutes).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("could not read the saved password")]
    Password(#[source] SecretError),
    #[error("a password is needed: {0}")]
    PasswordNeeded(PasswordNeeded),
    #[error(transparent)]
    Placeholder(#[from] PlaceholderError),
    #[error(
        "the MCP login {user:?} has no saved password; set it in the data source's settings in \
         dbdesk"
    )]
    NoMcpPassword { user: String },
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
    prompting: Prompting<'_>,
) -> Result<Box<dyn Connection>, OpenError> {
    let config = data_source.config.resolve(&ProcessEnvironment)?;
    let password = passwords::resolve(data_source, secrets, password, prompting).await?;
    tokio::time::timeout(CONNECT_TIMEOUT, connect(&config, password.as_ref()))
        .await
        .map_err(|_| OpenError::TimedOut(CONNECT_TIMEOUT))?
        .map_err(OpenError::Driver)
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
    open(
        data_source,
        secrets,
        &PasswordUpdate::Keep,
        Prompting::Never,
    )
    .await
}

/// Opens a connection for `dbdesk mcp`: as the data source's MCP login when it has one,
/// with that login's own saved password, and otherwise as [`open_data_source`] does.
///
/// An MCP login without a saved password is an error rather than a reason to fall back to
/// the data source's own login, whose grants are what the MCP login exists to narrow.
///
/// # Errors
///
/// [`OpenError`] as for [`open_data_source`], and [`OpenError::NoMcpPassword`].
pub async fn open_data_source_for_mcp(
    data_source: &DataSource,
    secrets: &Arc<dyn SecretStore>,
) -> Result<Box<dyn Connection>, OpenError> {
    let Some(login) = &data_source.mcp_login else {
        return open_data_source(data_source, secrets).await;
    };
    let Some(config) = data_source.config.with_user(&login.user) else {
        return open_data_source(data_source, secrets).await;
    };
    let id = data_source.id;
    let store = Arc::clone(secrets);
    let password = tokio::task::spawn_blocking(move || store.get(id, Secret::McpLogin))
        .await
        .map_err(OpenError::Task)?
        .map_err(OpenError::Password)?
        .ok_or_else(|| OpenError::NoMcpPassword {
            user: login.user.clone(),
        })?;
    let as_mcp = DataSource {
        config,
        ..data_source.clone()
    };
    open(
        &as_mcp,
        secrets,
        &PasswordUpdate::Set(password),
        Prompting::Never,
    )
    .await
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
    let mut connection = open(data_source, secrets, password, Prompting::Never).await?;
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
