//! The object helpers' DDL: applied on the explorer's connection, all or none where the
//! engine's DDL is transactional, refused on read-only data sources, recorded in the history;
//! a table built from a spec described back as the spec says, and views and schemas created,
//! changed and dropped, on every engine with a test server.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::{
    TestEngine, clickhouse_test_server, mysql_test_servers, postgres_test_server,
    sqlite_data_source, sqlite_database, sqlserver_test_server,
};
use dbdesk_core::{
    CatalogNode, CatalogPath, ConnectionConfig, DataSource, DataSourceId, DuckDbConfig,
    DuckDbDatabase, FolderKind, Identifier, NodeKind, Password, RelationName, SafetyLevel,
    TableDescription,
};
use dbdesk_engine::{Command, DdlFailure, Event, PasswordUpdate, SessionState};
use dbdesk_sql::{
    Check, ColumnSpec, DdlOperation, Dialect, ForeignKey, ObjectName, ReferentialAction,
    SchemaCreation, SchemaSpec, TableSpec, UniqueKey, ViewKind, ViewSpec, ddl_statements,
    split_statements, view_query,
};
use dbdesk_store::{HistoryOutcome, HistoryQuery};

fn start(dir: &std::path::Path) -> TestEngine {
    let mut engine = TestEngine::start(&dir.join("data_sources.toml"), Arc::default());
    engine.expect_started(vec![]);
    engine
}

fn save(engine: &mut TestEngine, data_source: &DataSource, password: Option<Password>) {
    engine.send(Command::SaveDataSource {
        data_source: data_source.clone(),
        password: password.map_or(PasswordUpdate::Keep, PasswordUpdate::Set),
        ssh_secret: PasswordUpdate::Keep,
    });
    engine.expect(Event::DataSourceSaved(data_source.clone()));
}

fn data_source(name: &str, config: ConnectionConfig) -> DataSource {
    DataSource {
        id: DataSourceId::generate(),
        name: name.to_owned(),
        color: None,
        safety: SafetyLevel::Normal,
        project: None,
        mcp_login: None,
        config,
    }
}

/// Applies `statements` and waits for the answer, skipping connection changes.
fn apply(
    engine: &mut TestEngine,
    data_source: DataSourceId,
    statements: Vec<String>,
) -> Result<(), DdlFailure> {
    engine.send(Command::ApplyDdl {
        data_source,
        statements,
    });
    loop {
        match engine.next_event() {
            Event::DdlApplied {
                data_source: answered,
                result,
            } if answered == data_source => return result,
            Event::SessionChanged {
                state: SessionState::Failed(error),
                ..
            } => panic!("could not connect: {error}"),
            Event::SessionChanged { .. } => {}
            other => panic!("expected DdlApplied, got {other:?}"),
        }
    }
}

fn describe(engine: &mut TestEngine, data_source: DataSourceId, name: &str) -> TableDescription {
    let table = RelationName {
        schema: None,
        name: Identifier {
            name: name.to_owned(),
            quoted: true,
        },
    };
    engine.send(Command::DescribeTable {
        data_source,
        table: table.clone(),
    });
    loop {
        match engine.next_event() {
            Event::TableDescribed { result, .. } => {
                return result
                    .expect("describe")
                    .unwrap_or_else(|| panic!("{name} exists"));
            }
            Event::SessionChanged { .. } => {}
            other => panic!("expected TableDescribed, got {other:?}"),
        }
    }
}

