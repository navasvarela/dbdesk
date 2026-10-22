//! Helpers shared by the engine's integration tests.

// Test helpers panic on failure like assertions do; clippy's test exemption only covers
// `#[test]` functions themselves, not helpers in an integration test crate.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use dbdesk_core::{ConnectionConfig, DataSource, DataSourceId, Password, SqliteConfig};
use dbdesk_engine::{Command, EngineConfig, EngineHandle, Event, Repaint, SessionState};
use dbdesk_store::{ConsoleFiles, DataSourceFile, SecretError, SecretStore};

pub const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<DataSourceId, Password>>);

impl MemorySecrets {
    pub fn get(&self, id: DataSourceId) -> Option<Password> {
        self.0.lock().unwrap().get(&id).cloned()
    }
}

impl SecretStore for MemorySecrets {
    fn password(&self, id: DataSourceId) -> Result<Option<Password>, SecretError> {
        Ok(self.get(id))
    }

    fn set_password(&self, id: DataSourceId, password: &Password) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(id, password.clone());
        Ok(())
    }

    fn delete_password(&self, id: DataSourceId) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(&id);
        Ok(())
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
        let (sender, repaints) = mpsc::channel();
        let dir = data_sources_file.parent().unwrap_or(Path::new("."));
        let config = EngineConfig {
            data_sources: DataSourceFile::new(data_sources_file),
            secrets,
            consoles: ConsoleFiles::new(dir.join("consoles")),
        };
        let engine = EngineHandle::start(config, SignalRepaint(sender)).unwrap();
        Self { engine, repaints }
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
        self.expect(Event::Started);
        self.expect(Event::DataSourcesLoaded(data_sources));
        self.expect(Event::ConsolesLoaded(vec![]));
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
        config: ConnectionConfig::Sqlite(SqliteConfig {
            path,
            read_only: false,
        }),
    }
}
