//! The dbdesk desktop application.

mod app;
mod log_console;
mod state;
mod telemetry;
mod workspace;

use anyhow::anyhow;
use dbdesk_store::{APP_NAME, AppPaths};

use crate::app::DbDeskApp;

const DEFAULT_WINDOW_SIZE: [f32; 2] = [1280.0, 800.0];

fn main() -> anyhow::Result<()> {
    let paths = AppPaths::discover();
    let logs_dir = paths.as_ref().map(AppPaths::logs_dir);
    let telemetry = telemetry::init(logs_dir.as_deref())?;
    let _file_guard = telemetry.file_guard;
    let log_feed = telemetry.log_feed;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("dbdesk")
            .with_app_id(APP_NAME)
            .with_inner_size(DEFAULT_WINDOW_SIZE),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(DbDeskApp::new(cc, log_feed)?))),
    )
    // eframe's error wraps winit/GL errors that aren't `Sync`, so it can't become an
    // `anyhow::Error` directly.
    .map_err(|error| anyhow!("dbdesk stopped with an error: {error}"))
}
