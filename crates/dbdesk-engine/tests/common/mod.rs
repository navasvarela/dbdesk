//! Helpers shared by the engine's integration tests.

// Test helpers panic on failure like assertions do; clippy's test exemption only covers
// `#[test]` functions themselves, not helpers in an integration test crate.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use std::str::FromStr;

use dbdesk_core::{
    ClickHouseConfig, ConnectionConfig, DataSource, DataSourceId, MysqlConfig, Password,
    PostgresConfig, ServerConfig, SqlServerConfig, SqliteConfig, TlsMode,
};
use dbdesk_engine::{
    Command, EngineConfig, EngineHandle, Event, InstallError, InstallSupport, Release,
    ReleaseError, ReleaseFeed, Repaint, SessionState, UpdateInstaller,
};
use dbdesk_store::{
    ConsoleFiles, DataSourceFile, HistoryFile, KeymapBindings, KeymapFile, ProjectFile, Projects,
    SecretError, SecretStore, Settings, SettingsFile,
};

pub const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a test lets the engine take to stop. Far longer than the app's own two seconds:
/// what matters here is that it stops and saves, not how fast, and a busy Windows runner has
/// taken over two seconds to close the history database.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct MemorySecrets {
    passwords: Mutex<HashMap<DataSourceId, Password>>,
    /// Refuses every read and write, like a keychain that is locked or missing.
    locked: AtomicBool,
}

impl MemorySecrets {
    pub fn locked() -> Self {
        Self {
            locked: AtomicBool::new(true),
            ..Self::default()
        }
    }

    pub fn get(&self, id: DataSourceId) -> Option<Password> {
        self.passwords.lock().unwrap().get(&id).cloned()
    }

    fn check_unlocked(&self) -> Result<(), SecretError> {
        if self.locked.load(Ordering::Relaxed) {
            return Err(SecretError::unavailable("the keychain is locked"));
        }
        Ok(())
    }
}

impl SecretStore for MemorySecrets {
    fn password(&self, id: DataSourceId) -> Result<Option<Password>, SecretError> {
        self.check_unlocked()?;
        Ok(self.get(id))
    }

    fn set_password(&self, id: DataSourceId, password: &Password) -> Result<(), SecretError> {
        self.check_unlocked()?;
        self.passwords.lock().unwrap().insert(id, password.clone());
        Ok(())
    }

    fn delete_password(&self, id: DataSourceId) -> Result<(), SecretError> {
        self.check_unlocked()?;
        self.passwords.lock().unwrap().remove(&id);
        Ok(())
    }
}

/// A release feed that answers without a network: with this release, or that there is none.
pub struct FixedReleases(pub Option<Release>);

impl ReleaseFeed for FixedReleases {
    fn latest(&self) -> Result<Release, ReleaseError> {
        self.0.clone().ok_or(ReleaseError::NoReleases)
    }
}

