//! Tunnels through the `ssh` service in docker-compose.yml, to its `postgres` service. They
//! skip unless `DBDESK_TEST_SSH_URL` and `DBDESK_TEST_SSH_POSTGRES_URL` are set (see
//! test-servers.env); the keys are the committed test keys in docker/ssh.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dbdesk_core::{Password, SshConfig, SshLogin};

use super::*;

const SSH_URL: &str = "DBDESK_TEST_SSH_URL";
const POSTGRES_URL: &str = "DBDESK_TEST_SSH_POSTGRES_URL";

/// The compose services, as the variables describe them.
struct Servers {
    ssh_host: String,
    ssh_port: u16,
    user: String,
    password: String,
    /// The PostgreSQL service as the SSH server reaches it.
    postgres: Url,
}

struct Url {
    user: String,
    password: String,
    host: String,
    port: u16,
    database: String,
}

/// Reads `scheme://user:password@host:port/database`; the test URLs have nothing more.
fn parse_url(url: &str) -> Url {
    let (_, rest) = url.split_once("://").unwrap();
    let (login, rest) = rest.split_once('@').unwrap();
    let (user, password) = login.split_once(':').unwrap();
    let (address, database) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = address.rsplit_once(':').unwrap();
    Url {
        user: user.to_owned(),
        password: password.to_owned(),
        host: host.to_owned(),
        port: port.parse().unwrap(),
        database: database.to_owned(),
    }
}

fn servers() -> Option<Servers> {
    let (Ok(ssh), Ok(postgres)) = (std::env::var(SSH_URL), std::env::var(POSTGRES_URL)) else {
        eprintln!("skipped: set {SSH_URL} and {POSTGRES_URL} (see test-servers.env)");
        return None;
    };
    let ssh = parse_url(&ssh);
    Some(Servers {
        ssh_host: ssh.host,
        ssh_port: ssh.port,
        user: ssh.user,
        password: ssh.password,
        postgres: parse_url(&postgres),
    })
}

fn docker_ssh(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docker/ssh")
        .join(file)
}

/// The test server's host key, which `known_hosts` must hold for it to be trusted.
fn server_host_key() -> PublicKey {
    PublicKey::read_openssh_file(docker_ssh("ssh_host_ed25519_key.pub")).unwrap()
}

/// A `known_hosts` in a temporary folder holding `lines`.
fn known_hosts(lines: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("known_hosts");
    std::fs::write(&path, lines).unwrap();
    (dir, path)
}

impl Servers {
    fn ssh(&self, login: SshLogin) -> SshConfig {
        SshConfig {
            host: self.ssh_host.clone(),
            port: self.ssh_port.into(),
            user: self.user.clone(),
            login,
        }
    }

    fn key_login(&self) -> SshConfig {
        self.ssh(SshLogin::KeyFile {
            key_file: docker_ssh("client_ed25519"),
        })
    }

    /// The `known_hosts` line that trusts the test server.
    fn trusted_line(&self) -> String {
        format!(
            "[{}]:{} {}\n",
            self.ssh_host,
            self.ssh_port,
            server_host_key().to_openssh().unwrap()
        )
    }

    async fn open(
        &self,
        ssh: &SshConfig,
        secret: Option<&Password>,
        known_hosts: &Path,
    ) -> Result<Tunnel, SshError> {
        Tunnel::open(TunnelRequest {
            ssh,
            secret,
            target_host: &self.postgres.host,
            target_port: self.postgres.port,
            known_hosts,
        })
        .await
    }

    /// Runs a query on the PostgreSQL service through `tunnel`.
    async fn query_through(&self, tunnel: &Tunnel) -> i32 {
        let postgres = &self.postgres;
        let (client, connection) = tokio_postgres::Config::new()
            .host("127.0.0.1")
            .port(tunnel.local_addr().port())
            .user(&postgres.user)
            .password(&postgres.password)
            .dbname(&postgres.database)
            .connect(tokio_postgres::NoTls)
            .await
            .unwrap();
        tokio::spawn(connection);
        client.query_one("SELECT 42", &[]).await.unwrap().get(0)
    }
}

#[tokio::test]
async fn forwards_to_postgres_after_signing_in_with_a_key() {
    let Some(servers) = servers() else { return };
    let (_dir, known_hosts) = known_hosts(&servers.trusted_line());

    let tunnel = servers
        .open(&servers.key_login(), None, &known_hosts)
        .await
        .unwrap();

    assert_eq!(servers.query_through(&tunnel).await, 42);
    // Each connection gets its own channel.
    assert_eq!(servers.query_through(&tunnel).await, 42);
    assert!(!tunnel.is_closed());
}

#[tokio::test]
async fn signs_in_with_a_password_and_asks_for_one_when_none_is_given() {
    let Some(servers) = servers() else { return };
    let (_dir, known_hosts) = known_hosts(&servers.trusted_line());
    let ssh = servers.ssh(SshLogin::Password);

    let missing = servers.open(&ssh, None, &known_hosts).await.unwrap_err();
    let wrong = servers
        .open(&ssh, Some(&Password::new("not it")), &known_hosts)
        .await
        .unwrap_err();
    let tunnel = servers
        .open(
            &ssh,
            Some(&Password::new(servers.password.clone())),
            &known_hosts,
        )
        .await
        .unwrap();

    assert!(
        matches!(missing, SshError::SecretNeeded(SecretNeeded::Password)),
        "{missing:?}"
    );
    assert!(
        matches!(
            wrong,
            SshError::LoginRefused {
                method: "password",
                ..
            }
        ),
        "{wrong:?}"
    );
    assert_eq!(servers.query_through(&tunnel).await, 42);
}

