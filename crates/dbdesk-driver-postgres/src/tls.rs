//! The TLS side of a PostgreSQL connection: which `sslmode` to ask the server for, and the
//! rustls connector, configured by `dbdesk-tls`.

use dbdesk_core::TlsMode;
pub use dbdesk_tls::TlsSetupError;
use tokio_postgres::config::SslMode;
use tokio_postgres_rustls::MakeRustlsConnect;

/// What tokio-postgres asks the server for. `verify-full` is `require` at this level: the
/// certificate checks happen in the connector.
pub(crate) fn ssl_mode(tls: &TlsMode) -> SslMode {
    match tls {
        TlsMode::Disable => SslMode::Disable,
        TlsMode::Prefer => SslMode::Prefer,
        TlsMode::Require | TlsMode::VerifyFull { .. } => SslMode::Require,
    }
}

/// The connector for `tls`. Built once per data source session and shared by its connections
/// and its cancel requests, which a TLS-only server also refuses in plain text.
///
/// # Errors
///
/// [`TlsSetupError`] if `verify-full`'s CA file can't be read or holds no certificate.
pub(crate) fn connector(tls: &TlsMode) -> Result<MakeRustlsConnect, TlsSetupError> {
    dbdesk_tls::client_config(tls).map(MakeRustlsConnect::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_the_server_for_tls_as_the_mode_says() {
        let cases = [
            (TlsMode::Disable, SslMode::Disable),
            (TlsMode::Prefer, SslMode::Prefer),
            (TlsMode::Require, SslMode::Require),
            (TlsMode::VerifyFull { ca_file: None }, SslMode::Require),
        ];
        for (tls, expected) in cases {
            assert_eq!(ssl_mode(&tls), expected, "{tls:?}");
            assert!(connector(&tls).is_ok(), "{tls:?}");
        }
    }
}
