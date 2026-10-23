use std::time::Instant;

use dbdesk_core::{CatalogPath, Connection, describe_error};

use super::SessionContext;
use crate::Event;

pub(super) async fn children(
    context: &SessionContext,
    connection: &mut dyn Connection,
    path: CatalogPath,
) {
    let name = context.name();
    let started = Instant::now();
    let result = connection.children(&path).await;
    match &result {
        Ok(nodes) => tracing::debug!(
            data_source = %name,
            elapsed_ms = started.elapsed().as_millis(),
            "loaded {} nodes under {path}",
            nodes.len()
        ),
        Err(error) => tracing::warn!(
            data_source = %name,
            "could not load {path}: {}",
            describe_error(error)
        ),
    }
    context.send(Event::ChildrenLoaded {
        id: context.data_source.id,
        path,
        result: result.map_err(|error| describe_error(&error)),
    });
}

pub(super) async fn definition(
    context: &SessionContext,
    connection: &mut dyn Connection,
    path: CatalogPath,
) {
    let result = connection.definition(&path).await;
    if let Err(error) = &result {
        tracing::warn!(
            data_source = %context.name(),
            "could not load the definition of {path}: {}",
            describe_error(error)
        );
    }
    context.send(Event::DefinitionLoaded {
        id: context.data_source.id,
        path,
        result: result.map_err(|error| describe_error(&error)),
    });
}