fn sqlite_tables(database: &std::path::Path) -> Vec<String> {
    let connection = rusqlite::Connection::open(database).unwrap();
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn applies_ddl_all_or_none_where_ddl_is_transactional_and_records_it() {
    let dir = tempfile::tempdir().unwrap();
    let database = sqlite_database(dir.path());
    let mut engine = start(dir.path());
    let app = sqlite_data_source(database.clone());
    save(&mut engine, &app, None);

    let failed = apply(
        &mut engine,
        app.id,
        vec![
            "CREATE TABLE kept (id INTEGER)".to_owned(),
            "CREATE TABLE t (id INTEGER)".to_owned(),
        ],
    );
    let failure = failed.expect_err("t exists already");
    assert_eq!(failure.statement, Some(1));
    assert_eq!(failure.applied, 0);
    assert!(failure.message.contains("already exists"), "{failure:?}");
    assert_eq!(
        sqlite_tables(&database),
        ["t"],
        "the first table was undone"
    );

    let applied = apply(
        &mut engine,
        app.id,
        vec![
            "CREATE TABLE a (id INTEGER)".to_owned(),
            "CREATE TABLE b (id INTEGER)".to_owned(),
        ],
    );
    assert_eq!(applied, Ok(()));
    assert_eq!(sqlite_tables(&database), ["a", "b", "t"]);

    engine.send(Command::SearchHistory(HistoryQuery::all(app.id)));
    let entries = match engine.next_event() {
        Event::HistoryFound { result, .. } => result.unwrap(),
        other => panic!("expected the history, got {other:?}"),
    };
    let recorded: Vec<(&str, HistoryOutcome)> = entries
        .iter()
        .map(|entry| (entry.sql.as_str(), entry.outcome))
        .collect();
    assert_eq!(
        recorded,
        [
            ("CREATE TABLE b (id INTEGER)", HistoryOutcome::Ok),
            ("CREATE TABLE a (id INTEGER)", HistoryOutcome::Ok),
            ("CREATE TABLE t (id INTEGER)", HistoryOutcome::Error),
            ("CREATE TABLE kept (id INTEGER)", HistoryOutcome::Ok),
        ],
        "newest first, the undone statement too, since it ran"
    );
    engine.shut_down();
}

#[test]
fn a_read_only_data_source_refuses_ddl_without_sending_it() {
    let dir = tempfile::tempdir().unwrap();
    let database = sqlite_database(dir.path());
    let mut engine = start(dir.path());
    let production = DataSource {
        name: "production".to_owned(),
        safety: SafetyLevel::ReadOnly,
        ..sqlite_data_source(database.clone())
    };
    save(&mut engine, &production, None);

    let refused = apply(&mut engine, production.id, vec!["DROP TABLE t".to_owned()]);
    assert_eq!(
        refused,
        Err(DdlFailure {
            statement: None,
            message: "Refused: \u{201c}production\u{201d} is read-only, so nothing in it can be \
                      created or changed."
                .to_owned(),
            applied: 0,
        })
    );
    assert_eq!(sqlite_tables(&database), ["t"]);
    engine.shut_down();
}

/// A parent table and a child pointing at it, with a key of two columns, a unique key, a
/// check and a default: the same spec on every engine, with its types and what it can't have
/// adjusted for the engine.
fn round_trip_tables(dialect: Dialect, suffix: &str) -> (TableSpec, TableSpec) {
    let (integer, text) = match dialect {
        Dialect::ClickHouse => ("Int32", "String"),
        _ => ("integer", "varchar(40)"),
    };
    let column = |name: &str, data_type: &str, nullable: bool, default: Option<&str>| ColumnSpec {
        name: name.to_owned(),
        data_type: data_type.to_owned(),
        nullable,
        default: default.map(str::to_owned),
    };
    let names = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
    let parent_name = format!("dbdesk_parent_{suffix}");
    let parent = TableSpec {
        name: ObjectName::new(None, &parent_name),
        columns: vec![column("id", integer, false, None)],
        primary_key: names(&["id"]),
        unique_keys: vec![],
        foreign_keys: vec![],
        checks: vec![],
    };
    let capabilities = dialect.capabilities();
    let child = TableSpec {
        name: ObjectName::new(None, &format!("dbdesk_child_{suffix}")),
        columns: vec![
            column("parent", integer, false, None),
            column("line", integer, false, None),
            column("code", text, false, None),
            column("quantity", integer, false, Some("1")),
            column("note", text, true, None),
        ],
        primary_key: names(&["parent", "line"]),
        unique_keys: if capabilities.unique_keys {
            vec![UniqueKey {
                name: None,
                columns: names(&["parent", "code"]),
            }]
        } else {
            vec![]
        },
        foreign_keys: if capabilities.foreign_keys {
            vec![ForeignKey {
                name: None,
                columns: names(&["parent"]),
                target: ObjectName::new(None, &parent_name),
                target_columns: names(&["id"]),
                on_delete: ReferentialAction::NoAction,
                on_update: ReferentialAction::NoAction,
            }]
        } else {
            vec![]
        },
        checks: vec![Check {
            name: Some(format!("quantity_positive_{suffix}")),
            expression: "quantity > 0".to_owned(),
        }],
    };
    (parent, child)
}

/// Creates the two tables in one `ApplyDdl`, checks the child is described as its spec says,
/// then drops both.
fn check_round_trip(engine: &mut TestEngine, data_source: &DataSource, password: Option<Password>) {
    save(engine, data_source, password);
    let dialect = Dialect::from(data_source.config.driver());
    let suffix = data_source.id.to_string().replace('-', "");
    let suffix = &suffix[..8];
    let (parent, child) = round_trip_tables(dialect, suffix);
    let create = ddl_statements(
        &[
            DdlOperation::CreateTable(parent.clone()),
            DdlOperation::CreateTable(child.clone()),
        ],
        dialect,
    )
    .unwrap();
    assert_eq!(apply(engine, data_source.id, create), Ok(()), "{dialect:?}");

    let described = describe(engine, data_source.id, &child.name.name);
    let columns: Vec<(&str, bool)> = described
        .columns
        .iter()
        .map(|column| (column.name.as_str(), column.nullable))
        .collect();
    let expected: Vec<(&str, bool)> = child
        .columns
        .iter()
        .map(|column| (column.name.as_str(), column.nullable))
        .collect();
    assert_eq!(columns, expected, "{dialect:?}");
    let quantity = &described.columns[3];
    assert!(
        quantity.default.as_deref().is_some_and(|d| d.contains('1')),
        "{dialect:?} default: {quantity:?}"
    );
    // A ClickHouse table's key is its sort order, which a description never reports.
    if dialect != Dialect::ClickHouse {
        assert_eq!(described.primary_key, child.primary_key, "{dialect:?}");
    }

    let drop = ddl_statements(
        &[
            DdlOperation::DropTable {
                table: child.name,
                cascade: false,
            },
            DdlOperation::DropTable {
                table: parent.name,
                cascade: false,
            },
        ],
        dialect,
    )
    .unwrap();
    assert_eq!(apply(engine, data_source.id, drop), Ok(()), "{dialect:?}");
}

#[test]
fn a_table_built_from_a_spec_is_described_as_the_spec_says_on_every_engine() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = start(dir.path());
    let sqlite = sqlite_data_source(sqlite_database(dir.path()));
    check_round_trip(&mut engine, &sqlite, None);
    let duckdb = data_source(
        "duckdb",
        ConnectionConfig::DuckDb(DuckDbConfig {
            database: DuckDbDatabase::InMemory,
        }),
    );
    check_round_trip(&mut engine, &duckdb, None);
    if let Some(server) = postgres_test_server() {
        let postgres = data_source("postgres", ConnectionConfig::Postgres(server.config));
        check_round_trip(&mut engine, &postgres, Some(server.password));
    }
    for (config, password) in mysql_test_servers() {
        let mysql = data_source("mysql", ConnectionConfig::Mysql(config));
        check_round_trip(&mut engine, &mysql, Some(password));
    }
    if let Some((config, password)) = sqlserver_test_server() {
        let sqlserver = data_source("sqlserver", ConnectionConfig::SqlServer(config));
        check_round_trip(&mut engine, &sqlserver, Some(password));
    }
    if let Some((config, password)) = clickhouse_test_server() {
        let clickhouse = data_source("clickhouse", ConnectionConfig::ClickHouse(config));
        check_round_trip(&mut engine, &clickhouse, Some(password));
    }
    engine.shut_down();
}

