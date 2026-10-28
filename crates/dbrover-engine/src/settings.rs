//! Reading and writing `settings.toml` on the blocking pool.

use dbrover_store::{HistorySettings, Settings, SettingsFile};

use crate::Event;
use crate::config_files::blocking;

/// Reads the settings and reports them as [`Event::SettingsLoaded`]. A broken file is
/// reported, not replaced: the UI keeps the defaults and shows why.
pub(crate) async fn load(file: &SettingsFile) -> Event {
    let task_file = file.clone();
    let result = blocking(move || task_file.load()).await;
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
    match blocking(move || task_file.save(&settings)).await {
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

/// The history's retention in what [`load`] or [`save`] read; `None` if the file couldn't be
/// read, so a broken file never prunes by the defaults.
pub(crate) fn history_retention(loaded: &Event) -> Option<HistorySettings> {
    match loaded {
        Event::SettingsLoaded {
            result: Ok(settings),
            ..
        } => Some(settings.history),
        _ => None,
    }
}
