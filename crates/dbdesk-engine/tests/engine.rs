//! Drives the engine headlessly through its public API: send commands, assert on events.

// Test helpers panic on failure like assertions do; clippy's test exemption only covers
// `#[test]` functions themselves, not helpers in an integration test crate.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::{
    MemorySecrets, TestEngine, postgres_test_server, sqlite_data_source, sqlite_database,
};
use dbdesk_core::{
    CatalogPath, ConnectionConfig, DataSource, DataSourceId, FolderKind, NodeKind, Password,
    PostgresConfig,
};
use dbdesk_engine::{Command, Event, PasswordUpdate, SessionState};
use dbdesk_store::{ConsoleKey, ConsoleName, DataSourceFile, SavedConsole};

#[test]
fn saves_connects_disconnects_and_deletes_a_data_source() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("data_sources.toml");
    let mut engine = TestEngine::start(&file, Arc::default());
    engine.expect_started(vec![]);
    let data_source = sqlite_data_source(sqlite_database(dir.path()));
    let id = data_source.id;

    engine.send(Command::SaveDataSource {
        data_source: data_source.clone(),
        password: PasswordUpdate::Keep,
    });
    engine.expect(Event::DataSourceSaved(data_source.clone()));
    assert_eq!(
        DataSourceFile::new(&file).load().unwrap(),
        vec![data_source]
    );

    engine.send(Command::Connect(id));
    assert_eq!(engine.expect_session(id), SessionState::Connecting);
    let SessionState::Connected(server) = engine.expect_session(id) else {
        panic!("expected to connect");
    };
    assert_eq!(server.product, "SQLite");

    engine.send(Command::Disconnect(id));
    assert_eq!(engine.expect_session(id), SessionState::Disconnected);

    engine.send(Command::DeleteDataSource(id));
    engine.expect(Event::DataSourceDeleted(id));
    assert!(DataSourceFile::new(&file).load().unwrap().is_empty());
    engine.engine.shutdown().unwrap();
}

#[test]
fn reloads_saved_data_sources_and_stores_passwords_in_the_secret_store() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("data_sources.toml");
    let secrets = Arc::new(MemorySecrets::default());
    let data_source = DataSource {
        id: DataSourceId::generate(),
        name: "remote".to_owned(),
        config: ConnectionConfig::Postgres(PostgresConfig {
            host: "db.internal".to_owned(),
            port: 5432,
            database: "app".to_owned(),
            user: "juan".to_owned(),
        }),
    };

    let mut engine = TestEngine::start(&file, Arc::clone(&secrets));
    engine.expect_started(vec![]);
    engine.send(Command::SaveDataSource {
        data_source: data_source.clone(),
        password: PasswordUpdate::Set(Password::new("s3cret")),
    });
    engine.expect(Event::DataSourceSaved(data_source.clone()));
    engine.engine.shutdown().unwrap();

    assert_eq!(secrets.get(data_source.id), Some(Password::new("s3cret")));
    assert!(!std::fs::read_to_string(&file).unwrap().contains("s3cret"));
    let mut restarted = TestEngine::start(&file, secrets);
    restarted.expect_started(vec![data_source]);
}

#[test]
fn saves_console_files_in_order_and_reloads_them_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("data_sources.toml");
    let data_source = DataSourceId::generate();
    let key = |name: &str| ConsoleKey {
        data_source,
        name: ConsoleName::try_from(name.to_owned()).unwrap(),
    };
    let mut engine = TestEngine::start(&file, Arc::default());
    engine.expect_started(vec![]);

    for text in ["select 1;", "select 2;"] {
        engine.send(Command::SaveConsole {
            key: key("console"),
            text: text.to_owned(),
        });
    }
    engine.send(Command::SaveConsole {
        key: key("console_2"),
        text: "gone".to_owned(),
    });
    engine.send(Command::DeleteConsoleFile(key("console_2")));
    engine.engine.shutdown().unwrap();

    let mut restarted = TestEngine::start(&file, Arc::default());
    restarted.expect(Event::Started);
    restarted.expect(Event::DataSourcesLoaded(vec![]));
    restarted.expect(Event::ConsolesLoaded(vec![SavedConsole {
        key: key("console"),
        text: "select 2;".to_owned(),
    }]));
}