fn load_ddl(engine: &mut TestEngine, data_source: DataSourceId, path: CatalogPath) -> String {
    engine.send(Command::LoadDdl {
        data_source,
        path: path.clone(),
    });
    loop {
        match engine.next_event() {
            Event::DdlLoaded {
                path: loaded,
                result,
                ..
            } if loaded == path => return result.expect("the table's DDL"),
            Event::SessionChanged { .. } => {}
            other => panic!("expected DdlLoaded, got {other:?}"),
        }
    }
}

#[test]
fn copies_a_sqlite_tables_ddl_from_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = start(dir.path());
    let app = sqlite_data_source(sqlite_database(dir.path()));
    save(&mut engine, &app, None);
    let path = CatalogPath::root()
        .child(NodeKind::Schema, "main")
        .child(NodeKind::Folder(FolderKind::Tables), "tables")
        .child(NodeKind::Table, "t");
    let ddl = load_ddl(&mut engine, app.id, path);
    assert!(
        ddl.contains("CREATE TABLE t (id INTEGER PRIMARY KEY)"),
        "{ddl}"
    );
    engine.shut_down();
}

/// PostgreSQL keeps no `CREATE TABLE`, so Copy DDL rebuilds it; running what it copied after
/// dropping the table must make the same table again.
#[test]
fn rebuilds_a_postgres_tables_ddl_that_recreates_it() {
    let Some(server) = postgres_test_server() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut engine = start(dir.path());
    let database = server.config.server.database.clone();
    let postgres = data_source("postgres", ConnectionConfig::Postgres(server.config));
    save(&mut engine, &postgres, Some(server.password));
    let suffix = postgres.id.to_string().replace('-', "");
    let schema = format!("dbdesk_copy_{}", &suffix[..8]);
    let (parent, mut child) = round_trip_tables(Dialect::Postgres, &suffix[..8]);
    let parent = TableSpec {
        name: ObjectName::new(Some(&schema), &parent.name.name),
        ..parent
    };
    child.name.schema = Some(schema.clone());
    child.foreign_keys[0].target = parent.name.clone();
    let mut create = vec![format!("CREATE SCHEMA {schema}")];
    create.extend(
        ddl_statements(
            &[
                DdlOperation::CreateTable(parent),
                DdlOperation::CreateTable(child.clone()),
            ],
            Dialect::Postgres,
        )
        .unwrap(),
    );
    assert_eq!(apply(&mut engine, postgres.id, create), Ok(()));

    let path = CatalogPath::root()
        .child(NodeKind::Database, database)
        .child(NodeKind::Schema, schema.clone())
        .child(NodeKind::Folder(FolderKind::Tables), "tables")
        .child(NodeKind::Table, child.name.name.clone());
    let ddl = load_ddl(&mut engine, postgres.id, path);
    assert!(
        ddl.starts_with(&format!("CREATE TABLE {schema}.{} (", child.name.name)),
        "{ddl}"
    );
    assert!(ddl.contains("quantity integer DEFAULT 1 NOT NULL"), "{ddl}");
    assert!(ddl.contains("PRIMARY KEY (parent, line)"), "{ddl}");
    for constraint in [
        "FOREIGN KEY (parent)",
        "UNIQUE (parent, code)",
        "CHECK (quantity > 0)",
    ] {
        assert!(ddl.contains(constraint), "{constraint} in {ddl}");
    }

    let mut recreate = vec![format!("DROP TABLE {schema}.{}", child.name.name)];
    recreate.extend(
        split_statements(&ddl, Dialect::Postgres)
            .into_iter()
            .map(|range| ddl[range].to_owned()),
    );
    let recreated = apply(&mut engine, postgres.id, recreate);
    let cleaned = apply(
        &mut engine,
        postgres.id,
        vec![format!("DROP SCHEMA {schema} CASCADE")],
    );
    assert_eq!(recreated, Ok(()), "{ddl}");
    assert_eq!(cleaned, Ok(()));
    engine.shut_down();
}

