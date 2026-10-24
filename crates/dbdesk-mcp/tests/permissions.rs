//! The permissions beyond reading, driven as a client would, against SQLite and, when their
//! `DBDESK_TEST_*_URL` is set, the compose test servers for PostgreSQL, MySQL and MariaDB.
//!
//! Checks the MCP2 exit criterion (`docs/mcp-server.md`):
//! - a project with `write` inserts a row, and is refused a `CREATE TABLE`, with the missing
//!   permission in the log;
//! - switching `schema` on in the project lets the `CREATE TABLE` through;
//! - with an MCP login whose role lacks `INSERT`, the database refuses the insert even though
//!   the project allows it (PostgreSQL, whose test user may create roles);
//!
//! and around them: a failed statement rolls back the ones before it, `query` still only
//! reads, transaction control is refused, and the `schemas` allow-list holds for `execute`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::Path;

use dbdesk_core::{DataSource, DriverKind, McpLogin, Password, ProjectId};
use dbdesk_store::{McpOverrides, Project, ProjectSlug, Projects, SchemaScope};
use serde_json::{Value, json};
use support::{
    Client, Engine, SLUG, Server, engines, failure, mcp_account, ok, passwords, run_directly,
    suffix,
};

fn main() {
    support::run("permissions", checks_each_permission);
}

fn project(id: ProjectId, overrides: McpOverrides) -> Project {
    let mut project = Project::new(
        id,
        "Test".to_owned(),
        ProjectSlug::try_from(SLUG.to_owned()).unwrap(),
    );
    project.mcp_enabled = true;
    project.mcp = overrides;
    project
}

fn write_project(dir: &Path, engines: &[Engine], project: Project) {
    let data_sources: Vec<DataSource> = engines
        .iter()
        .map(|engine| engine.data_source.clone())
        .collect();
    support::write_files(
        dir,
        &data_sources,
        Projects {
            projects: vec![project],
            ..Projects::default()
        },
        &passwords(engines),
    );
}

async fn count(engine: &Engine) -> String {
    let rows = run_directly(engine, &[format!("SELECT count(*) FROM {}", engine.table)]).await;
    rows[0][0].clone().unwrap()
}

async fn execute(client: &Client, engine: &Engine, sql: &str) -> Value {
    ok(
        client,
        "execute",
        json!({ "data_source": engine.data_source.name, "sql": sql }),
    )
    .await
}

async fn refused(client: &Client, tool: &'static str, engine: &Engine, sql: &str) -> String {
    let error = failure(
        client,
        tool,
        json!({ "data_source": engine.data_source.name, "sql": sql }),
    )
    .await;
    assert_eq!(error["kind"], "refused", "{sql}: {error}");
    error["message"].as_str().unwrap().to_owned()
}

/// `write` alone: rows change, the schema doesn't.
async fn check_write(client: &Client, engine: &Engine, created: &str) {
    let table = &engine.table;
    let inserted = execute(
        client,
        engine,
        &format!("INSERT INTO {table} (id, name) VALUES (4, 'four')"),
    )
    .await;
    assert_eq!(inserted["committed"], true, "{inserted}");
    assert_eq!(inserted["statements"][0]["rows_affected"], 1, "{inserted}");
    assert_eq!(count(engine).await, "4", "{}", engine.data_source.name);

    let create = format!("CREATE TABLE {created} (id INTEGER PRIMARY KEY)");
    let message = refused(client, "execute", engine, &create).await;
    assert!(
        message.contains("needs the project's schema permission"),
        "{message}"
    );
    let mixed = format!("INSERT INTO {table} (id, name) VALUES (5, 'five'); {create}");
    let message = refused(client, "execute", engine, &mixed).await;
    assert!(message.starts_with("statement 2 "), "{message}");
    assert_eq!(count(engine).await, "4", "nothing ran of a refused call");

    let failing = execute_failure(
        client,
        engine,
        &format!(
            "UPDATE {table} SET name = 'changed' WHERE id = 1; \
             INSERT INTO {table} (id, name) VALUES (1, 'duplicate')"
        ),
    )
    .await;
    assert_eq!(failing["kind"], "database", "{failing}");
    let message = failing["message"].as_str().unwrap();
    assert!(
        message.starts_with("statement 2 of 2 failed, and nothing was committed"),
        "{message}"
    );
    let rows = run_directly(engine, &[format!("SELECT name FROM {table} WHERE id = 1")]).await;
    assert_eq!(
        rows,
        [[Some("one".to_owned())]],
        "the UPDATE was rolled back"
    );

    let message = refused(
        client,
        "query",
        engine,
        &format!("DELETE FROM {table} WHERE id = 4"),
    )
    .await;
    assert!(message.contains("use execute"), "{message}");
    let message = refused(client, "execute", engine, "COMMIT").await;
    assert!(message.contains("transaction of its own"), "{message}");
}

async fn execute_failure(client: &Client, engine: &Engine, sql: &str) -> Value {
    failure(
        client,
        "execute",
        json!({ "data_source": engine.data_source.name, "sql": sql }),
    )
    .await
}

