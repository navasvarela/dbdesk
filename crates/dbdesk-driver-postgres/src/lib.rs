//! PostgreSQL driver for dbdesk, built on `tokio-postgres`.
//!
//! Must run inside a tokio runtime: each connection spawns a task that drives the socket.

use async_trait::async_trait;
use dbdesk_core::{BoxError, Connection, DriverError, Password, PostgresConfig, ServerInfo};
use tokio::task::JoinHandle;
use tokio_postgres::{Client, NoTls};

/// Shown in `pg_stat_activity`, so a DBA can tell dbdesk's sessions apart.
const APPLICATION_NAME: &str = "dbdesk";
const PRODUCT: &str = "PostgreSQL";

/// Opens a connection without TLS.
///
/// TODO(juan): support TLS (`sslmode`); servers that require it reject this connection with
/// a clear error for now. Tracked in `docs/roadmap.md` under M1 follow-ups.
///
/// # Errors
///
/// [`DriverError::Connect`] if the server is unreachable, rejects the credentials, or the
/// database doesn't exist.
pub async fn connect(
    config: &PostgresConfig,
    password: Option<&Password>,
) -> Result<PostgresConnection, DriverError> {
    let mut pg_config = tokio_postgres::Config::new();
    pg_config
        .host(&config.host)
        .port(config.port)
        .dbname(&config.database)
        .user(&config.user)
        .application_name(APPLICATION_NAME);
    if let Some(password) = password {
        pg_config.password(password.expose());
    }

    let (client, connection) = pg_config.connect(NoTls).await.map_err(connect_error)?;
    let connection_task = tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::warn!(error = %error, "PostgreSQL connection closed with an error");
        }
    });
    let version = server_version(&client).await.map_err(connect_error)?;

    Ok(PostgresConnection {
        client,
        connection_task,
        server_info: ServerInfo {
            product: PRODUCT.to_owned(),
            version,
        },
    })
}

async fn server_version(client: &Client) -> Result<String, tokio_postgres::Error> {
    client
        .query_one("SHOW server_version", &[])
        .await?
        .try_get(0)
}

fn connect_error(error: tokio_postgres::Error) -> DriverError {
    DriverError::Connect(Box::new(error))
}

fn connection_error(error: impl Into<BoxError>) -> DriverError {
    DriverError::Connection(error.into())
}

pub struct PostgresConnection {
    client: Client,
    /// Drives the socket. Finishes once `client` is dropped and the server has been told
    /// the session is over.
    connection_task: JoinHandle<()>,
    server_info: ServerInfo,
}

#[async_trait]
impl Connection for PostgresConnection {
    fn server_info(&self) -> &ServerInfo {
        &self.server_info
    }

    async fn ping(&mut self) -> Result<(), DriverError> {
        self.client
            .simple_query("SELECT 1")
            .await
            .map(drop)
            .map_err(connection_error)
    }

    async fn close(self: Box<Self>) -> Result<(), DriverError> {
        let Self {
            client,
            connection_task,
            server_info: _,
        } = *self;
        drop(client);
        connection_task.await.map_err(connection_error)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use dbdesk_core::conformance;
    use tokio_postgres::config::Host;

    use super::*;

    /// Points the tests at a server, e.g. `postgres://dbdesk:dbdesk@localhost:55432/dbdesk`
    /// from the repository's `docker-compose.yml`. Tests that need a server are skipped when
    /// it is unset, so `cargo test` works offline.
    const TEST_URL_VAR: &str = "DBDESK_TEST_POSTGRES_URL";

    fn test_server() -> Option<(PostgresConfig, Option<Password>)> {
        let Ok(url) = std::env::var(TEST_URL_VAR) else {
            eprintln!("skipping: {TEST_URL_VAR} is not set");
            return None;
        };
        let parsed = tokio_postgres::Config::from_str(&url).expect("valid test URL");
        let host = match parsed.get_hosts().first() {
            Some(Host::Tcp(host)) => host.clone(),
            other => panic!("test URL needs a TCP host, got {other:?}"),
        };
        let config = PostgresConfig {
            host,
            port: parsed
                .get_ports()
                .first()
                .copied()
                .unwrap_or(PostgresConfig::DEFAULT_PORT),
            database: parsed
                .get_dbname()
                .expect("database in test URL")
                .to_owned(),
            user: parsed.get_user().expect("user in test URL").to_owned(),
        };
        let password = parsed
            .get_password()
            .map(|bytes| Password::new(String::from_utf8_lossy(bytes)));
        Some((config, password))
    }

    #[tokio::test]
    async fn conforms_to_the_connection_lifecycle() {
        let Some((config, password)) = test_server() else {
            return;
        };

        let connection = connect(&config, password.as_ref()).await.unwrap();

        assert_eq!(connection.server_info().product, PRODUCT);
        conformance::check_lifecycle(Box::new(connection)).await;
    }

    #[tokio::test]
    async fn wrong_password_is_a_connect_error() {
        let Some((config, _)) = test_server() else {
            return;
        };

        let result = connect(&config, Some(&Password::new("definitely wrong"))).await;

        conformance::check_connect_failure(result);
    }

    #[tokio::test]
    async fn unreachable_server_is_a_connect_error() {
        let config = PostgresConfig {
            host: "127.0.0.1".to_owned(),
            // Port 1 (tcpmux) is reserved and has no listener on any normal machine, so the
            // connection is refused immediately instead of timing out.
            port: 1,
            database: "postgres".to_owned(),
            user: "postgres".to_owned(),
        };

        conformance::check_connect_failure(connect(&config, None).await);
    }
}
