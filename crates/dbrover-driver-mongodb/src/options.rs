//! The driver's client options from a data source: its server form, or its connection string,
//! with the user and password DbRover keeps apart from the string.

use std::time::Duration;

use dbrover_core::{BoxError, MongoConfig, Password, Route, ServerConfig, TlsMode};
use mongodb::options::{ClientOptions, ServerAddress, Tls, TlsOptions};

/// How long reaching the server may take before connecting fails, rather than the driver's
/// 30 seconds. Chosen, not measured: as long as the other drivers wait for a distant managed
/// server.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// The client options for `config`, through `route`.
///
/// # Errors
///
/// The connection string can't be read (or its `+srv` record looked up), or the data source
/// asks for an SSH tunnel with a connection string, which may name several hosts.
pub(crate) async fn client_options(
    config: &MongoConfig,
    password: Option<&Password>,
    route: Route,
) -> Result<ClientOptions, BoxError> {
    let mut options = match &config.connection_string {
        Some(string) => {
            if matches!(route, Route::Tunnel(_)) {
                return Err("an SSH tunnel needs the server form, not a connection string".into());
            }
            ClientOptions::parse(string).await?
        }
        None => server_options(&config.server, route)?,
    };
    options.app_name = Some("DbRover".to_owned());
    options.connect_timeout = Some(CONNECT_TIMEOUT);
    options.server_selection_timeout = Some(CONNECT_TIMEOUT);
    let user = &config.server.user;
    if !user.is_empty() {
        let mut credential = options.credential.take().unwrap_or_default();
        credential.username = Some(user.clone());
        credential.password = password.map(|password| password.expose().to_owned());
        if !config.auth_database.is_empty() {
            credential.source = Some(config.auth_database.clone());
        } else if credential.source.is_none() && config.connection_string.is_none() {
            credential.source = Some(MongoConfig::DEFAULT_AUTH_DATABASE.to_owned());
        }
        options.credential = Some(credential);
    }
    Ok(options)
}

/// One server, reached directly or through a tunnel's local end. A direct connection, so the
/// driver talks to that server alone: through a tunnel, the replica set's own names for its
/// members would lead past the tunnel.
fn server_options(server: &ServerConfig, route: Route) -> Result<ClientOptions, BoxError> {
    let address = match route {
        Route::Direct => ServerAddress::Tcp {
            host: server.host.clone(),
            port: server.port.number(),
        },
        Route::Tunnel(local) => ServerAddress::Tcp {
            host: local.ip().to_string(),
            port: Some(local.port()),
        },
    };
    let mut options = ClientOptions::default();
    options.hosts = vec![address];
    options.direct_connection = Some(true);
    options.tls = Some(tls(&server.tls, route)?);
    Ok(options)
}

/// The driver's TLS for a data source's mode. MongoDB serves TLS or not on a port, so `prefer`
/// can't try it and fall back: it connects unencrypted, as `disable` does.
///
/// # Errors
///
/// `verify-full` through an SSH tunnel: the driver checks the certificate's name against the
/// address it connects to, the tunnel's local end, and its rustls build has no way to check it
/// against the server's name instead.
fn tls(mode: &TlsMode, route: Route) -> Result<Tls, BoxError> {
    Ok(match mode {
        TlsMode::Disable | TlsMode::Prefer => Tls::Disabled,
        TlsMode::Require => {
            let mut options = TlsOptions::default();
            options.allow_invalid_certificates = Some(true);
            Tls::Enabled(options)
        }
        TlsMode::VerifyFull { .. } if matches!(route, Route::Tunnel(_)) => {
            return Err(
                "verify-full can't check a MongoDB server's name through an SSH \
                        tunnel; choose require, which still encrypts"
                    .into(),
            );
        }
        TlsMode::VerifyFull { ca_file } => {
            let mut options = TlsOptions::default();
            options.ca_file_path.clone_from(ca_file);
            Tls::Enabled(options)
        }
    })
}
