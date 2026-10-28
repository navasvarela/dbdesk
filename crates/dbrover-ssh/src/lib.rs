//! SSH tunnels: a port on this machine that an SSH server forwards to a database server, for
//! databases only a jump host can reach. A driver connects to [`Tunnel::local_addr`] instead
//! of the database's own address.
//!
//! The server's host key is checked against OpenSSH's `known_hosts` before anything is sent:
//! an unknown key fails with [`SshError::UnknownHostKey`], so the caller can ask the user and
//! [`trust`] it, and a changed or revoked one is always refused.

mod known_hosts;
mod login;
#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::fmt;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use dbrover_core::{Password, SshConfig};
use russh::client::{self, Handle};
use russh::keys::{Algorithm, HashAlg, PublicKey, PublicKeyOrCertificate};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

use known_hosts::{KnownHosts, Verdict};
use login::Credentials;

/// How often an idle tunnel asks the SSH server whether it is still there, and how many
/// unanswered asks close it. Three missed 15-second keepalives notice a dead jump host in
/// under a minute, where TCP alone can take many minutes.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const KEEPALIVE_MAX: usize = 3;

/// Where to open a tunnel, and to where.
#[derive(Debug, Clone, Copy)]
pub struct TunnelRequest<'a> {
    /// With every placeholder resolved.
    pub ssh: &'a SshConfig,
    /// The SSH password or the key's passphrase, if the login needs one.
    pub secret: Option<&'a Password>,
    /// The database server, as the SSH server reaches it.
    pub target_host: &'a str,
    pub target_port: u16,
    /// OpenSSH's `known_hosts`, normally `~/.ssh/known_hosts`.
    pub known_hosts: &'a Path,
}

/// The secret an SSH login lacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretNeeded {
    Password,
    Passphrase,
}

