//! The TLS side of a connection: which `sslmode` to ask the server for, and the rustls
//! connector that checks (or deliberately doesn't check) its certificate.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dbdesk_core::TlsMode;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, ring};
use rustls::pki_types::pem::{self, PemObject};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio_postgres::config::SslMode;
use tokio_postgres_rustls::MakeRustlsConnect;

/// Why the TLS settings couldn't be turned into a connector, before any network traffic.
/// Each message leaves its cause out: `describe_error` appends the source chain.
#[derive(Debug, thiserror::Error)]
pub enum TlsSetupError {
    #[error("could not read the CA file {}", path.display())]
    ReadCaFile { path: PathBuf, source: io::Error },
    #[error("the CA file {} is not valid PEM", path.display())]
    InvalidPem { path: PathBuf, source: pem::Error },
    #[error("the CA file {} holds no usable certificate", path.display())]
    NoCertificates { path: PathBuf },
    #[error("could not set up TLS")]
    Rustls(#[from] rustls::Error),
}

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
    let provider = Arc::new(ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?;
    let config = match tls {
        // `disable` never starts a handshake, so the verifier is never asked; `prefer` and
        // `require` encrypt without authenticating the server, as they do in libpq.
        TlsMode::Disable | TlsMode::Prefer | TlsMode::Require => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(EncryptOnly {
                algorithms: provider.signature_verification_algorithms,
            }))
            .with_no_client_auth(),
        TlsMode::VerifyFull { ca_file } => builder
            .with_root_certificates(trusted_roots(ca_file.as_deref())?)
            .with_no_client_auth(),
    };
    Ok(MakeRustlsConnect::new(config))
}

/// The authorities in `ca_file`, or the public web ones (Mozilla's list) without one.
fn trusted_roots(ca_file: Option<&Path>) -> Result<RootCertStore, TlsSetupError> {
    let Some(path) = ca_file else {
        return Ok(RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        });
    };
    let certificates = CertificateDer::pem_file_iter(path)
        .map_err(|error| pem_error(path, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| pem_error(path, error))?;
    let mut roots = RootCertStore::empty();
    let (added, _ignored) = roots.add_parsable_certificates(certificates);
    if added == 0 {
        return Err(TlsSetupError::NoCertificates {
            path: path.to_owned(),
        });
    }
    Ok(roots)
}

/// Opening the file fails with an I/O error wrapped in `pem::Error`; it reads better unwrapped.
fn pem_error(path: &Path, error: pem::Error) -> TlsSetupError {
    let path = path.to_owned();
    match error {
        pem::Error::Io(source) => TlsSetupError::ReadCaFile { path, source },
        source => TlsSetupError::InvalidPem { path, source },
    }
}

/// Accepts any certificate, but still checks that the server holds its private key, so the
/// session is encrypted to whoever answered. What libpq's `prefer` and `require` do.
#[derive(Debug)]
struct EncryptOnly {
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for EncryptOnly {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, certificate, signature, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, certificate, signature, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn ca_file_with(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        file
    }

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

    #[test]
    fn reports_a_ca_file_it_cannot_use() {
        let empty = ca_file_with("not a certificate\n");
        let cases = [
            (
                PathBuf::from("/nonexistent/dbdesk-ca.pem"),
                "could not read the CA file /nonexistent/dbdesk-ca.pem",
            ),
            (empty.path().to_owned(), "holds no usable certificate"),
        ];
        for (path, expected) in cases {
            let tls = TlsMode::VerifyFull {
                ca_file: Some(path),
            };
            let Err(error) = connector(&tls) else {
                panic!("expected {expected:?}");
            };
            assert!(error.to_string().contains(expected), "{error}");
        }
    }
}
