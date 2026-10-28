//! The dbdesk desktop application.

mod about_dialog;
mod app;
mod brand;
mod catalog_tree;
mod commands;
mod console;
mod consoles;
mod csv_import;
mod custom_actions;
mod custom_files;
mod data_source_dialog;
mod data_source_form;
mod database_brand;
mod definition_window;
mod delete_dialog;
mod explorer;
mod export_dialog;
mod export_sources_dialog;
mod file_picker;
mod fonts;
mod hazard_dialog;
mod history_window;
mod host_key_prompt;
mod icons;
mod import_dialog;
mod keymap;
mod keymap_window;
mod log_console;
#[cfg(target_os = "macos")]
mod macos;
mod mcp;
mod mcp_client_config;
mod mcp_settings;
mod menu_bar;
mod native_menu;
mod object_dialog;
mod parameters_dialog;
mod password_prompt;
mod project_dialog;
mod project_settings;
mod relaunch;
mod rollback_dialog;
#[cfg(test)]
mod screenshots;
mod settings;
mod settings_window;
mod sql_files;
mod state;
mod system_look;
mod telemetry;
#[cfg(test)]
mod test_support;
mod theme;
mod title_bar;
mod unsaved_dialog;
mod update_dialog;
mod updates;
mod version;
mod welcome;
mod widgets;
mod workspace;

use std::sync::Arc;

use anyhow::{Context as _, anyhow};
use dbdesk_engine::{EngineConfig, GithubReleases, PackagerInstaller};
use dbdesk_store::{
    APP_NAME, AppPaths, ConsoleFiles, DataSourceFile, HistoryFile, Keychain, KeymapFile,
    ProjectFile, SettingsFile,
};

use crate::app::DbDeskApp;

/// Also the size UI tests run at, so they see the layout a new user sees.
const DEFAULT_WINDOW_SIZE: [f32; 2] = [1440.0, 900.0];

fn main() -> anyhow::Result<()> {
    // Before anything else: `--version` must work without a display, a home directory or a
    // config folder, because the release workflow smoke-tests packaged builds with it; and
    // `mcp` must not open a window.
    let request = version::CliRequest::parse(std::env::args().skip(1));
    if request.answer() {
        return Ok(());
    }
    let files = match request {
        version::CliRequest::Mcp { project } => return mcp::run(&project),
        version::CliRequest::Invalid(problem) => {
            anyhow::bail!("{problem}\n\n{}", version::CliRequest::usage())
        }
        version::CliRequest::Run { files } => files,
        version::CliRequest::Version | version::CliRequest::Help => Vec::new(),
    };
    // Relative to where dbdesk was started, which the app no longer knows once it runs.
    let files: Vec<_> = files
        .into_iter()
        .map(|file| std::path::absolute(&file).unwrap_or(file))
        .collect();
    let paths = AppPaths::discover()
        .context("the OS reports no home directory, so dbdesk has nowhere to keep its files")?;
    let telemetry = telemetry::init(&paths.logs_dir())?;
    let _file_guard = telemetry.file_guard;
    let log_feed = telemetry.log_feed;
    let engine_config = EngineConfig {
        data_sources: DataSourceFile::new(paths.data_sources_file()),
        projects: ProjectFile::new(paths.projects_file()),
        secrets: Arc::new(Keychain),
        consoles: ConsoleFiles::new(paths.consoles_dir()),
        history: HistoryFile::new(paths.history_file()),
        keymap: KeymapFile::new(paths.keymap_file()),
        settings: SettingsFile::new(paths.settings_file()),
        custom_files: paths.custom_files(),
        releases: Arc::new(GithubReleases::new(
            updates::RELEASES_REPOSITORY,
            &format!("dbdesk/{}", version::VERSION),
        )),
        updater: Arc::new(PackagerInstaller::new(
            updates::UPDATE_MANIFEST_URL,
            updates::UPDATE_PUBLIC_KEY,
            version::VERSION,
        )),
        known_hosts: paths.ssh_known_hosts_file(),
        idle_ping_after: dbdesk_engine::IDLE_PING_AFTER,
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("dbdesk")
        .with_app_id(APP_NAME)
        .with_inner_size(DEFAULT_WINDOW_SIZE);
    if let Some(icon) = brand::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    // The content runs under a transparent title bar, and dbdesk draws its own top strip
    // (`title_bar`) around the window buttons. egui spells winit's
    // `with_titlebar_transparent(true)` as `with_titlebar_shown(false)`.
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(true)
        .with_title_shown(false)
        .with_titlebar_shown(false);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            let app = DbDeskApp::new(cc, log_feed, engine_config)?.with_files_to_open(files);
            #[cfg(target_os = "macos")]
            let app = app
                .with_native_menu(native_menu::NativeMenu::install(&cc.egui_ctx))
                .with_integrated_title_bar()
                .with_system_look(system_look::SystemLook::install(&cc.egui_ctx));
            Ok(Box::new(app))
        }),
    )
    // eframe's error wraps winit/GL errors that aren't `Sync`, so it can't become an
    // `anyhow::Error` directly.
    .map_err(|error| anyhow!("dbdesk stopped with an error: {error}"))?;
    relaunch::start_if_requested();
    Ok(())
}
