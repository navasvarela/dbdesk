//! The two kinds of transaction the MCP server opens on one writable SQLite connection:
//! reads under `query_only`, and changes in a transaction that commits or rolls back.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::{MemorySecrets, sqlite_data_source, sqlite_database};
use dbdesk_core::{Connection, DriverKind};
use dbdesk_engine::{Tunnels, open_data_source, read_only, read_write};
use dbdesk_store::SecretStore;

async fn run(connection: &mut dyn Connection, sql: &str) -> Result<Option<String>, String> {
    let mut cursor = connection
        .execute(sql)
        .await
        .map_err(|error| error.to_string())?;
    let rows = cursor
        .next_page(1)
        .await
        .map_err(|error| error.to_string())?;
    cursor.finish().await.map_err(|error| error.to_string())?;
    Ok(rows.first().and_then(|row| row.first().cloned().flatten()))
}

#[tokio::test]
async fn reads_refuse_writes_and_changes_commit_or_roll_back() {
    let dir = tempfile::tempdir().unwrap();
    let data_source = sqlite_data_source(sqlite_database(dir.path()));
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
    let mut connection = open_data_source(
        &data_source,
        &secrets,
        &Tunnels::new(dir.path().join("known_hosts")),
    )
    .await
    .unwrap();
    let connection = connection.as_mut();
    let read = read_only::mode_on_writable_connection(DriverKind::Sqlite).unwrap();
    let write = read_write::mode(DriverKind::Sqlite).unwrap();
    let count = "SELECT count(*) FROM t";

    read_only::begin(connection, read, None).await.unwrap();
    let refused = run(connection, "INSERT INTO t VALUES (1)").await;
    read_only::rollback(connection, read).await.unwrap();
    assert!(refused.is_err(), "query_only lets nothing be written");

    read_write::begin(connection, write, None).await.unwrap();
    run(connection, "INSERT INTO t VALUES (1)").await.unwrap();
    read_write::commit(connection).await.unwrap();
    read_write::begin(connection, write, None).await.unwrap();
    run(connection, "INSERT INTO t VALUES (2)").await.unwrap();
    read_write::rollback(connection).await.unwrap();

    read_only::begin(connection, read, None).await.unwrap();
    let rows = run(connection, count).await.unwrap();
    read_only::rollback(connection, read).await.unwrap();
    assert_eq!(
        rows.as_deref(),
        Some("1"),
        "the commit kept, the rollback undone"
    );
}