#[tokio::test]
async fn asks_for_an_encrypted_keys_passphrase_before_connecting() {
    let Some(servers) = servers() else { return };
    let (_dir, known_hosts) = known_hosts(&servers.trusted_line());
    let ssh = servers.ssh(SshLogin::KeyFile {
        key_file: docker_ssh("client_ed25519_encrypted"),
    });

    let missing = servers.open(&ssh, None, &known_hosts).await.unwrap_err();
    let wrong = servers
        .open(&ssh, Some(&Password::new("not it")), &known_hosts)
        .await
        .unwrap_err();
    let tunnel = servers
        .open(&ssh, Some(&Password::new("dbdesk")), &known_hosts)
        .await
        .unwrap();

    assert!(
        matches!(missing, SshError::SecretNeeded(SecretNeeded::Passphrase)),
        "{missing:?}"
    );
    assert!(matches!(wrong, SshError::KeyFile { .. }), "{wrong:?}");
    assert_eq!(servers.query_through(&tunnel).await, 42);
}

#[tokio::test]
async fn an_unknown_host_key_shows_its_fingerprint_and_is_trusted_once_asked() {
    let Some(servers) = servers() else { return };
    let (_dir, known_hosts) = known_hosts("");

    let error = servers
        .open(&servers.key_login(), None, &known_hosts)
        .await
        .unwrap_err();
    let SshError::UnknownHostKey(key) = error else {
        panic!("expected an unknown host key, got {error:?}");
    };
    // What `ssh-keygen -lf docker/ssh/ssh_host_ed25519_key.pub` prints.
    assert_eq!(
        key.fingerprint(),
        server_host_key().fingerprint(HashAlg::Sha256).to_string()
    );
    assert!(key.fingerprint().starts_with("SHA256:"));
    assert_eq!(key.algorithm(), "ssh-ed25519");
    trust(&known_hosts, &key).unwrap();

    let tunnel = servers
        .open(&servers.key_login(), None, &known_hosts)
        .await
        .unwrap();
    assert_eq!(servers.query_through(&tunnel).await, 42);
}

#[tokio::test]
async fn refuses_a_changed_or_revoked_host_key() {
    let Some(servers) = servers() else { return };
    let another_servers_key = std::fs::read_to_string(docker_ssh("authorized_keys")).unwrap();
    let another_servers_key = another_servers_key.lines().next().unwrap();
    let server = format!("[{}]:{}", servers.ssh_host, servers.ssh_port);
    let (_changed_dir, changed) = known_hosts(&format!(
        "# trusted before the key changed\n{server} {another_servers_key}\n"
    ));
    let (_revoked_dir, revoked) = known_hosts(&format!(
        "{}@revoked * {}\n",
        servers.trusted_line(),
        server_host_key().to_openssh().unwrap()
    ));

    let changed_error = servers
        .open(&servers.key_login(), None, &changed)
        .await
        .unwrap_err();
    let revoked_error = servers
        .open(&servers.key_login(), None, &revoked)
        .await
        .unwrap_err();

    let SshError::HostKeyChanged { line, .. } = &changed_error else {
        panic!("expected a changed host key, got {changed_error:?}");
    };
    assert_eq!(*line, 2);
    assert!(
        changed_error.to_string().contains("refusing to connect"),
        "{changed_error}"
    );
    assert!(
        matches!(revoked_error, SshError::HostKeyRevoked { line: 2, .. }),
        "{revoked_error:?}"
    );
}

#[tokio::test]
async fn says_why_the_server_would_not_forward() {
    let Some(servers) = servers() else { return };
    let (_dir, known_hosts) = known_hosts(&servers.trusted_line());
    let ssh = servers.key_login();
    // The server only forwards to postgres:5432 (docker/ssh/sshd_config).
    let tunnel = Tunnel::open(TunnelRequest {
        ssh: &ssh,
        secret: None,
        target_host: "mysql",
        target_port: 3306,
        known_hosts: &known_hosts,
    })
    .await
    .unwrap();

    let connected = TcpStream::connect(tunnel.local_addr()).await;

    assert!(connected.is_ok(), "the local end always accepts");
    let deadline = Instant::now() + Duration::from_secs(10);
    while tunnel.forward_error().is_none() {
        assert!(Instant::now() < deadline, "the refusal was never reported");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let reason = tunnel.forward_error().unwrap_or_default();
    assert!(reason.contains("mysql:3306"), "{reason}");
}

#[tokio::test]
async fn dropping_the_tunnel_closes_its_port() {
    let Some(servers) = servers() else { return };
    let (_dir, known_hosts) = known_hosts(&servers.trusted_line());
    let tunnel = servers
        .open(&servers.key_login(), None, &known_hosts)
        .await
        .unwrap();
    let address = tunnel.local_addr();

    drop(tunnel);

    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(address).await.is_ok() {
        assert!(Instant::now() < deadline, "the port stayed open");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn cannot_reach_a_server_that_is_not_there() {
    let (_dir, known_hosts) = known_hosts("");
    // Nothing listens on port 1 of this machine.
    let ssh = SshConfig {
        host: "127.0.0.1".to_owned(),
        port: 1.into(),
        user: "nobody".to_owned(),
        login: SshLogin::Agent,
    };

    let error = Tunnel::open(TunnelRequest {
        ssh: &ssh,
        secret: None,
        target_host: "db",
        target_port: 5432,
        known_hosts: &known_hosts,
    })
    .await
    .unwrap_err();

    assert!(matches!(error, SshError::Connect { .. }), "{error:?}");
}
