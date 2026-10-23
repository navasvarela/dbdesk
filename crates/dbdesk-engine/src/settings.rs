//! Reading and writing `settings.toml` on the blocking pool.

use dbdesk_core::describe_error;
use dbdesk_store::{Settings, SettingsError, SettingsFile};
use tokio::task::JoinError;

use crate::Event;

/// Reads the settings and reports them as [`Event::SettingsLoaded`]. A broken file is
/// reported, not replaced: the UI keeps the defaults and shows why.
pub(crate) async fn load(file: &SettingsFile) -> Event {
    let task_file = file.clone();
    let result = flatten(tokio::task::spawn_blocking(move || task_file.load()).await);
    if let Err(error) = &result {
        tracing::error!("could not load the settings: {error}");
    }
    Event::SettingsLoaded {
        path: file.path().to_owned(),
        result,
    }
}

/// Writes `settings`, then reports what the file holds.
pub(crate) async fn save(file: &SettingsFile, settings: Settings) -> Event {
    let task_file = file.clone();
    let saved = tokio::task::spawn_blocking(move || task_file.save(&settings)).await;
    match flatten(saved) {
        Ok(()) => load(file).await,
        Err(error) => {
            tracing::error!("could not save the settings: {error}");
            Event::SettingsLoaded {
                path: file.path().to_owned(),
                result: Err(error),
            }
        }
    }
}

fn flatten<T>(result: Result<Result<T, SettingsError>, JoinError>) -> Result<T, String> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(describe_error(&error)),
        Err(error) => Err(describe_error(&error)),
    }
}
