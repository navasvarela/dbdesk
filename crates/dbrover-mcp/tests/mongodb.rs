//! MongoDB served to an MCP client through its MCP login alone, driven as a client would, against
//! the compose test server when `DBROVER_TEST_MONGODB_URL` is set (see `test-servers.env`).
//!
//! Checks N2's exit for MCP (`docs/nosql.md`): a MongoDB data source without an MCP login isn't
//! served; one whose MCP login is a user with only the `read` role is, and queries a collection
//! as that user; a write is refused by DbRover's check and by the server; and `list_objects`
//! lists the databases and a database's collections.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::HashMap;

use dbrover_core::{ConnectionConfig, McpLogin, MongoConfig, Password, ProjectId};
use dbrover_store::{McpOverrides, Project, ProjectSlug, Projects};
use serde_json::json;
use support::{
    Engine, SLUG, Server, account, data_source, failure, mcp_account, ok, refused_directly,
    run_directly, server_from_env, suffix,
};

/// The database this test has to itself on the shared server.
const DATABASE: &str = "dbrover_mcp";

fn main() {
    support::run("mongodb", reads_mongodb_through_an_mcp_login);
}

async fn reads_mongodb_through_an_mcp_login() {
    let Some((mut server, password)) = server_from_env("DBROVER_TEST_MONGODB_URL") else {
        println!("mongodb: skipped, DBROVER_TEST_MONGODB_URL is not set");
        return;
    };
    DATABASE.clone_into(&mut server.database);
    server.tls = dbrover_core::TlsMode::Disable;
    let dir = tempfile::tempdir().unwrap();
    let project_id = ProjectId::generate();
    let suffix = suffix();
    let reader = format!("mcp_reader_{suffix}");
    let reader_password = Password::new(format!("reader-{suffix}"));
    let config = ConnectionConfig::Mongo(MongoConfig {
        server: server.clone(),
        auth_database: String::new(),
        connection_string: None,
    });

    let admin = Engine {
        data_source: data_source("unguarded", project_id, config.clone()),
        password: Some(password.clone()),
        table: String::new(),
        writing_function: None,
    };
    let mut guarded = data_source("guarded", project_id, config);
    guarded.mcp_login = Some(McpLogin {
        user: reader.clone(),
    });

    // The user the manual suggests: the `read` role on the data source's database.
    run_directly(
        &admin,
        &[
            "db.dropDatabase()".to_owned(),
            format!(
                "db.adminCommand({{createUser: '{reader}', pwd: '{}', roles: [{{role: 'read', db: '{DATABASE}'}}]}})",
                reader_password.expose()
            ),
            "db.orders.insertMany([{_id: 1, status: 'open'}, {_id: 2, status: 'paid'}])".to_owned(),
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
        (mcp_account(&guarded), reader_password.clone()),
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

    let read = ok(
        client,
        "query",
        json!({ "data_source": "guarded", "sql": "db.orders.find({status: 'open'})" }),
    )
    .await;
    assert_eq!(
        read["rows"],
        json!([[r#"{"_id":{"$numberInt":"1"},"status":"open"}"#]]),
        "{read}"
    );

    for tool in ["query", "execute"] {
        let refused = failure(
            client,
            tool,
            json!({ "data_source": "guarded", "sql": "db.orders.deleteMany({})" }),
        )
        .await;
        assert_eq!(refused["kind"], "refused", "{tool}: {refused}");
    }

    let databases = ok(client, "list_objects", json!({ "data_source": "guarded" })).await;
    let database = databases["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|object| object["name"] == DATABASE)
        .unwrap_or_else(|| panic!("{DATABASE} isn't listed: {databases}"))
        .clone();
    let collections = ok(
        client,
        "list_objects",
        json!({ "data_source": "guarded", "path": database["path"] }),
    )
    .await;
    assert_eq!(collections["objects"][0]["name"], "orders", "{collections}");
    assert_eq!(
        collections["objects"][0]["kind"], "collection",
        "{collections}"
    );

    let log = server.stop().await;
    assert!(
        log.contains("not served") && log.contains("only through an MCP login"),
        "{log}"
    );

    // The server refuses the reader's write too: its role, not DbRover's check.
    let mut as_reader = guarded.clone();
    as_reader.mcp_login = None;
    if let ConnectionConfig::Mongo(config) = &mut as_reader.config {
        config.server.user.clone_from(&reader);
    }
    let reader_engine = Engine {
        data_source: as_reader,
        password: Some(reader_password),
        table: String::new(),
        writing_function: None,
    };
    let refusal = refused_directly(&reader_engine, "db.orders.deleteMany({})").await;
    assert!(refusal.contains("not authorized"), "{refusal}");

    run_directly(
        &admin,
        &[
            format!("db.adminCommand({{dropUser: '{reader}'}})"),
            "db.dropDatabase()".to_owned(),
        ],
    )
    .await;
}
