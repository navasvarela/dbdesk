//! The byte stream a session runs on: TCP to the server, or to an SSH tunnel's local end, and
//! TLS over it when the data source asks. DbRover opens it rather than the `redis` client, so
//! TLS is `dbrover-tls`'s rustls configuration on ring, as for the other network drivers.

use std::sync::Arc;
use std::time::Duration;

use dbrover_core::{BoxError, Route, ServerConfig, TlsMode};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

/// How long reaching the server and the TLS handshake may take before the attempt fails,
/// rather than waiting for the operating system's own TCP timeout of a minute or more. Chosen,
/// not measured: long enough for a distant managed server.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// A stream the `redis` client can run a connection on.
pub(crate) trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// Opens the stream to `server`, through `route`.
///
/// Redis serves TLS on a port of its own rather than upgrading a plain connection, so `prefer`
/// can't try TLS and fall back on the same port: it connects unencrypted, as `disable` does.
///
/// # Errors
///
/// The server can't be reached in time, the TLS settings can't be used, or the handshake or
/// the certificate check fails.
pub(crate) async fn open(server: &ServerConfig, route: Route) -> Result<Box<dyn Stream>, BoxError> {
    tokio::time::timeout(CONNECT_TIMEOUT, open_now(server, route))
        .await
        .map_err(|_| format!("the server didn't answer within {CONNECT_TIMEOUT:?}"))?
}

async fn open_now(server: &ServerConfig, route: Route) -> Result<Box<dyn Stream>, BoxError> {
    let tcp = match route {
        Route::Direct => TcpStream::connect((server.host.as_str(), server.port_number()?)).await?,
        Route::Tunnel(local) => TcpStream::connect(local).await?,
    };
    // Each command is one small write that waits for its reply: Nagle's algorithm would only
    // delay it.
    tcp.set_nodelay(true)?;
    match &server.tls {
        TlsMode::Disable | TlsMode::Prefer => Ok(Box::new(tcp)),
        tls @ (TlsMode::Require | TlsMode::VerifyFull { .. }) => {
            let config = dbrover_tls::client_config(tls)?;
            // The route may lead to a tunnel's local end; the certificate must name the server.
            let name = ServerName::try_from(server.host.clone())?;
            let stream = TlsConnector::from(Arc::new(config))
                .connect(name, tcp)
                .await?;
            Ok(Box::new(stream))
        }
    }
}
