//! An engine without a read-only mode, served through its MCP login alone, driven as a client
//! would, against the compose test server for SQL Server when `DBROVER_TEST_SQLSERVER_URL` is
//! set (see `test-servers.env`).
//!
//! Checks N0a's guard where there is no read-only session (`docs/nosql.md`): a data source
//! without an MCP login isn't served, and the log says why; one with an MCP login is, and its
//! queries run as that login, whose grants are what refuses writes; and `execute` is refused,
//! since DbRover has no transaction to run changes in there for MCP clients.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::HashMap;

use dbrover_core::{ConnectionConfig, McpLogin, Password, ProjectId, SqlServerConfig};
use dbrover_store::{McpOverrides, Project, ProjectSlug, Projects};
use serde_json::json;
use support::{
    Engine, SLUG, Server, account, data_source, failure, mcp_account, ok, run_directly,
    server_from_env, suffix,
};

fn main() {
    support::run(
        "login",
        serves_an_engine_without_a_read_only_mode_through_its_login,
    );
}

async fn serves_an_engine_without_a_read_only_mode_through_its_login() {
    let Some((server, password)) = server_from_env("DBROVER_TEST_SQLSERVER_URL") else {
        println!("login: skipped, DBROVER_TEST_SQLSERVER_URL is not set");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let project_id = ProjectId::generate();
    let suffix = suffix();
    let table = format!("mcp_login_items_{suffix}");
    let reader = format!("mcp_reader_{suffix}");
    // SQL Server's password policy wants upper and lower case, a digit and a symbol.
    let reader_password = Password::new(format!("Reader-{suffix}-2026!"));
    let config = ConnectionConfig::SqlServer(SqlServerConfig { server });

    let admin = Engine {
        data_source: data_source("unguarded", project_id, config.clone()),
        password: Some(password.clone()),
        table: table.clone(),
        writing_function: None,
    };
    let mut guarded = data_source("guarded", project_id, config);
    guarded.mcp_login = Some(McpLogin {
        user: reader.clone(),
    });

    run_directly(
        &admin,
        &[
            format!("CREATE TABLE {table} (id INT PRIMARY KEY, name NVARCHAR(40) NOT NULL)"),
            format!("INSERT INTO {table} (id, name) VALUES (1, N'one'), (2, N'two')"),
            format!(
                "CREATE LOGIN [{reader}] WITH PASSWORD = '{}', CHECK_POLICY = OFF",
                reader_password.expose()
            ),
            format!("CREATE USER [{reader}] FOR LOGIN [{reader}]"),
            format!("ALTER ROLE db_datareader ADD MEMBER [{reader}]"),
        ],
    )
    .await;

    let mut project = Project::new(
        project_id,
        "Test".to_owned(),
        ProjectSlug::try_from(SLUG.to_owned()).unwrap(),
    );
    project.mcp_enabled = true;
    project.mcp = McpOverrides {
        write: Some(true),
        ..McpOverrides::default()
    };
    let passwords = HashMap::from([
        (account(&admin.data_source), password.clone()),
        (account(&guarded), password),
        (mcp_account(&guarded), reader_password),
    ]);
    support::write_files(
        dir.path(),
        &[admin.data_source.clone(), guarded.clone()],
        Projects {
            projects: vec![project],
            ..Projects::default()
        },
        &passwords,
    );

    let server = Server::start(dir.path()).await;
    let client = &server.client;
    let listed = ok(client, "list_data_sources", json!({})).await;
    let names: Vec<&str> = listed["data_sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|data_source| data_source["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["guarded"], "{listed}");

    let queried = ok(
        client,
        "query",
        json!({
            "data_source": "guarded",
            "sql": format!("SELECT SUSER_SNAME() AS login, name FROM {table} ORDER BY id"),
        }),
    )
    .await;
    assert_eq!(
        queried["rows"],
        json!([[reader, "one"], [reader, "two"]]),
        "{queried}"
    );

    let refused = failure(
        client,
        "execute",
        json!({
            "data_source": "guarded",
            "sql": format!("INSERT INTO {table} (id, name) VALUES (3, N'three')"),
        }),
    )
    .await;
    assert_eq!(refused["kind"], "refused", "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("served for reading only"),
        "{refused}"
    );

    let log = server.stop().await;
    assert!(
        log.contains("not served") && log.contains("only through an MCP login"),
        "{log}"
    );

    // The server has stopped, so the login has no sessions left; should one linger, the login
    // is left behind rather than failing the test.
    run_directly(
        &admin,
        &[
            format!("DROP TABLE {table}"),
            format!("DROP USER [{reader}]"),
            format!(
                "IF NOT EXISTS (SELECT 1 FROM sys.dm_exec_sessions WHERE login_name = N'{reader}') \
                 DROP LOGIN [{reader}]"
            ),
        ],
    )
    .await;
}
