//! Reading and creating `keymap.toml` on the blocking pool.

use dbdesk_core::describe_error;
use dbdesk_store::{KeymapError, KeymapFile};
use tokio::task::JoinError;

use crate::Event;

/// Reads the keymap and reports it as [`Event::KeymapLoaded`]. A broken file is reported, not
/// replaced: the UI keeps the default bindings and shows why.
pub(crate) async fn load(file: &KeymapFile) -> Event {
    let task_file = file.clone();
    let loaded = tokio::task::spawn_blocking(move || task_file.load()).await;
    let result = flatten(loaded);
    if let Err(error) = &result {
        tracing::error!("could not load the keymap: {error}");
    }
    Event::KeymapLoaded {
        path: file.path().to_owned(),
        result,
    }
}

/// Writes a new keymap file with `text`, then reads it back.
pub(crate) async fn create(file: &KeymapFile, text: String) -> Event {
    let task_file = file.clone();
    let created = tokio::task::spawn_blocking(move || task_file.create(&text)).await;
    match flatten(created) {
        Ok(()) => {
            tracing::info!("created {}", file.path().display());
            load(file).await
        }
        Err(error) => {
            tracing::error!("could not create the keymap: {error}");
            Event::KeymapLoaded {
                path: file.path().to_owned(),
                result: Err(error),
            }
        }
    }
}

fn flatten<T>(result: Result<Result<T, KeymapError>, JoinError>) -> Result<T, String> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(describe_error(&error)),
        Err(error) => Err(describe_error(&error)),
    }
}