/// Where DDL commits as it runs, a failure leaves the statements before it applied, and says
/// how many.
#[test]
fn mysql_keeps_the_statements_before_a_failure_and_says_so() {
    let Some((config, password)) = mysql_test_servers().into_iter().next() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut engine = start(dir.path());
    let mysql = data_source("mysql", ConnectionConfig::Mysql(config));
    save(&mut engine, &mysql, Some(password));
    let table = format!(
        "dbdesk_kept_{}",
        &mysql.id.to_string().replace('-', "")[..8]
    );
    let failed = apply(
        &mut engine,
        mysql.id,
        vec![
            format!("CREATE TABLE {table} (id integer)"),
            format!("CREATE TABLE {table} (id integer)"),
        ],
    );
    let cleaned = apply(&mut engine, mysql.id, vec![format!("DROP TABLE {table}")]);
    let failure = failed.expect_err("the second CREATE fails");
    assert_eq!((failure.statement, failure.applied), (Some(1), 1));
    assert_eq!(cleaned, Ok(()), "the first table was kept");
    engine.shut_down();
}

#[test]
fn the_history_keeps_ddl_apart_from_catalog_queries() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = start(dir.path());
    let app = sqlite_data_source(sqlite_database(dir.path()));
    save(&mut engine, &app, None);
    engine.send(Command::LoadChildren {
        id: app.id,
        path: CatalogPath::root(),
    });
    loop {
        if let Event::ChildrenLoaded { .. } = engine.next_event() {
            break;
        }
    }
    engine.send(Command::SearchHistory(HistoryQuery::all(app.id)));
    match engine.next_event() {
        Event::HistoryFound { result, .. } => assert_eq!(result.unwrap(), vec![]),
        other => panic!("expected the history, got {other:?}"),
    }
    engine.shut_down();
}