#[test]
fn reports_failed_tests_and_connections() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = TestEngine::start(&dir.path().join("data_sources.toml"), Arc::default());
    engine.expect_started(vec![]);
    let missing = sqlite_data_source(dir.path().join("missing.db"));
    let id = missing.id;

    engine.send(Command::TestConnection {
        data_source: missing.clone(),
        password: PasswordUpdate::Keep,
    });
    match engine.next_event() {
        Event::ConnectionTested {
            id: tested,
            result: Err(error),
        } if tested == id => assert!(error.starts_with("could not connect"), "{error}"),
        other => panic!("expected a failed test, got {other:?}"),
    }

    engine.send(Command::SaveDataSource {
        data_source: missing,
        password: PasswordUpdate::Keep,
    });
    assert!(matches!(engine.next_event(), Event::DataSourceSaved(_)));
    engine.send(Command::Connect(id));
    assert_eq!(engine.expect_session(id), SessionState::Connecting);
    assert!(matches!(engine.expect_session(id), SessionState::Failed(_)));
}

#[test]
fn never_overwrites_an_unreadable_data_sources_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("data_sources.toml");
    let broken = "[[data_source]]\nname = 3\n";
    std::fs::write(&file, broken).unwrap();
    let mut engine = TestEngine::start(&file, Arc::default());
    engine.expect(Event::Started);
    assert!(matches!(
        engine.next_event(),
        Event::DataSourcesUnavailable { .. }
    ));
    engine.expect(Event::ConsolesLoaded(vec![]));

    let data_source = sqlite_data_source(dir.path().join("app.db"));
    engine.send(Command::SaveDataSource {
        data_source: data_source.clone(),
        password: PasswordUpdate::Keep,
    });

    assert!(matches!(
        engine.next_event(),
        Event::DataSourceSaveFailed { id, .. } if id == data_source.id
    ));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), broken);
}

#[test]
fn loading_children_connects_first_and_answers_catalog_requests() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = TestEngine::start(&dir.path().join("data_sources.toml"), Arc::default());
    engine.expect_started(vec![]);
    let database = sqlite_database(dir.path());
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE VIEW v AS SELECT id FROM t")
        .unwrap();
    let data_source = sqlite_data_source(database);
    let id = data_source.id;
    engine.send(Command::SaveDataSource {
        data_source,
        password: PasswordUpdate::Keep,
    });
    assert!(matches!(engine.next_event(), Event::DataSourceSaved(_)));

    engine.send(Command::LoadChildren {
        id,
        path: CatalogPath::root(),
    });

    assert_eq!(engine.expect_session(id), SessionState::Connecting);
    assert!(matches!(
        engine.expect_session(id),
        SessionState::Connected(_)
    ));
    match engine.next_event() {
        Event::ChildrenLoaded {
            id: loaded,
            path,
            result: Ok(nodes),
        } if loaded == id && path.is_root() => {
            let labels: Vec<_> = nodes.iter().map(|node| node.label.as_str()).collect();
            assert_eq!(labels, ["main"]);
        }
        other => panic!("expected the root's children, got {other:?}"),
    }

    let view = CatalogPath::root()
        .child(NodeKind::Schema, "main")
        .child(NodeKind::Folder(FolderKind::Views), "views")
        .child(NodeKind::View, "v");
    engine.send(Command::LoadDefinition {
        id,
        path: view.clone(),
    });
    engine.expect(Event::DefinitionLoaded {
        id,
        path: view,
        result: Ok("CREATE VIEW v AS SELECT id FROM t;".to_owned()),
    });
}

/// Needs `DBDESK_TEST_POSTGRES_URL`, like the Postgres driver tests; skipped without it.
#[test]
fn connects_to_postgres_with_the_saved_password() {
    let Some(parsed) = postgres_test_server() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut engine = TestEngine::start(&dir.path().join("data_sources.toml"), Arc::default());
    engine.expect_started(vec![]);
    let data_source = DataSource {
        id: DataSourceId::generate(),
        name: "test server".to_owned(),
        config: ConnectionConfig::Postgres(parsed.config),
    };
    let id = data_source.id;

    engine.send(Command::SaveDataSource {
        data_source,
        password: PasswordUpdate::Set(parsed.password),
    });
    assert!(matches!(engine.next_event(), Event::DataSourceSaved(_)));
    engine.send(Command::Connect(id));

    assert_eq!(engine.expect_session(id), SessionState::Connecting);
    let state = engine.expect_session(id);
    assert!(
        matches!(&state, SessionState::Connected(server) if server.product == "PostgreSQL"),
        "{state:?}"
    );
}