/// An installer that pretends: it reports a download in three steps, then installs
/// `Installs(version)` or fails with `Fails(reason)`, touching nothing.
pub enum FakeInstaller {
    Installs(&'static str),
    Fails(&'static str),
}

impl UpdateInstaller for FakeInstaller {
    fn support(&self) -> InstallSupport {
        InstallSupport::Supported
    }

    fn install(&self, progress: &dyn Fn(u64, Option<u64>)) -> Result<String, InstallError> {
        for downloaded in [0, 50, 100] {
            progress(downloaded, Some(100));
        }
        match self {
            Self::Installs(version) => Ok((*version).to_owned()),
            Self::Fails(reason) => Err(InstallError::Unsupported((*reason).to_owned())),
        }
    }
}

pub struct SignalRepaint(mpsc::Sender<()>);

impl Repaint for SignalRepaint {
    fn request_repaint(&self) {
        // The test may already have dropped the receiver; that's fine.
        let _ = self.0.send(());
    }
}

pub struct TestEngine {
    pub engine: EngineHandle,
    repaints: mpsc::Receiver<()>,
}

impl TestEngine {
    pub fn start(data_sources_file: &Path, secrets: Arc<MemorySecrets>) -> Self {
        Self::start_with_releases(data_sources_file, secrets, FixedReleases(None))
    }

    pub fn start_with_releases(
        data_sources_file: &Path,
        secrets: Arc<MemorySecrets>,
        releases: FixedReleases,
    ) -> Self {
        Self::start_with(
            data_sources_file,
            secrets,
            releases,
            FakeInstaller::Installs("9.9.9"),
        )
    }

    pub fn start_with_installer(data_sources_file: &Path, installer: FakeInstaller) -> Self {
        Self::start_with(
            data_sources_file,
            Arc::default(),
            FixedReleases(None),
            installer,
        )
    }

    fn start_with(
        data_sources_file: &Path,
        secrets: Arc<MemorySecrets>,
        releases: FixedReleases,
        installer: FakeInstaller,
    ) -> Self {
        let (sender, repaints) = mpsc::channel();
        let dir = data_sources_file.parent().unwrap_or(Path::new("."));
        let config = EngineConfig {
            data_sources: DataSourceFile::new(data_sources_file),
            projects: ProjectFile::new(dir.join("projects.toml")),
            secrets,
            consoles: ConsoleFiles::new(dir.join("consoles")),
            history: HistoryFile::new(dir.join("history.sqlite")),
            keymap: KeymapFile::new(dir.join("keymap.toml")),
            settings: SettingsFile::new(dir.join("settings.toml")),
            releases: Arc::new(releases),
            updater: Arc::new(installer),
        };
        let engine = EngineHandle::start(config, SignalRepaint(sender)).unwrap();
        Self { engine, repaints }
    }

    /// Stops the engine, waiting for it to save everything first.
    pub fn shut_down(self) {
        self.engine.shutdown_within(SHUTDOWN_TIMEOUT).unwrap();
    }

    pub fn send(&self, command: Command) {
        self.engine.send(command).unwrap();
    }

    pub fn next_event(&mut self) -> Event {
        loop {
            if let Some(event) = self.engine.try_recv().unwrap() {
                return event;
            }
            self.repaints
                .recv_timeout(EVENT_TIMEOUT)
                .expect("timed out waiting for an event");
        }
    }

    pub fn expect(&mut self, expected: Event) {
        assert_eq!(self.next_event(), expected);
    }

    /// Expects the start-up events of an engine with no saved consoles.
    pub fn expect_started(&mut self, data_sources: Vec<DataSource>) {
        self.expect_started_with_projects(Projects::default(), data_sources);
    }

    /// Expects the start-up events of an engine with these projects and no saved consoles.
    pub fn expect_started_with_projects(
        &mut self,
        projects: Projects,
        data_sources: Vec<DataSource>,
    ) {
        self.expect(Event::Started);
        self.expect_keymap(Ok(KeymapBindings::new()));
        self.expect_settings(Ok(Settings::default()));
        self.expect(Event::ProjectsLoaded(projects));
        self.expect(Event::DataSourcesLoaded(data_sources));
        self.expect(Event::ConsolesLoaded(vec![]));
    }

    pub fn expect_keymap(&mut self, expected: Result<KeymapBindings, String>) {
        match self.next_event() {
            Event::KeymapLoaded { result, .. } => match (result, expected) {
                (Ok(bindings), Ok(expected)) => assert_eq!(bindings, expected),
                (Err(_), Err(_)) => {}
                (other, expected) => panic!("expected keymap {expected:?}, got {other:?}"),
            },
            other => panic!("expected the keymap, got {other:?}"),
        }
    }

    pub fn expect_settings(&mut self, expected: Result<Settings, String>) {
        match self.next_event() {
            Event::SettingsLoaded { result, .. } => match (result, expected) {
                (Ok(settings), Ok(expected)) => assert_eq!(settings, expected),
                (Err(_), Err(_)) => {}
                (other, expected) => panic!("expected settings {expected:?}, got {other:?}"),
            },
            other => panic!("expected the settings, got {other:?}"),
        }
    }

    pub fn expect_session(&mut self, id: DataSourceId) -> SessionState {
        match self.next_event() {
            Event::SessionChanged {
                id: event_id,
                state,
            } if event_id == id => state,
            other => panic!("expected a session change for {id}, got {other:?}"),
        }
    }
}

pub fn sqlite_database(dir: &Path) -> PathBuf {
    let path = dir.join("app.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        .unwrap();
    path
}

pub fn sqlite_data_source(path: PathBuf) -> DataSource {
    DataSource {
        id: DataSourceId::generate(),
        name: "app".to_owned(),
        color: None,
        safety: dbdesk_core::SafetyLevel::Normal,
        project: None,
        config: ConnectionConfig::Sqlite(SqliteConfig {
            path,
            read_only: false,
        }),
    }
}

/// The server named by `DBDESK_TEST_POSTGRES_URL`, like the Postgres driver tests use, or
/// `None` (with a note) when it is unset, so tests that need it skip.
pub fn postgres_test_server() -> Option<ParsedUrl> {
    let Ok(url) = std::env::var("DBDESK_TEST_POSTGRES_URL") else {
        eprintln!("skipping: DBDESK_TEST_POSTGRES_URL is not set");
        return None;
    };
    Some(parse_test_url(&url))
}

/// The MySQL-family servers named by `DBDESK_TEST_MYSQL_URL` and `DBDESK_TEST_MARIADB_URL`
/// (`mysql://user:password@host:port/database`), as data source settings and passwords.
/// Empty, with a note, when neither is set, so tests that need one skip.
pub fn mysql_test_servers() -> Vec<(MysqlConfig, Password)> {
    let servers: Vec<(MysqlConfig, Password)> =
        ["DBDESK_TEST_MYSQL_URL", "DBDESK_TEST_MARIADB_URL"]
            .into_iter()
            .filter_map(|variable| std::env::var(variable).ok())
            .map(|url| {
                let rest = url.strip_prefix("mysql://").expect("mysql:// URL");
                let (config, password) = parse_server(rest, "3306");
                (MysqlConfig { server: config }, password)
            })
            .collect();
    if servers.is_empty() {
        eprintln!("skipping: neither DBDESK_TEST_MYSQL_URL nor DBDESK_TEST_MARIADB_URL is set");
    }
    servers
}

/// The SQL Server named by `DBDESK_TEST_SQLSERVER_URL`
/// (`sqlserver://user:password@host:port/database`), or `None`, with a note, so tests that
/// need it skip.
pub fn sqlserver_test_server() -> Option<(SqlServerConfig, Password)> {
    let Ok(url) = std::env::var("DBDESK_TEST_SQLSERVER_URL") else {
        eprintln!("skipping: DBDESK_TEST_SQLSERVER_URL is not set");
        return None;
    };
    let rest = url.strip_prefix("sqlserver://").expect("sqlserver:// URL");
    let (server, password) = parse_server(rest, "1433");
    Some((SqlServerConfig { server }, password))
}

pub fn clickhouse_test_server() -> Option<(ClickHouseConfig, Password)> {
    let Ok(url) = std::env::var("DBDESK_TEST_CLICKHOUSE_URL") else {
        eprintln!("skipping: DBDESK_TEST_CLICKHOUSE_URL is not set");
        return None;
    };
    let rest = url
        .strip_prefix("clickhouse://")
        .expect("clickhouse:// URL");
    let (server, password) = parse_server(rest, "8123");
    Some((ClickHouseConfig { server }, password))
}

pub struct ParsedUrl {
    pub config: PostgresConfig,
    pub password: Password,
}

/// Parses `postgres://user:password@host:port/database` without pulling tokio-postgres
/// into this crate's dev-dependencies just for its URL parser.
fn parse_test_url(url: &str) -> ParsedUrl {
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
        .expect("postgres:// URL");
    let (server, password) = parse_server(rest, "5432");
    ParsedUrl {
        config: PostgresConfig { server },
        password,
    }
}

/// `user:password@host[:port]/database`, the part of a URL after its scheme.
fn parse_server(rest: &str, default_port: &str) -> (ServerConfig, Password) {
    let (credentials, rest) = rest.split_once('@').expect("user:password@ in URL");
    let (user, password) = credentials.split_once(':').expect("password in URL");
    let (host_port, database) = rest.split_once('/').expect("/database in URL");
    let (host, port) = host_port
        .split_once(':')
        .unwrap_or((host_port, default_port));
    let server = ServerConfig {
        host: host.to_owned(),
        port: u16::from_str(port).expect("numeric port").into(),
        database: database.to_owned(),
        user: user.to_owned(),
        tls: TlsMode::default(),
    };
    (server, Password::new(password))
}

/// A port on this machine where every connection is accepted and closed at once, so a
/// connection attempt fails straight away. A port nothing listens on isn't enough: some
/// machines (CI runners among them) drop the attempt instead of refusing it, and it then
/// waits for the connect timeout.
pub fn closing_server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            drop(connection);
        }
    });
    port
}
