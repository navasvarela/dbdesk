//! A data source's TLS mode as `mysql_async`'s TLS options.
//!
//! `mysql_async` has no "prefer": with TLS options set, a server without TLS is an error
//! ([`mysql_async::DriverError::NoClientSslFlagFromServer`]), and without them nothing is
//! encrypted. `connect` gets `prefer` by trying TLS first and, on exactly that error,
//! connecting again without it.

use std::io;
use std::path::{Path, PathBuf};

use dbrover_core::TlsMode;
use mysql_async::SslOpts;

/// What a PEM certificate starts with. A file without one would only fail later, inside the
/// handshake, with a far less helpful message.
const PEM_CERTIFICATE: &str = "-----BEGIN CERTIFICATE-----";

/// Why the TLS settings couldn't be used, before any network traffic. Each message leaves
/// its cause out: `describe_error` appends the source chain.
#[derive(Debug, thiserror::Error)]
pub enum TlsSetupError {
    #[error("could not read the CA file {}", path.display())]
    ReadCaFile { path: PathBuf, source: io::Error },
    #[error("the CA file {} holds no PEM certificate", path.display())]
    NoCertificates { path: PathBuf },
}

/// The TLS options for `tls`, or `None` to connect unencrypted.
pub(crate) fn ssl_opts(tls: &TlsMode) -> Result<Option<SslOpts>, TlsSetupError> {
    Ok(match tls {
        TlsMode::Disable => None,
        // Encrypted, but whoever answers is trusted, as libpq's modes of the same names do.
        TlsMode::Prefer | TlsMode::Require => Some(
            SslOpts::default()
                .with_danger_accept_invalid_certs(true)
                .with_danger_skip_domain_validation(true),
        ),
        TlsMode::VerifyFull { ca_file: None } => Some(SslOpts::default()),
        TlsMode::VerifyFull {
            ca_file: Some(path),
        } => Some(
            SslOpts::default()
                .with_root_certs(vec![read_ca_file(path)?.into()])
                .with_disable_built_in_roots(true),
        ),
    })
}

fn read_ca_file(path: &Path) -> Result<Vec<u8>, TlsSetupError> {
    let contents = std::fs::read(path).map_err(|source| TlsSetupError::ReadCaFile {
        path: path.to_owned(),
        source,
    })?;
    if !String::from_utf8_lossy(&contents).contains(PEM_CERTIFICATE) {
        return Err(TlsSetupError::NoCertificates {
            path: path.to_owned(),
        });
    }
    Ok(contents)
}

/// Whether a failed TLS connection should be retried in plain text: only `prefer` does, and
/// only when the server offers no TLS at all, never when a handshake or a certificate fails.
pub(crate) fn falls_back_to_plain_text(tls: &TlsMode, error: &mysql_async::Error) -> bool {
    *tls == TlsMode::Prefer
        && matches!(
            error,
            mysql_async::Error::Driver(mysql_async::DriverError::NoClientSslFlagFromServer)
        )
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn encrypts_unless_disabled() {
        let modes = [
            (TlsMode::Disable, false),
            (TlsMode::Prefer, true),
            (TlsMode::Require, true),
            (TlsMode::VerifyFull { ca_file: None }, true),
        ];
        for (tls, encrypted) in modes {
            let opts = ssl_opts(&tls).unwrap();
            assert_eq!(opts.is_some(), encrypted, "{tls:?}");
        }
        let checked = ssl_opts(&TlsMode::VerifyFull { ca_file: None })
            .unwrap()
            .unwrap();
        assert!(!checked.accept_invalid_certs() && !checked.skip_domain_validation());
    }

    #[test]
    fn reports_a_ca_file_it_cannot_use() {
        let mut empty = tempfile::NamedTempFile::new().unwrap();
        empty.write_all(b"not a certificate\n").unwrap();
        let cases = [
            (
                PathBuf::from("/nonexistent/dbrover-ca.pem"),
                "could not read the CA file /nonexistent/dbrover-ca.pem",
            ),
            (empty.path().to_owned(), "holds no PEM certificate"),
        ];
        for (path, expected) in cases {
            let tls = TlsMode::VerifyFull {
                ca_file: Some(path),
            };
            let Err(error) = ssl_opts(&tls) else {
                panic!("expected {expected:?}");
            };
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn only_prefer_falls_back_and_only_without_server_tls() {
        let no_tls =
            mysql_async::Error::Driver(mysql_async::DriverError::NoClientSslFlagFromServer);
        let other = mysql_async::Error::Driver(mysql_async::DriverError::ConnectionClosed);

        assert!(falls_back_to_plain_text(&TlsMode::Prefer, &no_tls));
        assert!(!falls_back_to_plain_text(&TlsMode::Require, &no_tls));
        assert!(!falls_back_to_plain_text(&TlsMode::Prefer, &other));
    }
}
