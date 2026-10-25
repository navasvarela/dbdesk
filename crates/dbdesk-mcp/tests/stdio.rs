//! Drives the MCP server as a client would: in another process, over its standard input and
//! output, against SQLite and, when their `DBDESK_TEST_*_URL` is set, the compose test
//! servers for PostgreSQL, MySQL and MariaDB (see `test-servers.env`).
//!
//! Checks the MCP1 exit criterion (`docs/mcp-server.md`): list, describe and query on each
//! engine; a `DELETE` hidden in a CTE refused by the parser; a write hidden in a function
//! refused by the read-only transaction (PostgreSQL, MySQL and MariaDB; SQLite has no SQL
//! functions, and its file is opened read-only); both refusals in the log; and every
//! statement in the query history, marked as coming from MCP.
//!
//! Without libtest's harness (`harness = false` in Cargo.toml), because the server process is
//! this same binary started again, and libtest would print its own lines to the standard
//! output the protocol runs on.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use dbdesk_core::{
    ConnectionConfig, DataSource, DataSourceId, MysqlConfig, Password, PostgresConfig, ProjectId,
    ServerConfig, SqliteConfig, TlsMode,
};
use dbdesk_engine::open_data_source;
use dbdesk_mcp::{McpConfig, serve_stdio};
use dbdesk_store::{
    DataSourceFile, HistoryFile, McpOverrides, Project, ProjectFile, ProjectSlug, Projects,
    SecretError, SecretStore,
};
use rmcp::ServiceExt as _;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt as _;

/// Set in the server process to the directory holding its files.
const CHILD_DIR: &str = "DBDESK_MCP_TEST_DIR";
const PASSWORDS: &str = "passwords.json";
const SLUG: &str = "test";
/// Low, so truncation shows with a three-row table.
const MAX_ROWS: u32 = 2;

fn main() {
    if let Ok(dir) = std::env::var(CHILD_DIR) {
        serve_as_child(Path::new(&dir));
        return;
    }
    let filters: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    if !filters.is_empty()
        && !filters
            .iter()
            .any(|filter| "stdio".contains(filter.as_str()))
    {
        return;
    }
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(drives_the_server_over_stdio());
    println!("stdio: ok");
}

/// Passwords in memory, standing in for the OS keychain; the server process reads them from
/// a file the test wrote.
struct FileSecrets(HashMap<DataSourceId, Password>);

impl SecretStore for FileSecrets {
    fn password(&self, id: DataSourceId) -> Result<Option<Password>, SecretError> {
        Ok(self.0.get(&id).cloned())
    }
    fn set_password(&self, _: DataSourceId, _: &Password) -> Result<(), SecretError> {
        Ok(())
    }
    fn delete_password(&self, _: DataSourceId) -> Result<(), SecretError> {
        Ok(())
    }
}

