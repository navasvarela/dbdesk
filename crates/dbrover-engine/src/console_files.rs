//! Reading and writing query console files on the blocking pool.

use dbrover_core::describe_error;
use dbrover_store::{ConsoleFiles, ConsoleKey, ConsoleStoreError, SavedConsole};
use tokio::task::JoinError;

use crate::Event;

#[derive(Debug, thiserror::Error)]
enum ConsoleFileError {
    #[error(transparent)]
    Store(#[from] ConsoleStoreError),
    #[error("a background task failed")]
    Task(#[source] JoinError),
}

/// Reads every console file. Files that can't be read are logged and left out, so the UI
/// drops their tabs and the files stay untouched on disk.
pub(crate) async fn load(files: &ConsoleFiles) -> Event {
    let files = files.clone();
    let loaded = tokio::task::spawn_blocking(move || files.load_all())
        .await
        .map_err(ConsoleFileError::Task)
        .and_then(|result| result.map_err(ConsoleFileError::from));
    let consoles: Vec<SavedConsole> = match loaded {
        Ok(results) => results
            .into_iter()
            .filter_map(|result| {
                result
                    .inspect_err(|error| {
                        tracing::error!("could not load a console: {}", describe_error(error));
                    })
                    .ok()
            })
            .collect(),
        Err(error) => {
            tracing::error!("could not load consoles: {}", describe_error(&error));
            Vec::new()
        }
    };
    Event::ConsolesLoaded(consoles)
}

/// Writes the console's file, answering with [`Event::ConsoleSaveFailed`] if that fails.
pub(crate) async fn save(files: &ConsoleFiles, key: ConsoleKey, text: String) -> Option<Event> {
    let files = files.clone();
    let task_key = key.clone();
    let result = tokio::task::spawn_blocking(move || files.save(&task_key, &text)).await;
    let error = flatten(result)?;
    tracing::error!(console = %key.name, "could not save the console: {error}");
    Some(Event::ConsoleSaveFailed { key, error })
}

/// Removes the console's file. A leftover file is harmless, so a failure is only logged.
pub(crate) async fn delete(files: &ConsoleFiles, key: ConsoleKey) {
    let files = files.clone();
    let task_key = key.clone();
    let result = tokio::task::spawn_blocking(move || files.delete(&task_key)).await;
    if let Some(error) = flatten(result) {
        tracing::warn!(console = %key.name, "could not remove the console file: {error}");
    }
}

/// The description of whatever went wrong, if anything did.
fn flatten(result: Result<Result<(), ConsoleStoreError>, JoinError>) -> Option<String> {
    match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(describe_error(&error)),
        Err(error) => Some(describe_error(&error)),
    }
}
