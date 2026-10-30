//! Drives the MCP server as a client would: in another process, over its standard input and
//! output, against SQLite and, when their `DBROVER_TEST_*_URL` is set, the compose test
//! servers for PostgreSQL, MySQL and MariaDB (see `test-servers.env`).
//!
//! Checks the MCP1 exit criterion (`docs/mcp-server.md`): list, describe and query on each
//! engine; a `DELETE` hidden in a CTE refused by the parser; a write hidden in a function
//! refused by the read-only transaction (PostgreSQL, MySQL and MariaDB; SQLite has no SQL
//! functions, and its file is opened read-only); both refusals in the log; and every
//! statement in the query history, marked as coming from MCP and the client's name.
//! The permissions beyond reading are checked in `permissions.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use dbrover_core::{ConnectionConfig, DataSourceId, ProjectId};
use dbrover_store::{McpOverrides, Project, ProjectSlug, Projects};
use serde_json::{Value, json};
use support::{Client, Engine, SLUG, Server, engines, failure, ok, passwords, run_directly};

/// Low, so truncation shows with a three-row table.
const MAX_ROWS: u32 = 2;

fn main() {
    support::run("stdio", drives_the_server_over_stdio);
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
        dbrover_core::DriverKind::Postgres => format!(
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
            dbrover_core::DriverKind::Postgres => format!("DROP FUNCTION {function}()"),
            _ => format!("DROP FUNCTION {function}"),
        });
    }
    statements
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

    let ordered = format!("SELECT id, name FROM {table} ORDER BY id");
    let as_text = [
        ("csv", "id,name\r\n1,one\r\n2,two\r\n"),
        (
            "markdown",
            "| id | name |\n| --- | --- |\n| 1 | one |\n| 2 | two |\n",
        ),
        (
            "jsonl",
            "{\"id\":\"1\",\"name\":\"one\"}\n{\"id\":\"2\",\"name\":\"two\"}\n",
        ),
    ];
    for (format, text) in as_text {
        let formatted = ok(
            client,
            "query",
            json!({ "data_source": name, "sql": ordered, "format": format }),
        )
        .await;
        assert_eq!(formatted["text"], text, "{format}: {formatted}");
        assert_eq!(formatted["truncated"], true, "{format}");
        assert_eq!(formatted["row_count"], 2, "{format}");
    }

    ok(
        client,
        "explain",
        json!({ "data_source": name, "sql": format!("SELECT * FROM {table} WHERE id = 1") }),
    )
    .await;

    let hidden_delete = match engine.data_source.config.driver() {
        dbrover_core::DriverKind::Postgres => {
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
    let mut data_sources: Vec<_> = engines
        .iter()
        .map(|engine| engine.data_source.clone())
        .collect();
    let mut hidden = engines[0].data_source.clone();
    hidden.id = DataSourceId::generate();
    hidden.name = "in another project".to_owned();
    hidden.project = Some(other.id);
    data_sources.push(hidden);
    support::write_files(
        dir.path(),
        &data_sources,
        Projects {
            projects: vec![project, other],
            ..Projects::default()
        },
        &passwords(&engines),
    );

    let server = Server::start(dir.path()).await;
    assert!(
        !server.tools().await.contains(&"execute".to_owned()),
        "a project that only reads has no execute tool"
    );
    let client = &server.client;

    let listed = ok(client, "list_data_sources", json!({})).await;
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
        check_engine(client, engine).await;
    }
    let unknown = failure(
        client,
        "query",
        json!({ "data_source": "nope", "sql": "SELECT 1" }),
    )
    .await;
    assert_eq!(unknown["kind"], "invalid_arguments");

    let log = server.stop().await;
    for engine in &engines {
        run_directly(engine, &teardown(engine)).await;
    }

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
            // `rmcp` is the name rmcp's client gives itself when it connects.
            "SELECT count(*) FROM history
             WHERE source = 'mcp' AND origin = 'mcp' AND detail = 'rmcp'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // Per engine: the query, the explain, the typo; the refused CTE never reached it.
    let expected = i64::try_from(3 * engines.len()).unwrap();
    assert!(recorded >= expected, "{recorded} statements recorded");
}