/// Every data source the round trips run on: SQLite and an in-memory DuckDB, and each test
/// server that is set, with its password.
fn every_engine() -> Vec<(DataSource, Option<Password>)> {
    let mut sources = vec![(
        data_source(
            "duckdb",
            ConnectionConfig::DuckDb(DuckDbConfig {
                database: DuckDbDatabase::InMemory,
            }),
        ),
        None,
    )];
    if let Some(server) = postgres_test_server() {
        sources.push((
            data_source("postgres", ConnectionConfig::Postgres(server.config)),
            Some(server.password),
        ));
    }
    for (config, password) in mysql_test_servers() {
        sources.push((
            data_source("mysql", ConnectionConfig::Mysql(config)),
            Some(password),
        ));
    }
    if let Some((config, password)) = sqlserver_test_server() {
        sources.push((
            data_source("sqlserver", ConnectionConfig::SqlServer(config)),
            Some(password),
        ));
    }
    if let Some((config, password)) = clickhouse_test_server() {
        sources.push((
            data_source("clickhouse", ConnectionConfig::ClickHouse(config)),
            Some(password),
        ));
    }
    sources
}

/// A name no other test run uses, from the data source's id.
fn unique(prefix: &str, data_source: &DataSource) -> String {
    let id = data_source.id.to_string().replace('-', "");
    format!("{prefix}_{}", &id[..8])
}

/// Where unqualified names go: the connection's default schema, under its database where
/// the tree lists databases.
fn default_schema(data_source: &DataSource) -> CatalogPath {
    let root = match data_source.config.database() {
        Some(database) => CatalogPath::root().child(NodeKind::Database, database),
        None => CatalogPath::root(),
    };
    root.child(NodeKind::Schema, data_source.config.default_schema())
}

fn view_path(data_source: &DataSource, kind: ViewKind, name: &str) -> CatalogPath {
    let (folder, node) = match kind {
        ViewKind::View => (FolderKind::Views, NodeKind::View),
        ViewKind::Materialized => (FolderKind::MaterializedViews, NodeKind::MaterializedView),
    };
    default_schema(data_source)
        .child(NodeKind::Folder(folder), folder.label())
        .child(node, name)
}

fn load_definition(
    engine: &mut TestEngine,
    data_source: DataSourceId,
    path: CatalogPath,
) -> String {
    engine.send(Command::LoadDefinition {
        id: data_source,
        path: path.clone(),
    });
    loop {
        match engine.next_event() {
            Event::DefinitionLoaded {
                path: loaded,
                result,
                ..
            } if loaded == path => return result.expect("the definition"),
            Event::SessionChanged { .. } => {}
            other => panic!("expected DefinitionLoaded, got {other:?}"),
        }
    }
}

fn load_children(
    engine: &mut TestEngine,
    data_source: DataSourceId,
    path: CatalogPath,
) -> Vec<CatalogNode> {
    engine.send(Command::LoadChildren {
        id: data_source,
        path: path.clone(),
    });
    loop {
        match engine.next_event() {
            Event::ChildrenLoaded {
                path: loaded,
                result,
                ..
            } if loaded == path => return result.expect("the children"),
            Event::SessionChanged { .. } => {}
            other => panic!("expected ChildrenLoaded, got {other:?}"),
        }
    }
}

/// Applies one operation, built for the data source's dialect.
fn apply_operation(
    engine: &mut TestEngine,
    data_source: &DataSource,
    operation: DdlOperation,
) -> Result<(), DdlFailure> {
    let dialect = Dialect::from(data_source.config.driver());
    let statements = operation.statements(dialect).expect("the operation's SQL");
    apply(engine, data_source.id, statements)
}

