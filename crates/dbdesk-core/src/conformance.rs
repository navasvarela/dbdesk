//! Behaviour every driver must share, run by each driver crate against its own backend.
//! See `docs/testing.md`. These functions panic on failure, like any test assertion.

#![allow(clippy::expect_used)]

use std::time::{Duration, Instant};

use crate::{
    CatalogNode, CatalogPath, Connection, DriverError, NodeKind, ResultCursor, describe_error,
};

/// A fresh connection reports its server, answers pings, and closes cleanly.
pub async fn check_lifecycle(mut connection: Box<dyn Connection>) {
    let info = connection.server_info().clone();
    assert!(!info.product.is_empty(), "server product is empty");
    assert!(!info.version.is_empty(), "server version is empty");

    connection.ping().await.expect("first ping");
    connection.ping().await.expect("second ping");
    connection.close().await.expect("close");
}

/// A connection attempt that can't succeed is reported as [`DriverError::Connect`].
pub fn check_connect_failure<C>(result: Result<C, DriverError>) {
    match result {
        Err(DriverError::Connect(_)) => {}
        Err(other) => panic!("expected DriverError::Connect, got {other:?}"),
        Ok(_) => panic!("expected the connection attempt to fail"),
    }
}

/// Walks the catalog breadth-first, `max_depth` levels deep, skipping system nodes, and checks
/// the invariants every driver keeps: each child's path extends its parent's, labels are not
/// empty, and every node that claims a definition can produce one. Returns every node visited,
/// so driver tests can assert on what their backend should contain.
pub async fn walk_catalog(connection: &mut dyn Connection, max_depth: usize) -> Vec<CatalogNode> {
    let mut visited = Vec::new();
    let mut frontier = vec![CatalogPath::root()];
    for _ in 0..max_depth {
        let mut next = Vec::new();
        for path in frontier {
            let children = connection
                .children(&path)
                .await
                .unwrap_or_else(|error| panic!("children of {path}: {}", describe_error(&error)));
            for child in children {
                check_node(connection, &path, &child).await;
                if child.has_children && !child.system {
                    next.push(child.path.clone());
                }
                visited.push(child);
            }
        }
        frontier = next;
    }
    visited
}

async fn check_node(connection: &mut dyn Connection, parent: &CatalogPath, node: &CatalogNode) {
    assert_eq!(
        node.path.parent().as_ref(),
        Some(parent),
        "{} is not a child of {parent}",
        node.path
    );
    assert!(!node.label.is_empty(), "empty label at {}", node.path);
    if node.has_definition {
        let definition = connection
            .definition(&node.path)
            .await
            .unwrap_or_else(|error| {
                panic!("definition of {}: {}", node.path, describe_error(&error))
            });
        assert!(
            !definition.trim().is_empty(),
            "empty definition at {}",
            node.path
        );
    }
}

/// Paths the driver never produced are reported as [`DriverError::InvalidPath`], not as a
/// failed query or a panic.
pub async fn check_invalid_path(connection: &mut dyn Connection) {
    let nonsense = CatalogPath::root()
        .child(NodeKind::Column, "no such column")
        .child(NodeKind::Database, "no such database");

    let children = connection.children(&nonsense).await;
    assert!(
        matches!(children, Err(DriverError::InvalidPath(_))),
        "children of a nonsense path: {children:?}"
    );
    let definition = connection.definition(&nonsense).await;
    assert!(
        matches!(definition, Err(DriverError::InvalidPath(_))),
        "definition of a nonsense path: {definition:?}"
    );
}

const TABLE_ROWS: u64 = 1200;
const PAGE: usize = 500;

/// Runs `sql` to completion, reading every row, and returns what `finish` reports.
async fn run_to_end(connection: &mut dyn Connection, sql: &str) -> Option<u64> {
    let mut cursor = connection
        .execute(sql)
        .await
        .unwrap_or_else(|error| panic!("{sql}: {}", describe_error(&error)));
    while !cursor.is_exhausted() {
        cursor.next_page(PAGE).await.expect("read rows");
    }
    cursor.finish().await.expect("finish")
}

/// Statements run, results page lazily, NULL is distinct from text, a cursor can be
/// abandoned halfway, and errors point at the offending text. Uses only SQL that both
/// PostgreSQL and SQLite accept, and a temporary table.
pub async fn check_execution(connection: &mut dyn Connection) {
    run_to_end(
        connection,
        "CREATE TEMP TABLE dbdesk_conformance (id integer, name text)",
    )
    .await;
    let inserted = run_to_end(
        connection,
        "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 1200) \
         INSERT INTO dbdesk_conformance SELECT n, 'row ' || n FROM s",
    )
    .await;
    assert_eq!(inserted, Some(TABLE_ROWS), "rows inserted");

    check_paging(connection).await;
    check_null(connection).await;
    check_abandon(connection).await;
    check_error_position(connection).await;
    run_to_end(connection, "DROP TABLE dbdesk_conformance").await;
}

