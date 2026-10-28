//! Reading and creating `keymap.toml` on the blocking pool.

use dbrover_store::KeymapFile;

use crate::Event;
use crate::config_files::blocking;

/// Reads the keymap and reports it as [`Event::KeymapLoaded`]. A broken file is reported, not
/// replaced: the UI keeps the default bindings and shows why.
pub(crate) async fn load(file: &KeymapFile) -> Event {
    let task_file = file.clone();
    let result = blocking(move || task_file.load()).await;
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
    match blocking(move || task_file.create(&text)).await {
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