/// The query the catalog shows for the view at `path`, as Edit view starts from it.
fn shown_query(engine: &mut TestEngine, data_source: &DataSource, path: CatalogPath) -> String {
    let dialect = Dialect::from(data_source.config.driver());
    let definition = load_definition(engine, data_source.id, path);
    view_query(&definition, dialect)
        .unwrap_or_else(|| panic!("{dialect:?}: no query in {definition:?}"))
}

/// Creates a view, edits its query starting from the one its definition shows, copies its
/// DDL, renames it where the engine can, and drops it.
fn check_view_round_trip(
    engine: &mut TestEngine,
    data_source: &DataSource,
    password: Option<Password>,
) {
    save(engine, data_source, password);
    let dialect = Dialect::from(data_source.config.driver());
    let name = unique("dbdesk_view", data_source);
    let view = ViewSpec {
        name: ObjectName::new(None, &name),
        kind: ViewKind::View,
        query: "SELECT 1 AS one".to_owned(),
        with_data: true,
    };
    let path = view_path(data_source, ViewKind::View, &name);
    let created = apply_operation(engine, data_source, DdlOperation::CreateView(view.clone()));
    assert_eq!(created, Ok(()), "{dialect:?}");
    let shown = shown_query(engine, data_source, path.clone());
    assert!(shown.contains('1'), "{dialect:?}: {shown}");

    // The same column with another value: PostgreSQL's `CREATE OR REPLACE VIEW` refuses to
    // rename or drop a view's columns.
    let edited = ViewSpec {
        query: shown.replace('1', "2"),
        ..view.clone()
    };
    let replaced = apply_operation(engine, data_source, DdlOperation::ReplaceView(edited));
    assert_eq!(replaced, Ok(()), "{dialect:?}");
    let shown = shown_query(engine, data_source, path.clone());
    assert!(shown.contains('2'), "{dialect:?}: {shown}");
    let copied = load_ddl(engine, data_source.id, path);
    assert!(copied.contains(&name), "{dialect:?}: {copied}");

    let mut dropped = view.name.clone();
    if dialect.capabilities().renames_views {
        let new_name = format!("{name}_renamed");
        let renamed = apply_operation(
            engine,
            data_source,
            DdlOperation::RenameView {
                view: view.name.clone(),
                kind: ViewKind::View,
                new_name: new_name.clone(),
            },
        );
        assert_eq!(renamed, Ok(()), "{dialect:?}");
        dropped.name = new_name;
    }
    let views = load_children(
        engine,
        data_source.id,
        path_parent(&view_path(data_source, ViewKind::View, &dropped.name)),
    );
    assert!(
        views.iter().any(|node| node.label == dropped.name),
        "{dialect:?} lists {}",
        dropped.name
    );
    let drop = DdlOperation::DropView {
        view: dropped,
        kind: ViewKind::View,
        cascade: false,
    };
    assert_eq!(
        apply_operation(engine, data_source, drop),
        Ok(()),
        "{dialect:?}"
    );
}

fn path_parent(path: &CatalogPath) -> CatalogPath {
    path.parent().expect("a view has a folder")
}

#[test]
fn creates_edits_renames_and_drops_a_view_on_every_engine() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = start(dir.path());
    let sqlite = sqlite_data_source(sqlite_database(dir.path()));
    check_view_round_trip(&mut engine, &sqlite, None);
    for (data_source, password) in every_engine() {
        check_view_round_trip(&mut engine, &data_source, password);
    }
    engine.shut_down();
}