async fn check_paging(connection: &mut dyn Connection) {
    let mut cursor = connection
        .execute("SELECT id, name FROM dbdesk_conformance ORDER BY id")
        .await
        .expect("select");
    let names: Vec<&str> = cursor.columns().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "name"]);

    let mut page_sizes = Vec::new();
    let mut first_row = None;
    while !cursor.is_exhausted() {
        let page = cursor.next_page(PAGE).await.expect("page");
        first_row = first_row.or_else(|| page.first().cloned());
        page_sizes.push(page.len());
    }
    assert_eq!(
        first_row,
        Some(vec![Some("1".to_owned()), Some("row 1".to_owned())])
    );
    assert_eq!(
        page_sizes.iter().sum::<usize>(),
        1200,
        "pages: {page_sizes:?}"
    );
    assert!(
        page_sizes.iter().all(|size| *size <= PAGE),
        "pages: {page_sizes:?}"
    );
    cursor.finish().await.expect("finish select");
}

async fn check_null(connection: &mut dyn Connection) {
    let mut cursor = connection
        .execute("SELECT NULL AS no_value, '' AS empty_text")
        .await
        .expect("select null");
    let rows = cursor.next_page(PAGE).await.expect("null row");
    assert_eq!(rows, vec![vec![None, Some(String::new())]]);
    cursor.finish().await.expect("finish null");
}

async fn check_abandon(connection: &mut dyn Connection) {
    let mut cursor = connection
        .execute("SELECT id FROM dbdesk_conformance ORDER BY id")
        .await
        .expect("select");
    assert_eq!(cursor.next_page(10).await.expect("first rows").len(), 10);
    assert!(!cursor.is_exhausted());
    finish_ignoring_cancellation(cursor).await;
    connection
        .ping()
        .await
        .expect("usable after abandoning a result");
}

async fn finish_ignoring_cancellation(cursor: Box<dyn ResultCursor>) {
    match cursor.finish().await {
        Ok(_) | Err(DriverError::Statement { .. }) => {}
        Err(other) => panic!("finishing an abandoned cursor: {other:?}"),
    }
}

async fn check_error_position(connection: &mut dyn Connection) {
    // A syntax error, because that is where both engines report a position (SQLite gives
    // none for unknown tables or columns).
    let sql = "SELECT id FROM dbdesk_conformance WHERE WHERE id = 1";
    let expected = sql.rfind("WHERE").expect("second WHERE");
    let error = match connection.execute(sql).await {
        Ok(cursor) => {
            let mut cursor = cursor;
            let read = cursor.next_page(PAGE).await;
            match (read, cursor.finish().await) {
                (Err(error), _) | (Ok(_), Err(error)) => error,
                (Ok(_), Ok(_)) => panic!("{sql} succeeded"),
            }
        }
        Err(error) => error,
    };
    match error {
        DriverError::Statement { position, .. } => {
            assert_eq!(position, Some(expected), "error position in {sql:?}");
        }
        other => panic!("expected a statement error, got {other:?}"),
    }
    connection.ping().await.expect("usable after an error");
}

/// How long `check_cancel` waits before cancelling, and the most the whole statement may
/// take once cancelled. Generous so slow CI machines don't flake.
const CANCEL_AFTER: Duration = Duration::from_millis(200);
const CANCEL_DEADLINE: Duration = Duration::from_secs(10);

/// A statement that would run far longer than [`CANCEL_DEADLINE`] stops promptly when
/// cancelled from another task, with a statement error, and the connection stays usable.
pub async fn check_cancel(connection: &mut dyn Connection, long_running_sql: &str) {
    let handle = connection.cancel_handle();
    let started = Instant::now();
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(CANCEL_AFTER).await;
        handle.cancel().await
    });

    let result = async {
        let mut cursor = connection.execute(long_running_sql).await?;
        while !cursor.is_exhausted() {
            cursor.next_page(PAGE).await?;
        }
        cursor.finish().await
    }
    .await;

    assert!(
        matches!(result, Err(DriverError::Statement { .. })),
        "cancelled statement: {result:?}"
    );
    assert!(
        started.elapsed() < CANCEL_DEADLINE,
        "cancel took {:?}",
        started.elapsed()
    );
    canceller
        .await
        .expect("cancel task")
        .expect("cancel request");
    connection.ping().await.expect("usable after cancel");
}
