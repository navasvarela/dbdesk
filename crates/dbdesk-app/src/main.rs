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
mod hazard_dialog;
mod log_console;
mod state;
mod telemetry;
mod theme;
mod workspace;

use std::sync::Arc;

use anyhow::{Context as _, anyhow};
use dbdesk_engine::EngineConfig;
use dbdesk_store::{APP_NAME, AppPaths, DataSourceFile, Keychain};

use crate::app::DbDeskApp;

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
