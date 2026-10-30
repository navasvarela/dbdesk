//! Redis served to an MCP client through its MCP login alone, driven as a client would, against
//! the compose test server when `DBROVER_TEST_REDIS_URL` is set (see `test-servers.env`).
//!
//! Checks N1's exit for MCP (`docs/nosql.md`): a Redis data source without an MCP login isn't
//! served; one whose MCP login is an ACL user that may only read is, and reads a key as that
//! user; a write is refused by DbRover's check and by the server; and `list_objects` pages a
//! database's keys.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::HashMap;

use dbrover_core::{ConnectionConfig, McpLogin, Password, ProjectId, RedisConfig};
use dbrover_store::{McpOverrides, Project, ProjectSlug, Projects};
use serde_json::json;
use support::{
    Engine, SLUG, Server, account, data_source, failure, mcp_account, ok, refused_directly,
    run_directly, server_from_env, suffix,
};

/// The database this test has to itself on the shared server.
const DATABASE: &str = "11";

fn main() {
    support::run("redis", reads_redis_through_an_mcp_login);
}

async fn reads_redis_through_an_mcp_login() {
    let Some((mut server, password)) = server_from_env("DBROVER_TEST_REDIS_URL") else {
        println!("redis: skipped, DBROVER_TEST_REDIS_URL is not set");
        return;
    };
    DATABASE.clone_into(&mut server.database);
    let dir = tempfile::tempdir().unwrap();
    let project_id = ProjectId::generate();
    let suffix = suffix();
    let reader = format!("mcp_reader_{suffix}");
    let reader_password = Password::new(format!("reader-{suffix}"));
    let config = ConnectionConfig::Redis(RedisConfig {
        server: server.clone(),
        separator: RedisConfig::DEFAULT_SEPARATOR.to_owned(),
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

    // The ACL user the manual suggests, with `SELECT` for a database other than 0.
    let mut fixture = vec![
        "FLUSHDB".to_owned(),
        format!(
            "ACL SETUSER {reader} on >{} ~* +@read +select",
            reader_password.expose()
        ),
        "HSET user:42 name Ada".to_owned(),
    ];
    let keys: Vec<String> = (1..=1_500).map(|index| format!("k{index:04} x")).collect();
    fixture.extend(
        keys.chunks(500)
            .map(|chunk| format!("MSET {}", chunk.join(" "))),
    );
    run_directly(&admin, &fixture).await;

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
        json!({ "data_source": "guarded", "sql": "HGETALL user:42" }),
    )
    .await;
    assert_eq!(read["rows"], json!([["name", "Ada"]]), "{read}");

    let refused = failure(
        client,
        "query",
        json!({ "data_source": "guarded", "sql": "SET user:42 changed" }),
    )
    .await;
    assert_eq!(refused["kind"], "refused", "{refused}");
    let refused = failure(
        client,
        "execute",
        json!({ "data_source": "guarded", "sql": "SET user:42 changed" }),
    )
    .await;
    assert_eq!(refused["kind"], "refused", "{refused}");

    let databases = ok(client, "list_objects", json!({ "data_source": "guarded" })).await;
    let database = &databases["objects"][0];
    assert_eq!(database["name"], format!("db{DATABASE}"), "{databases}");
    let first = ok(
        client,
        "list_objects",
        json!({ "data_source": "guarded", "path": database["path"] }),
    )
    .await;
    assert_eq!(first["objects"].as_array().unwrap().len(), 1_000, "a page");
    let next = first["next"]
        .as_str()
        .expect("a token for the rest")
        .to_owned();
    let rest = ok(
        client,
        "list_objects",
        json!({ "data_source": "guarded", "path": database["path"], "after": next }),
    )
    .await;
    // 1,500 keys and the `user` group.
    assert_eq!(rest["objects"].as_array().unwrap().len(), 501, "the rest");
    assert!(rest.get("next").is_none(), "{rest}");
    let filtered = ok(
        client,
        "list_objects",
        json!({ "data_source": "guarded", "path": database["path"], "filter": "USER:4" }),
    )
    .await;
    assert_eq!(filtered["objects"][0]["name"], "user:42", "{filtered}");
    assert_eq!(filtered["objects"][0]["kind"], "key", "{filtered}");

    let log = server.stop().await;
    assert!(
        log.contains("not served") && log.contains("only through an MCP login"),
        "{log}"
    );

    // The server refuses the reader's write too: its grants, not DbRover's check.
    let mut as_reader = guarded.clone();
    as_reader.mcp_login = None;
    if let ConnectionConfig::Redis(config) = &mut as_reader.config {
        config.server.user.clone_from(&reader);
    }
    let reader_engine = Engine {
        data_source: as_reader,
        password: Some(reader_password),
        table: String::new(),
        writing_function: None,
    };
    let refusal = refused_directly(&reader_engine, "SET user:42 changed").await;
    assert!(refusal.contains("NOPERM"), "{refusal}");

    run_directly(
        &admin,
        &[format!("ACL DELUSER {reader}"), "FLUSHDB".to_owned()],
    )
    .await;
}