/// `write` and `schema`: the `CREATE TABLE` refused before now runs.
async fn check_schema(client: &Client, engine: &Engine, created: &str) {
    let created_and_filled = execute(
        client,
        engine,
        &format!(
            "CREATE TABLE {created} (id INTEGER PRIMARY KEY); \
             INSERT INTO {created} (id) VALUES (1)"
        ),
    )
    .await;
    assert_eq!(
        created_and_filled["statements"].as_array().unwrap().len(),
        2,
        "{created_and_filled}"
    );
    let rows = run_directly(engine, &[format!("SELECT count(*) FROM {created}")]).await;
    assert_eq!(rows, [[Some("1".to_owned())]]);
    execute(client, engine, &format!("DROP TABLE {created}")).await;
    let message = refused(client, "execute", engine, "GRANT SELECT ON t TO someone").await;
    assert!(
        message.contains("needs the project's admin permission"),
        "{message}"
    );
}

async fn checks_each_permission() {
    let dir = tempfile::tempdir().unwrap();
    let id = ProjectId::generate();
    let engines = engines(dir.path(), id);
    for engine in &engines {
        run_directly(engine, &fixture(engine)).await;
    }
    let created = format!("mcp_created_{}", suffix());

    println!("permissions: write");
    write_project(
        dir.path(),
        &engines,
        project(
            id,
            McpOverrides {
                write: Some(true),
                ..McpOverrides::default()
            },
        ),
    );
    let server = Server::start(dir.path()).await;
    assert!(server.tools().await.contains(&"execute".to_owned()));
    for engine in &engines {
        println!("permissions: write on {}", engine.data_source.name);
        check_write(&server.client, engine, &created).await;
    }
    let log = server.stop().await;
    assert!(log.contains("missing_permission=\"schema\""), "{log}");

    println!("permissions: write and schema");
    write_project(
        dir.path(),
        &engines,
        project(
            id,
            McpOverrides {
                write: Some(true),
                schema: Some(true),
                ..McpOverrides::default()
            },
        ),
    );
    let server = Server::start(dir.path()).await;
    for engine in &engines {
        println!("permissions: schema on {}", engine.data_source.name);
        check_schema(&server.client, engine, &created).await;
    }
    let log = server.stop().await;
    assert!(
        log.contains("only a dedicated login's grants are airtight"),
        "{log}"
    );

    if let Some(postgres) = engines
        .iter()
        .find(|engine| engine.data_source.config.driver() == DriverKind::Postgres)
    {
        println!("permissions: an MCP login that can't insert");
        check_mcp_login(dir.path(), postgres, id).await;
    }

    for engine in &engines {
        run_directly(engine, &[format!("DROP TABLE {}", engine.table)]).await;
    }
    let history = rusqlite::Connection::open(dir.path().join("history.sqlite")).unwrap();
    let executed: i64 = history
        .query_row(
            "SELECT count(*) FROM history WHERE source = 'mcp' AND sql LIKE 'INSERT%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(executed >= 1, "execute's statements are in the history");
}

/// A role that may only read, used as the data source's MCP login, in a project that
/// allows writing: the parser lets the `INSERT` through, the database refuses it.
async fn check_mcp_login(dir: &Path, postgres: &Engine, id: ProjectId) {
    let role = format!("mcp_reader_{}", suffix());
    let password = format!("pw-{}", suffix());
    let table = &postgres.table;
    run_directly(
        postgres,
        &[
            format!("CREATE ROLE {role} LOGIN PASSWORD '{password}'"),
            format!("GRANT SELECT ON {table} TO {role}"),
        ],
    )
    .await;
    let mut as_reader = postgres.data_source.clone();
    as_reader.mcp_login = Some(McpLogin { user: role.clone() });
    let mut secrets = passwords(std::slice::from_ref(postgres));
    secrets.insert(mcp_account(&as_reader), Password::new(password));
    support::write_files(
        dir,
        &[as_reader],
        Projects {
            projects: vec![project(
                id,
                McpOverrides {
                    write: Some(true),
                    schemas: Some(SchemaScope::Only(vec!["public".to_owned()])),
                    ..McpOverrides::default()
                },
            )],
            ..Projects::default()
        },
        &secrets,
    );

    let server = Server::start(dir).await;
    let client = &server.client;
    let read = ok(
        client,
        "query",
        json!({ "data_source": postgres.data_source.name, "sql": "SELECT current_user" }),
    )
    .await;
    assert_eq!(read["rows"], json!([[role.as_str()]]), "{read}");
    let denied = execute_failure(
        client,
        postgres,
        &format!("INSERT INTO {table} (id, name) VALUES (9, 'nine')"),
    )
    .await;
    assert_eq!(denied["kind"], "database", "{denied}");
    assert_eq!(
        denied["sqlstate"], "42501",
        "insufficient_privilege: {denied}"
    );

    let message = refused(
        client,
        "execute",
        postgres,
        "DELETE FROM pg_catalog.pg_class WHERE false",
    )
    .await;
    assert!(message.contains("\"pg_catalog\""), "{message}");
    let log = server.stop().await;
    assert!(
        log.contains(&format!("connecting as the MCP login {role}")),
        "{log}"
    );

    run_directly(
        postgres,
        &[
            format!("REVOKE ALL ON {table} FROM {role}"),
            format!("DROP ROLE {role}"),
        ],
    )
    .await;
}

fn fixture(engine: &Engine) -> Vec<String> {
    let table = &engine.table;
    vec![
        format!("CREATE TABLE {table} (id INTEGER PRIMARY KEY, name VARCHAR(40) NOT NULL)"),
        format!("INSERT INTO {table} (id, name) VALUES (1, 'one'), (2, 'two'), (3, 'three')"),
    ]
}
