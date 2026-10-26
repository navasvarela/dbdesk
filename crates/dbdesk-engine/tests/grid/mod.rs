//! The grid's power features through the engine, on every engine with a test server: the
//! statements the app writes, built by `dbdesk_sql`, run against real databases.

use dbdesk_core::{Cell, Row, TableDescription};
use dbdesk_engine::DatabaseFile;
use dbdesk_sql::{ColumnValue, Dialect, RowSql};

use super::*;

/// A data source to run a test on, saved in the engine.
pub struct TestSource {
    pub id: DataSourceId,
    pub dialect: Dialect,
    pub name: &'static str,
}

/// Every engine a test can run on here: SQLite and DuckDB always, in files under `dir` (two
/// connections, the console's and the explorer's, must see the same tables), and the servers
/// whose `DBDESK_TEST_*_URL` is set. ClickHouse is left out: the grid never changes its rows.
pub fn test_sources(engine: &mut TestEngine, dir: &std::path::Path) -> Vec<TestSource> {
    let duckdb = dir.join("grid.duckdb");
    engine.send(Command::CreateDatabaseFile(DatabaseFile::DuckDb(
        duckdb.clone(),
    )));
    match engine.next_event() {
        Event::DatabaseFileCreated { result, .. } => result.unwrap(),
        other => panic!("expected the DuckDB file, got {other:?}"),
    }
    let mut configs = vec![
        (
            "sqlite",
            sqlite_data_source(sqlite_database(dir)).config,
            PasswordUpdate::Keep,
        ),
        (
            "duckdb",
            ConnectionConfig::DuckDb(DuckDbConfig {
                database: DuckDbDatabase::File {
                    path: duckdb,
                    read_only: false,
                },
            }),
            PasswordUpdate::Keep,
        ),
    ];
    if let Some(server) = postgres_test_server() {
        configs.push((
            "postgres",
            ConnectionConfig::Postgres(server.config),
            PasswordUpdate::Set(server.password),
        ));
    }
    for (config, password) in mysql_test_servers() {
        configs.push((
            "mysql",
            ConnectionConfig::Mysql(config),
            PasswordUpdate::Set(password),
        ));
    }
    if let Some((config, password)) = sqlserver_test_server() {
        configs.push((
            "sqlserver",
            ConnectionConfig::SqlServer(config),
            PasswordUpdate::Set(password),
        ));
    }
    configs
        .into_iter()
        .map(|(name, config, password)| {
            let dialect = Dialect::from(config.driver());
            let data_source = DataSource {
                id: DataSourceId::generate(),
                name: name.to_owned(),
                color: None,
                safety: dbdesk_core::SafetyLevel::Normal,
                project: None,
                mcp_login: None,
                config,
            };
            let id = data_source.id;
            engine.send(Command::SaveDataSource {
                data_source,
                password,
                ssh_secret: PasswordUpdate::Keep,
            });
            assert!(matches!(engine.next_event(), Event::DataSourceSaved(_)));
            TestSource { id, dialect, name }
        })
        .collect()
}

/// A table name no other test, or another run sharing the test servers, uses at the same time.
pub fn scratch_table(purpose: &str) -> String {
    format!("w5_{purpose}_{}", std::process::id())
}

/// Runs statements in the console and waits for the run to end, failing the test if any
/// statement fails.
pub fn run_ok(engine: &mut TestEngine, source: &TestSource, run: u64, statements: &[&str]) {
    execute(engine, source.id, run, statements);
    loop {
        match next_run_event(engine) {
            Event::StatementFinished {
                result: Err(failure),
                ..
            } => panic!("{}: {statements:?} failed: {failure:?}", source.name),
            Event::RunFinished { run: RunId(r), .. } if r == run => return,
            _ => {}
        }
    }
}

/// Runs one query in the console and returns all its rows.
pub fn query(engine: &mut TestEngine, source: &TestSource, run: u64, sql: &str) -> Vec<Row> {
    execute(engine, source.id, run, &[sql]);
    let mut rows = Vec::new();
    loop {
        match next_run_event(engine) {
            Event::RowsFetched {
                run: RunId(r),
                rows: page,
                ..
            } if r == run => rows.extend(page),
            Event::StatementFinished {
                result: Err(failure),
                ..
            } => panic!("{}: {sql} failed: {failure:?}", source.name),
            Event::RunFinished { run: RunId(r), .. } if r == run => return rows,
            _ => {}
        }
    }
}

/// Describes a table on the explorer's connection, as the grid does.
pub fn describe(engine: &mut TestEngine, source: &TestSource, table: &str) -> TableDescription {
    let relation = RelationName {
        schema: None,
        name: Identifier {
            name: table.to_owned(),
            quoted: false,
        },
    };
    engine.send(Command::DescribeTable {
        data_source: source.id,
        table: relation,
    });
    loop {
        match next_answer(engine) {
            Event::TableDescribed {
                result: Ok(Some(description)),
                ..
            } => return description,
            Event::TableDescribed { result, .. } => {
                panic!("{}: couldn't describe {table}: {result:?}", source.name)
            }
            Event::TransactionChanged { .. } => {}
            other => panic!("expected the table's description, got {other:?}"),
        }
    }
}

fn text(value: &str) -> Cell {
    Some(value.to_owned())
}

