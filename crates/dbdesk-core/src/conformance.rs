//! Behaviour every driver must share, run by each driver crate against its own backend.
//! See `docs/testing.md`. These functions panic on failure, like any test assertion.

#![allow(clippy::expect_used)]

use std::time::{Duration, Instant};

use crate::{
    CatalogNode, CatalogPath, Connection, DriverError, Identifier, NodeKind, RejectionKind,
    RelationName, ResultCursor, TransactionStatus, describe_error,
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

/// The transaction status follows `BEGIN`, `COMMIT` and `ROLLBACK`, committed changes stay,
/// rolled back ones don't, and a failure inside a transaction leaves it open (PostgreSQL also
/// marks it failed) until it is rolled back.
pub async fn check_transactions(connection: &mut dyn Connection) {
    assert_eq!(
        status(connection).await,
        TransactionStatus::Idle,
        "at first"
    );
    run_to_end(
        connection,
        "CREATE TEMP TABLE dbdesk_transactions (n integer)",
    )
    .await;

    run_to_end(connection, "BEGIN").await;
    assert_eq!(
        status(connection).await,
        TransactionStatus::InTransaction,
        "after BEGIN"
    );
    run_to_end(connection, "INSERT INTO dbdesk_transactions VALUES (1)").await;
    run_to_end(connection, "COMMIT").await;
    assert_eq!(
        status(connection).await,
        TransactionStatus::Idle,
        "after COMMIT"
    );

    run_to_end(connection, "BEGIN").await;
    run_to_end(connection, "INSERT INTO dbdesk_transactions VALUES (2)").await;
    let failed = connection
        .execute("SELECT * FROM dbdesk_no_such_table")
        .await;
    assert!(failed.is_err(), "selecting from a missing table");
    assert_ne!(
        status(connection).await,
        TransactionStatus::Idle,
        "after a failure"
    );
    run_to_end(connection, "ROLLBACK").await;
    assert_eq!(
        status(connection).await,
        TransactionStatus::Idle,
        "after ROLLBACK"
    );

    let mut cursor = connection
        .execute("SELECT n FROM dbdesk_transactions")
        .await
        .expect("select");
    let rows = cursor.next_page(PAGE).await.expect("rows");
    cursor.finish().await.expect("finish");
    assert_eq!(
        rows,
        vec![vec![Some("1".to_owned())]],
        "only the committed row"
    );
    run_to_end(connection, "DROP TABLE dbdesk_transactions").await;
}

/// Tables resolve as statements would (unquoted names ignoring case), with their columns in
/// table order and their primary key in key order; views and missing tables are `None`.
pub async fn check_describe_table(connection: &mut dyn Connection) {
    run_to_end(
        connection,
        "CREATE TEMP TABLE dbdesk_describe (id integer, part text, note text, \
         PRIMARY KEY (part, id))",
    )
    .await;
    run_to_end(connection, "CREATE TEMP TABLE dbdesk_keyless (n integer)").await;
    run_to_end(
        connection,
        "CREATE TEMP VIEW dbdesk_describe_view AS SELECT 1 AS one",
    )
    .await;
    let unquoted = |name: &str| RelationName {
        schema: None,
        name: Identifier {
            name: name.to_owned(),
            quoted: false,
        },
    };
    let described = connection
        .describe_table(&unquoted("DBDESK_DESCRIBE"))
        .await
        .expect("describe")
        .expect("the table exists");
    assert_eq!(described.name, "dbdesk_describe");
    assert!(!described.schema.is_empty());
    assert_eq!(described.columns, ["id", "part", "note"]);
    assert_eq!(described.primary_key, ["part", "id"]);
    let keyless = connection
        .describe_table(&unquoted("dbdesk_keyless"))
        .await
        .expect("describe")
        .expect("the table exists");
    assert!(keyless.primary_key.is_empty());
    for not_a_table in ["dbdesk_describe_view", "dbdesk_no_such_table"] {
        let found = connection
            .describe_table(&unquoted(not_a_table))
            .await
            .expect("describe");
        assert_eq!(found, None, "{not_a_table}");
    }
    for drop in [
        "DROP VIEW dbdesk_describe_view",
        "DROP TABLE dbdesk_keyless",
        "DROP TABLE dbdesk_describe",
    ] {
        run_to_end(connection, drop).await;
    }
}

/// Validating prepares without running: an accepted `INSERT` inserts nothing, the connection's
/// temporary tables are visible, unknown names and syntax errors are told apart (syntax errors
/// with their position), and no transaction is left open.
pub async fn check_validate(connection: &mut dyn Connection) {
    run_to_end(connection, "CREATE TEMP TABLE dbdesk_validate (n integer)").await;
    for accepted in [
        "SELECT n FROM dbdesk_validate WHERE n > 1",
        "INSERT INTO dbdesk_validate VALUES (1)",
        "DELETE FROM dbdesk_validate",
    ] {
        let rejection = connection.validate(accepted).await.expect("validate");
        assert_eq!(rejection, None, "{accepted}");
    }
    let mut cursor = connection
        .execute("SELECT count(*) FROM dbdesk_validate")
        .await
        .expect("count");
    let rows = cursor.next_page(PAGE).await.expect("read the count");
    cursor.finish().await.expect("finish");
    assert_eq!(
        rows,
        vec![vec![Some("0".to_owned())]],
        "nothing was inserted"
    );

    for (unknown, sql) in [
        ("table", "SELECT * FROM dbdesk_no_such_table"),
        ("column", "SELECT missing FROM dbdesk_validate"),
    ] {
        let rejection = connection
            .validate(sql)
            .await
            .expect("validate")
            .unwrap_or_else(|| panic!("an unknown {unknown} is rejected"));
        assert_eq!(rejection.kind, RejectionKind::UnknownName, "{rejection:?}");
        assert!(!rejection.message.is_empty());
    }
    let sql = "SELECT n n n FROM dbdesk_validate";
    let rejection = connection
        .validate(sql)
        .await
        .expect("validate")
        .expect("a syntax error is rejected");
    assert_eq!(rejection.kind, RejectionKind::Syntax, "{rejection:?}");
    assert_eq!(
        rejection.position,
        Some("SELECT n n ".len()),
        "{rejection:?}"
    );
    assert_eq!(status(connection).await, TransactionStatus::Idle);
    run_to_end(connection, "DROP TABLE dbdesk_validate").await;
}

async fn status(connection: &mut dyn Connection) -> TransactionStatus {
    connection
        .transaction_status()
        .await
        .expect("transaction status")
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
