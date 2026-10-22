//! Behaviour every driver must share, run by each driver crate against its own backend.
//! See `docs/testing.md`. These functions panic on failure, like any test assertion.

#![allow(clippy::expect_used)]

use crate::{CatalogNode, CatalogPath, Connection, DriverError, NodeKind, describe_error};

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
