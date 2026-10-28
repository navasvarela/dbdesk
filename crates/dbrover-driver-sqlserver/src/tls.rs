//! A data source's TLS mode as `tiberius`'s encryption settings, and the crypto provider they
//! run on.
//!
//! SQL Server negotiates encryption in its pre-login message and always offers it (with a
//! self-signed certificate if it was given none), so `prefer` asks for encryption the way
//! `require` does, without checking the certificate. `disable` sends nothing encrypted, which
//! a server that forces encryption refuses.

use std::io;
use std::path::{Path, PathBuf};

use dbrover_core::TlsMode;
use tiberius::{Config, EncryptionLevel};

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

/// Makes ring rustls's process-wide crypto provider, unless one is installed already.
///
/// `tiberius`'s rustls support compiles in aws-lc-rs as well as the ring every other driver
/// uses, and with two providers compiled in, rustls no longer picks one on its own: both this
/// driver and `mysql_async` take the process-wide one. Installing ring keeps every TLS
/// connection on the same provider, and the MySQL driver does the same before connecting.
pub(crate) fn install_crypto_provider() {
    // An error only means a provider is installed already, which is what was wanted.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Sets `config`'s encryption as `tls` says.
pub(crate) fn configure(config: &mut Config, tls: &TlsMode) -> Result<(), TlsSetupError> {
    match tls {
        TlsMode::Disable => config.encryption(EncryptionLevel::NotSupported),
        // Encrypted, but whoever answers is trusted, as libpq's modes of the same names do.
        TlsMode::Prefer => {
            config.encryption(EncryptionLevel::On);
            config.trust_cert();
        }
        TlsMode::Require => {
            config.encryption(EncryptionLevel::Required);
            config.trust_cert();
        }
        TlsMode::VerifyFull { ca_file: None } => {
            config.encryption(EncryptionLevel::Required);
            config.trust_webpki_roots();
        }
        TlsMode::VerifyFull {
            ca_file: Some(path),
        } => {
            config.encryption(EncryptionLevel::Required);
            config.trust_cert_ca_bundle(read_ca_file(path)?);
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

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
            let Err(error) = configure(&mut Config::new(), &tls) else {
                panic!("expected {expected:?}");
            };
            assert!(error.to_string().contains(expected), "{error}");
        }
    }
}
