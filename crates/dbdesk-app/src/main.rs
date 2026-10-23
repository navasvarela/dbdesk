//! The dbdesk desktop application.

mod app;
mod brand;
mod catalog_tree;
mod console;
mod consoles;
mod data_source_dialog;
mod data_source_form;
mod definition_window;
mod delete_dialog;
mod explorer;
mod export_dialog;
mod file_picker;
mod fonts;
mod hazard_dialog;
mod history_window;
mod icons;
mod keymap;
mod keymap_window;
mod log_console;
mod rollback_dialog;
#[cfg(test)]
mod screenshots;
mod state;
mod telemetry;
#[cfg(test)]
mod test_support;
mod theme;
mod welcome;
mod widgets;
mod workspace;

use std::sync::Arc;

use anyhow::{Context as _, anyhow};
use dbdesk_engine::EngineConfig;
use dbdesk_store::{
    APP_NAME, AppPaths, ConsoleFiles, DataSourceFile, HistoryFile, Keychain, KeymapFile,
};

use crate::app::DbDeskApp;

/// Also the size UI tests run at, so they see the layout a new user sees.
const DEFAULT_WINDOW_SIZE: [f32; 2] = [1280.0, 800.0];

fn main() -> anyhow::Result<()> {
    let paths = AppPaths::discover()
        .context("the OS reports no home directory, so dbdesk has nowhere to keep its files")?;
    let telemetry = telemetry::init(&paths.logs_dir())?;
    let _file_guard = telemetry.file_guard;
    let log_feed = telemetry.log_feed;
    let engine_config = EngineConfig {
        data_sources: DataSourceFile::new(paths.data_sources_file()),
        secrets: Arc::new(Keychain),
        consoles: ConsoleFiles::new(paths.consoles_dir()),
        history: HistoryFile::new(paths.history_file()),
        keymap: KeymapFile::new(paths.keymap_file()),
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("dbdesk")
        .with_app_id(APP_NAME)
        .with_inner_size(DEFAULT_WINDOW_SIZE);
    if let Some(icon) = brand::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(DbDeskApp::new(cc, log_feed, engine_config)?))),
    )
    // eframe's error wraps winit/GL errors that aren't `Sync`, so it can't become an
    // `anyhow::Error` directly.
    .map_err(|error| anyhow!("dbdesk stopped with an error: {error}"))
}