#[test]
fn creates_refreshes_edits_and_drops_a_postgres_materialized_view() {
    let Some(server) = postgres_test_server() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut engine = start(dir.path());
    let postgres = data_source("postgres", ConnectionConfig::Postgres(server.config));
    save(&mut engine, &postgres, Some(server.password));
    let name = unique("dbdesk_matview", &postgres);
    let view = ViewSpec {
        name: ObjectName::new(None, &name),
        kind: ViewKind::Materialized,
        query: "SELECT 1 AS one".to_owned(),
        with_data: false,
    };
    let path = view_path(&postgres, ViewKind::Materialized, &name);
    let steps = [
        DdlOperation::CreateView(view.clone()),
        DdlOperation::RefreshMaterializedView {
            view: view.name.clone(),
            concurrently: false,
        },
        DdlOperation::ReplaceView(ViewSpec {
            query: "SELECT 2 AS two".to_owned(),
            with_data: true,
            ..view.clone()
        }),
    ];
    for step in steps {
        assert_eq!(
            apply_operation(&mut engine, &postgres, step.clone()),
            Ok(()),
            "{step:?}"
        );
    }
    let copied = load_ddl(&mut engine, postgres.id, path);
    assert!(
        copied.starts_with(&format!("CREATE MATERIALIZED VIEW public.{name} AS")),
        "{copied}"
    );
    assert!(copied.contains("2 AS two"), "{copied}");

    let renamed = format!("{name}_renamed");
    let steps = [
        DdlOperation::RenameView {
            view: view.name.clone(),
            kind: ViewKind::Materialized,
            new_name: renamed.clone(),
        },
        DdlOperation::DropView {
            view: ObjectName::new(None, &renamed),
            kind: ViewKind::Materialized,
            cascade: true,
        },
    ];
    for step in steps {
        assert_eq!(
            apply_operation(&mut engine, &postgres, step.clone()),
            Ok(()),
            "{step:?}"
        );
    }
    engine.shut_down();
}

/// The node schemas are listed under: the data source's database, or the data source itself
/// where its schemas are databases.
fn schemas_parent(data_source: &DataSource) -> CatalogPath {
    default_schema(data_source)
        .parent()
        .expect("a schema has a parent")
}

/// Creates a schema and a table in it, sees the schema listed, then drops it with what it
/// holds: with `CASCADE` where the engine has it, as `DROP DATABASE` always does, and
/// otherwise by dropping the table first, after the schema's drop was refused.
fn check_schema_round_trip(
    engine: &mut TestEngine,
    data_source: &DataSource,
    password: Option<Password>,
) {
    save(engine, data_source, password);
    let dialect = Dialect::from(data_source.config.driver());
    let capabilities = dialect.capabilities();
    let schema = unique("dbdesk_schema", data_source);
    let table = TableSpec {
        name: ObjectName::new(Some(&schema), "items"),
        columns: vec![ColumnSpec {
            name: "id".to_owned(),
            data_type: capabilities.common_types[0].to_owned(),
            nullable: false,
            default: None,
        }],
        primary_key: vec!["id".to_owned()],
        unique_keys: vec![],
        foreign_keys: vec![],
        checks: vec![],
    };
    let created = apply(
        engine,
        data_source.id,
        ddl_statements(
            &[
                DdlOperation::CreateSchema(SchemaSpec {
                    name: schema.clone(),
                    owner: None,
                }),
                DdlOperation::CreateTable(table.clone()),
            ],
            dialect,
        )
        .unwrap(),
    );
    assert_eq!(created, Ok(()), "{dialect:?}");
    let listed = |engine: &mut TestEngine| {
        load_children(engine, data_source.id, schemas_parent(data_source))
            .iter()
            .any(|node| node.label == schema)
    };
    assert!(listed(engine), "{dialect:?} lists {schema}");

    let drop = |cascade| DdlOperation::DropSchema {
        schema: schema.clone(),
        cascade,
    };
    let empties_itself = capabilities.drop_schema_cascade
        || capabilities.schema_creation == SchemaCreation::CreateDatabase;
    if !empties_itself {
        assert!(
            apply_operation(engine, data_source, drop(false)).is_err(),
            "{dialect:?} refuses to drop a schema holding a table"
        );
        let dropped = apply_operation(
            engine,
            data_source,
            DdlOperation::DropTable {
                table: table.name,
                cascade: false,
            },
        );
        assert_eq!(dropped, Ok(()), "{dialect:?}");
    }
    let dropped = apply_operation(engine, data_source, drop(capabilities.drop_schema_cascade));
    assert_eq!(dropped, Ok(()), "{dialect:?}");
    assert!(!listed(engine), "{dialect:?} no longer lists {schema}");
}

/// MySQL's and MariaDB's test user may only use its own database, so it can't create
/// another; their `CREATE DATABASE` is covered by the builder's golden tests.
#[test]
fn creates_lists_and_drops_a_schema_with_what_it_holds() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = start(dir.path());
    for (data_source, password) in every_engine() {
        if !matches!(data_source.config, ConnectionConfig::Mysql(_)) {
            check_schema_round_trip(&mut engine, &data_source, password);
        }
    }
    engine.shut_down();
}