/// The server process, as `dbdesk mcp` runs it: logs to standard error, the protocol on
/// standard output.
fn serve_as_child(dir: &Path) {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let passwords: HashMap<String, String> =
        serde_json::from_str(&std::fs::read_to_string(dir.join(PASSWORDS)).unwrap()).unwrap();
    let secrets = FileSecrets(
        passwords
            .into_iter()
            .map(|(id, password)| (id.parse().unwrap(), Password::new(password)))
            .collect(),
    );
    let config = McpConfig {
        data_sources: DataSourceFile::new(dir.join("data_sources.toml")),
        projects: ProjectFile::new(dir.join("projects.toml")),
        history: HistoryFile::new(dir.join("history.sqlite")),
        secrets: Arc::new(secrets),
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(serve_stdio(config, SLUG))
        .unwrap();
}

/// One engine under test: its data source and how its fixture is spelt.
struct Engine {
    data_source: DataSource,
    password: Option<Password>,
    /// What the table is called; unique per run, since other test runs share the servers.
    table: String,
    /// A function that writes, called from a `SELECT`. SQLite has no SQL functions.
    writing_function: Option<String>,
}

fn suffix() -> String {
    DataSourceId::generate().to_string()[..8].to_owned()
}

fn engines(dir: &Path, project: ProjectId) -> Vec<Engine> {
    let table = format!("mcp_items_{}", suffix());
    let sqlite_path = dir.join("shop.db");
    rusqlite::Connection::open(&sqlite_path).unwrap();
    let mut engines = vec![Engine {
        data_source: data_source(
            "sqlite",
            project,
            ConnectionConfig::Sqlite(SqliteConfig {
                path: sqlite_path,
                read_only: false,
            }),
        ),
        password: None,
        table: table.clone(),
        writing_function: None,
    }];
    if let Some((server, password)) = server_from_env("DBDESK_TEST_POSTGRES_URL") {
        engines.push(Engine {
            data_source: data_source(
                "postgres",
                project,
                ConnectionConfig::Postgres(PostgresConfig { server }),
            ),
            password: Some(password),
            table: table.clone(),
            writing_function: Some(format!("mcp_bump_{}", suffix())),
        });
    }
    for (name, variable) in [
        ("mysql", "DBDESK_TEST_MYSQL_URL"),
        ("mariadb", "DBDESK_TEST_MARIADB_URL"),
    ] {
        if let Some((server, password)) = server_from_env(variable) {
            engines.push(Engine {
                data_source: data_source(
                    name,
                    project,
                    ConnectionConfig::Mysql(MysqlConfig { server }),
                ),
                password: Some(password),
                table: table.clone(),
                writing_function: Some(format!("mcp_bump_{}", suffix())),
            });
        }
    }
    engines
}

fn data_source(name: &str, project: ProjectId, config: ConnectionConfig) -> DataSource {
    DataSource {
        id: DataSourceId::generate(),
        name: name.to_owned(),
        color: None,
        safety: dbdesk_core::SafetyLevel::Normal,
        project: Some(project),
        config,
    }
}

/// Reads `scheme://user:password@host:port/database`, the form `test-servers.env` uses.
fn server_from_env(variable: &str) -> Option<(ServerConfig, Password)> {
    let url = std::env::var(variable).ok()?;
    let (_, rest) = url.split_once("://")?;
    let (credentials, rest) = rest.split_once('@')?;
    let (user, password) = credentials.split_once(':')?;
    let (address, database) = rest.split_once('/')?;
    let (host, port) = address.split_once(':')?;
    Some((
        ServerConfig {
            host: host.to_owned(),
            port: port.parse::<u16>().ok()?.into(),
            database: database.to_owned(),
            user: user.to_owned(),
            tls: TlsMode::default(),
        },
        Password::new(password),
    ))
}

/// Runs statements on an ordinary, writable connection: the fixture, and checks after.
async fn run_directly(engine: &Engine, statements: &[String]) -> Vec<Vec<Option<String>>> {
    let secrets: Arc<dyn SecretStore> = Arc::new(FileSecrets(
        engine
            .password
            .iter()
            .map(|password| (engine.data_source.id, password.clone()))
            .collect(),
    ));
    let mut writable = engine.data_source.clone();
    if let ConnectionConfig::Sqlite(config) = &mut writable.config {
        config.read_only = false;
    }
    let mut connection = open_data_source(&writable, &secrets).await.unwrap();
    let mut last = Vec::new();
    for statement in statements {
        let mut cursor = connection
            .execute(statement)
            .await
            .unwrap_or_else(|error| panic!("{statement}: {error:?}"));
        last = cursor.next_page(100).await.unwrap();
        cursor.finish().await.unwrap();
    }
    connection.close().await.unwrap();
    last
}

fn fixture(engine: &Engine) -> Vec<String> {
    let table = &engine.table;
    let mut statements = vec![
        format!("CREATE TABLE {table} (id INTEGER PRIMARY KEY, name VARCHAR(40) NOT NULL)"),
        format!("INSERT INTO {table} (id, name) VALUES (1, 'one'), (2, 'two'), (3, 'three')"),
    ];
    let Some(function) = &engine.writing_function else {
        return statements;
    };
    statements.push(match engine.data_source.config.driver() {
        dbdesk_core::DriverKind::Postgres => format!(
            "CREATE FUNCTION {function}() RETURNS integer LANGUAGE sql AS \
             $$ INSERT INTO {table} (id, name) VALUES (100, 'sneaky') RETURNING id $$"
        ),
        // Creating it needs log_bin_trust_function_creators on MySQL, as docker-compose.yml
        // sets it, since the test user lacks SUPER.
        _ => format!(
            "CREATE FUNCTION {function}() RETURNS INT MODIFIES SQL DATA \
             BEGIN INSERT INTO {table} (id, name) VALUES (100, 'sneaky'); RETURN 1; END"
        ),
    });
    statements
}

fn teardown(engine: &Engine) -> Vec<String> {
    let mut statements = vec![format!("DROP TABLE {}", engine.table)];
    if let Some(function) = &engine.writing_function {
        statements.push(match engine.data_source.config.driver() {
            dbdesk_core::DriverKind::Postgres => format!("DROP FUNCTION {function}()"),
            _ => format!("DROP FUNCTION {function}"),
        });
    }
    statements
}

fn write_files(dir: &Path, engines: &[Engine], project: Project, other: Project) {
    let mut data_sources: Vec<DataSource> = engines
        .iter()
        .map(|engine| engine.data_source.clone())
        .collect();
    let mut hidden = engines[0].data_source.clone();
    hidden.id = DataSourceId::generate();
    hidden.name = "in another project".to_owned();
    hidden.project = Some(other.id);
    data_sources.push(hidden);
    DataSourceFile::new(dir.join("data_sources.toml"))
        .save(&data_sources)
        .unwrap();
    ProjectFile::new(dir.join("projects.toml"))
        .save(&Projects {
            projects: vec![project, other],
            ..Projects::default()
        })
        .unwrap();
    let passwords: HashMap<String, String> = engines
        .iter()
        .filter_map(|engine| {
            let password = engine.password.as_ref()?;
            Some((
                engine.data_source.id.to_string(),
                password.expose().to_owned(),
            ))
        })
        .collect();
    std::fs::write(
        dir.join(PASSWORDS),
        serde_json::to_string(&passwords).unwrap(),
    )
    .unwrap();
}

type Client = rmcp::service::RunningService<rmcp::RoleClient, ()>;

async fn call(client: &Client, tool: &'static str, arguments: Value) -> (bool, Value) {
    let Value::Object(arguments) = arguments else {
        panic!("arguments must be an object");
    };
    let result = client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
        .await
        .unwrap();
    let value = result.structured_content.clone().unwrap_or(Value::Null);
    (result.is_error == Some(true), value)
}

async fn ok(client: &Client, tool: &'static str, arguments: Value) -> Value {
    let (failed, value) = call(client, tool, arguments.clone()).await;
    assert!(!failed, "{tool} {arguments} failed: {value}");
    value
}

async fn failure(client: &Client, tool: &'static str, arguments: Value) -> Value {
    let (failed, value) = call(client, tool, arguments.clone()).await;
    assert!(failed, "{tool} {arguments} should have failed: {value}");
    value["error"].clone()
}

/// Follows `list_objects` from the top down to the table, through its database, schema and
/// `tables` folder, as an assistant would.
async fn find_table(client: &Client, engine: &Engine) -> Value {
    let database = match &engine.data_source.config {
        ConnectionConfig::Postgres(config) => Some(config.server.database.clone()),
        _ => None,
    };
    let schema = engine.data_source.config.default_schema().to_owned();
    let mut path = json!([]);
    for _ in 0..4 {
        let listed = ok(
            client,
            "list_objects",
            json!({ "data_source": engine.data_source.name, "path": path }),
        )
        .await;
        let objects = listed["objects"].as_array().unwrap();
        if let Some(table) = objects
            .iter()
            .find(|object| object["kind"] == "table" && object["name"] == engine.table.as_str())
        {
            return table.clone();
        }
        let next = objects.iter().find(|object| match object["kind"].as_str() {
            Some("database") => database.as_deref() == object["name"].as_str(),
            Some("schema") => object["name"] == schema.as_str(),
            Some("folder") => object["name"] == "tables",
            _ => false,
        });
        path =
            next.unwrap_or_else(|| panic!("no way down from {path} in {listed}"))["path"].clone();
    }
    panic!("{} not found in {}", engine.table, engine.data_source.name);
}

async fn check_engine(client: &Client, engine: &Engine) {
    let name = engine.data_source.name.as_str();
    let table = engine.table.as_str();

    let found = find_table(client, engine).await;
    assert!(found["path"].as_array().unwrap().len() >= 3, "{found}");

    let described = ok(
        client,
        "describe_table",
        json!({ "data_source": name, "table": table }),
    )
    .await;
    assert_eq!(described["primary_key"], json!(["id"]), "{described}");
    let columns: Vec<&str> = described["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| column["name"].as_str().unwrap())
        .collect();
    assert_eq!(columns, ["id", "name"], "{described}");

    let queried = ok(
        client,
        "query",
        json!({ "data_source": name, "sql": format!("SELECT id, name FROM {table} ORDER BY id;") }),
    )
    .await;
    assert_eq!(
        queried["rows"],
        json!([["1", "one"], ["2", "two"]]),
        "{queried}"
    );
    assert_eq!(queried["truncated"], true);
    assert_eq!(queried["columns"][1]["name"], "name");

    ok(
        client,
        "explain",
        json!({ "data_source": name, "sql": format!("SELECT * FROM {table} WHERE id = 1") }),
    )
    .await;

    let hidden_delete = match engine.data_source.config.driver() {
        dbdesk_core::DriverKind::Postgres => {
            format!("WITH gone AS (DELETE FROM {table} RETURNING *) SELECT * FROM gone")
        }
        _ => format!(
            "WITH ids AS (SELECT 1 AS id) DELETE FROM {table} WHERE id IN (SELECT id FROM ids)"
        ),
    };
    let refused = failure(
        client,
        "query",
        json!({ "data_source": name, "sql": hidden_delete }),
    )
    .await;
    assert_eq!(refused["kind"], "refused", "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("changes data"),
        "{refused}"
    );

    let typo = failure(
        client,
        "query",
        json!({ "data_source": name, "sql": format!("SELECT nope FROM {table}") }),
    )
    .await;
    assert_eq!(typo["kind"], "database", "{typo}");
    if engine.password.is_some() {
        assert!(typo["sqlstate"].is_string(), "{typo}");
    }

    if let Some(function) = &engine.writing_function {
        let caught = failure(
            client,
            "query",
            json!({ "data_source": name, "sql": format!("SELECT {function}()") }),
        )
        .await;
        assert_eq!(caught["sqlstate"], "25006", "{caught}");
    }

    let rows = run_directly(engine, &[format!("SELECT count(*) FROM {table}")]).await;
    assert_eq!(
        rows,
        [[Some("3".to_owned())]],
        "{name}: something was written"
    );
}

async fn drives_the_server_over_stdio() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::new(
        ProjectId::generate(),
        "Test".to_owned(),
        ProjectSlug::try_from(SLUG.to_owned()).unwrap(),
    );
    project.mcp_enabled = true;
    project.mcp = McpOverrides {
        max_rows: std::num::NonZeroU32::new(MAX_ROWS),
        ..McpOverrides::default()
    };
    let other = Project::new(
        ProjectId::generate(),
        "Other".to_owned(),
        ProjectSlug::try_from("other".to_owned()).unwrap(),
    );
    let engines = engines(dir.path(), project.id);
    for engine in &engines {
        run_directly(engine, &fixture(engine)).await;
    }
    write_files(dir.path(), &engines, project, other);

    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command.env(CHILD_DIR, dir.path());
    let (transport, stderr) = TokioChildProcess::builder(command)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let log = Arc::new(Mutex::new(String::new()));
    let log_task = {
        let log = Arc::clone(&log);
        let mut stderr = stderr.unwrap();
        tokio::spawn(async move {
            let mut text = String::new();
            stderr.read_to_string(&mut text).await.unwrap();
            log.lock().unwrap().push_str(&text);
        })
    };
    let client: Client = ().serve(transport).await.unwrap();

    let listed = ok(&client, "list_data_sources", json!({})).await;
    let names: Vec<&str> = listed["data_sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|data_source| data_source["name"].as_str().unwrap())
        .collect();
    let expected: Vec<&str> = engines
        .iter()
        .map(|engine| engine.data_source.name.as_str())
        .collect();
    assert_eq!(names, expected, "only the project's data sources: {listed}");
    assert_eq!(listed["limits"]["max_rows"], MAX_ROWS);

    for engine in &engines {
        println!("stdio: checking {}", engine.data_source.name);
        check_engine(&client, engine).await;
    }
    let unknown = failure(
        &client,
        "query",
        json!({ "data_source": "nope", "sql": "SELECT 1" }),
    )
    .await;
    assert_eq!(unknown["kind"], "invalid_arguments");

    client.cancel().await.unwrap();
    log_task.await.unwrap();
    for engine in &engines {
        run_directly(engine, &teardown(engine)).await;
    }

    let log = log.lock().unwrap().clone();
    assert!(
        log.contains("MCP call refused: this statement changes data"),
        "{log}"
    );
    if engines
        .iter()
        .any(|engine| engine.writing_function.is_some())
    {
        assert!(
            log.contains("refused by the read-only transaction"),
            "{log}"
        );
    }
    let history = rusqlite::Connection::open(dir.path().join("history.sqlite")).unwrap();
    let recorded: i64 = history
        .query_row(
            "SELECT count(*) FROM history WHERE source = 'mcp'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // Per engine: the query, the explain, the typo; the refused CTE never reached it.
    let expected = i64::try_from(3 * engines.len()).unwrap();
    assert!(recorded >= expected, "{recorded} statements recorded");
}