fn texts(rows: &[(&str, &str)]) -> Vec<Row> {
    rows.iter()
        .map(|(id, name)| vec![text(id), text(name)])
        .collect()
}

/// A submit as the grid builds it: a delete, an update, an insert that takes the key the
/// delete freed, and an insert of nothing but defaults (`DEFAULT VALUES`, or `() VALUES ()` on
/// MySQL). Then a submit whose insert collides with a key, which undoes its delete too. Every
/// statement sent is in the history.
#[test]
fn inserts_updates_and_deletes_rows_all_or_none_on_every_engine() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = TestEngine::start(&dir.path().join("data_sources.toml"), Arc::default());
    engine.expect_started(vec![]);
    let table = scratch_table("rows");
    for source in test_sources(&mut engine, dir.path()) {
        run_ok(
            &mut engine,
            &source,
            1,
            &[
                &format!(
                    "CREATE TABLE {table} (id int NOT NULL DEFAULT 9 PRIMARY KEY, \
                     name varchar(40) NOT NULL DEFAULT 'anon')"
                ),
                &format!("INSERT INTO {table} (id, name) VALUES (1, 'ada'), (2, 'bob'), (3, 'cy')"),
            ],
        );
        let described = describe(&mut engine, &source, &table);
        let sql = RowSql::new(source.dialect, &described);
        let value = |column, value| ColumnValue { column, value };
        let (one, two, three, ada, bo) = (text("1"), text("2"), text("3"), text("Ada"), text("Bo"));

        edits(
            &engine,
            &[
                &sql.delete(&[value("id", &two)]),
                &sql.update(&[value("name", &ada)], &[value("id", &one)]),
                &sql.insert(&[value("id", &two), value("name", &bo)]),
                &sql.insert(&[]),
            ],
        );
        assert_eq!(expect_edits(&mut engine), Ok(()), "{}", source.name);
        let select = format!("SELECT id, name FROM {table} ORDER BY id");
        let expected = texts(&[("1", "Ada"), ("2", "Bo"), ("3", "cy"), ("9", "anon")]);
        assert_eq!(
            query(&mut engine, &source, 2, &select),
            expected,
            "{}",
            source.name
        );

        edits(
            &engine,
            &[
                &sql.delete(&[value("id", &three)]),
                &sql.insert(&[value("id", &one)]),
            ],
        );
        let failure = expect_edits(&mut engine).unwrap_err();
        assert_eq!(failure.statement, Some(1), "{}: {failure:?}", source.name);
        assert_eq!(
            query(&mut engine, &source, 3, &select),
            expected,
            "{}",
            source.name
        );

        engine.send(Command::SearchHistory(HistoryQuery {
            text: table.clone(),
            ..HistoryQuery::all(source.id)
        }));
        match next_answer(&mut engine) {
            Event::HistoryFound {
                result: Ok(found), ..
            } => {
                let sent = |start: &str, outcome: HistoryOutcome| {
                    found
                        .iter()
                        .any(|entry| entry.sql.starts_with(start) && entry.outcome == outcome)
                };
                assert!(
                    sent("DELETE", HistoryOutcome::Ok) && sent("INSERT", HistoryOutcome::Error),
                    "{}: {found:?}",
                    source.name
                );
            }
            other => panic!("expected the history, got {other:?}"),
        }
        run_ok(&mut engine, &source, 4, &[&format!("DROP TABLE {table}")]);
    }
    engine.shut_down();
}

/// A table's foreign key, in both tables' descriptions: as the child's own key, and as a key
/// pointing at the parent. Column names are listed in key order, and tables by their stored
/// names.
#[test]
fn describes_foreign_keys_both_ways_on_every_engine() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = TestEngine::start(&dir.path().join("data_sources.toml"), Arc::default());
    engine.expect_started(vec![]);
    let (parent, child) = (scratch_table("customers"), scratch_table("orders"));
    for source in test_sources(&mut engine, dir.path()) {
        run_ok(
            &mut engine,
            &source,
            1,
            &[
                &format!(
                    "CREATE TABLE {parent} (id int NOT NULL, region int NOT NULL, \
                     name varchar(40), PRIMARY KEY (region, id))"
                ),
                &format!(
                    "CREATE TABLE {child} (id int NOT NULL PRIMARY KEY, customer int, \
                     customer_region int, \
                     FOREIGN KEY (customer_region, customer) REFERENCES {parent} (region, id))"
                ),
            ],
        );
        let described_parent = describe(&mut engine, &source, &parent);
        let described_child = describe(&mut engine, &source, &child);
        let key = dbdesk_core::ForeignKey {
            table: described_child.table_name(),
            columns: vec!["customer_region".to_owned(), "customer".to_owned()],
            referenced: described_parent.table_name(),
            referenced_columns: vec!["region".to_owned(), "id".to_owned()],
        };
        assert_eq!(
            described_child.foreign_keys,
            std::slice::from_ref(&key),
            "{}",
            source.name
        );
        assert!(described_child.referenced_by.is_empty(), "{}", source.name);
        assert_eq!(described_parent.referenced_by, [key], "{}", source.name);
        assert!(described_parent.foreign_keys.is_empty(), "{}", source.name);
        run_ok(
            &mut engine,
            &source,
            2,
            &[
                &format!("DROP TABLE {child}"),
                &format!("DROP TABLE {parent}"),
            ],
        );
    }
    engine.shut_down();
}