impl fmt::Display for SecretNeeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Password => "the SSH password",
            Self::Passphrase => "the passphrase of the SSH key",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("{0} is needed and none is saved")]
    SecretNeeded(SecretNeeded),
    #[error("could not use the SSH key {}", path.display())]
    KeyFile {
        path: PathBuf,
        #[source]
        source: russh::keys::Error,
    },
    #[error("could not read {}", path.display())]
    KnownHosts {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not reach the SSH server {server}")]
    Connect {
        server: String,
        #[source]
        source: io::Error,
    },
    #[error(
        "the SSH server {} is not known yet: its {} key's fingerprint is {}",
        .0.server(), .0.algorithm(), .0.fingerprint()
    )]
    UnknownHostKey(HostKey),
    #[error(
        "the SSH server {}'s host key has changed since it was trusted, so someone may be \
         impersonating it; refusing to connect. Its {} key's fingerprint is now {}. If the \
         server's key really changed, remove line {line} of {}",
        key.server(), key.algorithm(), key.fingerprint(), path.display()
    )]
    HostKeyChanged {
        key: HostKey,
        path: PathBuf,
        line: usize,
    },
    #[error(
        "the SSH server {}'s host key is revoked (line {line} of {}); refusing to connect",
        key.server(), path.display()
    )]
    HostKeyRevoked {
        key: HostKey,
        path: PathBuf,
        line: usize,
    },
    #[error("the SSH connection failed")]
    Handshake(#[source] russh::Error),
    #[error("the SSH server refused to sign in {user} with the {method}")]
    LoginRefused { user: String, method: &'static str },
    #[error("could not use the SSH agent: {0}")]
    Agent(String),
    #[error("could not open a local port for the SSH tunnel")]
    Listen(#[source] io::Error),
}

/// An SSH server's host key, with the server that presented it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKey {
    host: String,
    port: u16,
    /// Boxed: a key is several hundred bytes, which every `Result` carrying an [`SshError`]
    /// would otherwise pay for.
    key: Box<PublicKey>,
}

impl HostKey {
    /// `host`, or `host:port` when not on port 22.
    pub fn server(&self) -> String {
        if self.port == 22 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// As `ssh` shows it: `SHA256:` and the base64 of the key's SHA-256 hash.
    pub fn fingerprint(&self) -> String {
        self.key.fingerprint(HashAlg::Sha256).to_string()
    }

    /// The key's type, such as `ssh-ed25519`.
    pub fn algorithm(&self) -> String {
        self.key.algorithm().to_string()
    }
}

/// Adds `key` to `known_hosts`, so the next tunnel to its server trusts it.
///
/// # Errors
///
/// The I/O error if the file (or its folder) can't be written.
pub fn trust(known_hosts: &Path, key: &HostKey) -> io::Result<()> {
    known_hosts::append(known_hosts, &key.host, key.port, &key.key)
}

/// A local port forwarded to the target through the SSH server, open until dropped.
pub struct Tunnel {
    local_addr: SocketAddr,
    session: Arc<Handle<ServerCheck>>,
    forwarding: JoinHandle<()>,
    forward_error: Arc<Mutex<Option<String>>>,
}

impl Tunnel {
    /// Connects to the SSH server, checks its host key, signs in, and starts forwarding
    /// connections made to a new port on `127.0.0.1`.
    ///
    /// # Errors
    ///
    /// [`SshError`] saying which step failed. A key the user may choose to trust is
    /// [`SshError::UnknownHostKey`].
    pub async fn open(request: TunnelRequest<'_>) -> Result<Self, SshError> {
        let ssh = request.ssh;
        let port = ssh.port.number().ok_or_else(|| SshError::Connect {
            server: ssh.to_string(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "unresolved port"),
        })?;
        let credentials = Credentials::prepare(&ssh.login, request.secret).await?;
        let known = KnownHosts::read(request.known_hosts, &ssh.host, port).map_err(|source| {
            SshError::KnownHosts {
                path: request.known_hosts.to_owned(),
                source,
            }
        })?;
        let mut session = handshake(ssh, port, known, request.known_hosts).await?;
        login::sign_in(&mut session, &ssh.user, credentials).await?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(SshError::Listen)?;
        let local_addr = listener.local_addr().map_err(SshError::Listen)?;
        let session = Arc::new(session);
        let forward_error = Arc::default();
        let target = Target {
            host: request.target_host.to_owned(),
            port: request.target_port,
        };
        tracing::info!("SSH tunnel through {ssh} to {target} listening on {local_addr}");
        let forwarding = tokio::spawn(forward(
            listener,
            Arc::clone(&session),
            target,
            Arc::clone(&forward_error),
        ));
        Ok(Self {
            local_addr,
            session,
            forwarding,
            forward_error,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Whether the SSH connection has ended, so the tunnel no longer forwards anything.
    pub fn is_closed(&self) -> bool {
        self.session.is_closed() || self.forwarding.is_finished()
    }

    /// Why the SSH server last refused to forward a connection, if it did. A refused forward
    /// only shows as a closed connection to the driver; this says why.
    pub fn forward_error(&self) -> Option<String> {
        lock(&self.forward_error).clone()
    }
}

impl fmt::Debug for Tunnel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tunnel")
            .field("local_addr", &self.local_addr)
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl Drop for Tunnel {
    /// Stops forwarding, which closes the forwarded connections, and lets the SSH session
    /// end with the last handle to it.
    fn drop(&mut self) {
        self.forwarding.abort();
    }
}

/// Connects and runs the key exchange, accepting only a host key `known_hosts` trusts.
async fn handshake(
    ssh: &SshConfig,
    port: u16,
    known: KnownHosts,
    known_hosts: &Path,
) -> Result<Handle<ServerCheck>, SshError> {
    let server = format!("{}:{port}", ssh.host);
    let stream = TcpStream::connect((ssh.host.as_str(), port))
        .await
        .map_err(|source| SshError::Connect {
            server: server.clone(),
            source,
        })?;
    let config = client::Config {
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        keepalive_max: KEEPALIVE_MAX,
        inactivity_timeout: None,
        nodelay: true,
        preferred: russh::Preferred {
            key: preferred_host_keys(&known.trusted_algorithms()),
            ..russh::Preferred::default()
        },
        ..client::Config::default()
    };
    let problem = Arc::new(Mutex::new(None));
    let check = ServerCheck {
        host: ssh.host.clone(),
        port,
        known,
        problem: Arc::clone(&problem),
    };
    match client::connect_stream(Arc::new(config), stream, check).await {
        Ok(session) => Ok(session),
        Err(error) => Err(match lock(&problem).take() {
            Some(problem) => problem.into_error(known_hosts),
            None => SshError::Handshake(error),
        }),
    }
}

/// The host key types to ask for: those `known_hosts` trusts for the server, as `ssh` does,
/// so a server that has keys of several types presents the trusted one rather than a type
/// that would look unknown. Every supported type when none is trusted yet.
fn preferred_host_keys(trusted: &[Algorithm]) -> Cow<'static, [Algorithm]> {
    let all = russh::Preferred::default().key;
    let preferred: Vec<Algorithm> = all
        .iter()
        .filter(|offered| {
            trusted
                .iter()
                .any(|trusted| trusted == *offered || (is_rsa(trusted) && is_rsa(offered)))
        })
        .cloned()
        .collect();
    if preferred.is_empty() {
        all
    } else {
        Cow::Owned(preferred)
    }
}

/// RSA keys sign with SHA-1, SHA-256 or SHA-512, each a host key algorithm of its own, so
/// one trusted RSA key allows all three.
fn is_rsa(algorithm: &Algorithm) -> bool {
    matches!(algorithm, Algorithm::Rsa { .. })
}

/// The handshake's host key check. Keeps why it refused a key, since russh only reports that
/// it did.
#[derive(Debug)]
pub(crate) struct ServerCheck {
    host: String,
    port: u16,
    known: KnownHosts,
    problem: Arc<Mutex<Option<HostKeyProblem>>>,
}

#[derive(Debug)]
enum HostKeyProblem {
    Unknown(HostKey),
    Changed(HostKey, usize),
    Revoked(HostKey, usize),
    /// Never asked for, so never expected; refused rather than trusted unchecked.
    Certificate,
}

impl HostKeyProblem {
    fn into_error(self, path: &Path) -> SshError {
        let path = path.to_owned();
        match self {
            Self::Unknown(key) => SshError::UnknownHostKey(key),
            Self::Changed(key, line) => SshError::HostKeyChanged { key, path, line },
            Self::Revoked(key, line) => SshError::HostKeyRevoked { key, path, line },
            Self::Certificate => SshError::Handshake(russh::Error::UnknownKey),
        }
    }
}

impl client::Handler for ServerCheck {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        presented: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = presented else {
            *lock(&self.problem) = Some(HostKeyProblem::Certificate);
            return Ok(false);
        };
        let host_key = || HostKey {
            host: self.host.clone(),
            port: self.port,
            key: Box::new(key.clone()),
        };
        let problem = match self.known.check(key) {
            Verdict::Trusted => return Ok(true),
            Verdict::Unknown => HostKeyProblem::Unknown(host_key()),
            Verdict::Changed { line } => HostKeyProblem::Changed(host_key(), line),
            Verdict::Revoked { line } => HostKeyProblem::Revoked(host_key(), line),
        };
        *lock(&self.problem) = Some(problem);
        Ok(false)
    }
}

/// The database server, as the SSH server reaches it.
#[derive(Debug, Clone)]
struct Target {
    host: String,
    port: u16,
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// Accepts connections on the tunnel's port and pipes each through its own SSH channel,
/// until aborted. Aborting drops the set of pipes, which closes them.
async fn forward(
    listener: TcpListener,
    session: Arc<Handle<ServerCheck>>,
    target: Target,
    forward_error: Arc<Mutex<Option<String>>>,
) {
    let mut pipes = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((socket, peer)) => {
                    pipes.spawn(pipe(
                        Arc::clone(&session),
                        socket,
                        peer,
                        target.clone(),
                        Arc::clone(&forward_error),
                    ));
                }
                Err(error) => tracing::warn!("the SSH tunnel to {target} could not accept: {error}"),
            },
            Some(_) = pipes.join_next() => {}
        }
    }
}

async fn pipe(
    session: Arc<Handle<ServerCheck>>,
    mut socket: TcpStream,
    peer: SocketAddr,
    target: Target,
    forward_error: Arc<Mutex<Option<String>>>,
) {
    let channel = session
        .channel_open_direct_tcpip(
            target.host.clone(),
            u32::from(target.port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        )
        .await;
    let channel = match channel {
        Ok(channel) => channel,
        Err(error) => {
            let reason = format!("the SSH server could not forward to {target}: {error}");
            tracing::warn!("{reason}");
            *lock(&forward_error) = Some(reason);
            return;
        }
    };
    let mut stream = channel.into_stream();
    if let Err(error) = tokio::io::copy_bidirectional(&mut socket, &mut stream).await {
        tracing::debug!("a connection through the SSH tunnel to {target} ended: {error}");
    }
}

/// A panic while holding one of these locks can't leave its `Option` half-written.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
