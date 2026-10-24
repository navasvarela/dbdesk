use std::path::PathBuf;
use std::time::Instant;

use dbdesk_core::{CatalogPath, DataSourceId};
use dbdesk_engine::{Command, EngineConfig, EngineError, EngineHandle, Event, Repaint};
use dbdesk_store::{ConsoleKey, ExportFormat};
use egui::{Frame, Margin, RichText, ThemePreference};
use egui_dock::{DockArea, DockState};

use crate::about_dialog;
use crate::console::{ConsoleAction, FormatScope};
use crate::consoles::Consoles;
use crate::data_source_dialog::{DataSourceDialog, DialogAction};
use crate::definition_window::DefinitionWindow;
use crate::delete_dialog::{DeleteChoice, DeleteConfirmation};
use crate::explorer::{ExplorerAction, ExplorerOptions};
use crate::export_dialog;
use crate::history_window::{HistoryAction, HistoryWindow};
use crate::keymap::{self, Keymap};
use crate::keymap_window::{self, KeymapWindowAction};
use crate::log_console::{LogConsole, LogFeed};
use crate::rollback_dialog::{Closing, RollbackChoice, RollbackConfirmation};
use crate::settings::AppSettings;
use crate::settings_window::{self, SettingsWindowAction};
use crate::state::{AppState, EngineStatus};
use crate::theme::{self, Palette};
use crate::update_dialog;
use crate::updates::{UpdateStatus, Updates};
use crate::workspace::{self, Panels, Tab};
use crate::{fonts, icons, widgets};

/// Where the dock layout is saved. Versioned so a change to the default layout reaches people
/// who already have one saved: layouts under an older key are ignored once, and the default
/// takes their place. Version 2: the explorer became a full-height column, and layouts saved
/// before it could give the explorer four fifths of the window. Open console tabs aren't
/// restored after a version change, but their files are, under Open saved console.
const LAYOUT_STORAGE_KEY: &str = "dock_layout_v2";

pub struct DbDeskApp {
    /// `None` once the engine has been shut down, at exit or after it stopped unexpectedly.
    engine: Option<EngineHandle>,
    state: AppState,
    dock: DockState<Tab>,
    log_console: LogConsole,
    explorer_options: ExplorerOptions,
    data_source_dialog: Option<DataSourceDialog>,
    delete_confirmation: Option<DeleteConfirmation>,
    definition_window: Option<DefinitionWindow>,
    history_window: Option<HistoryWindow>,
    rollback_confirmation: Option<RollbackConfirmation>,
    /// Set once quitting may roll back open transactions, so the close isn't asked again.
    quit_confirmed: bool,
    keymap: Keymap,
    /// Where `keymap.toml` is, once the engine has reported it.
    keymap_path: Option<PathBuf>,
    keymap_window_open: bool,
    about_open: bool,
    settings: AppSettings,
    updates: Updates,
    /// Asks where to export rows. The platform's save dialog, except in tests.
    pick_export_path: Box<dyn Fn(ExportFormat) -> Option<PathBuf>>,
    consoles: Consoles,
    /// Set on first launch until the window has been asked to maximize. After that, eframe
    /// restores whatever size the user left it at.
    maximize_window: bool,
}

impl DbDeskApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        log_feed: LogFeed,
        engine_config: EngineConfig,
    ) -> Result<Self, EngineError> {
        let saved_layout: Option<DockState<Tab>> = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, LAYOUT_STORAGE_KEY));
        // No saved layout means a first launch (or a layout version change): maximize the
        // window rather than keep the small default size.
        let maximize_window = saved_layout.is_none();
        let dock = saved_layout.unwrap_or_else(workspace::default_layout);
        let engine = EngineHandle::start(engine_config, EguiRepaint(cc.egui_ctx.clone()))?;
        fonts::install(&cc.egui_ctx);
        theme::install(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);

        Ok(Self {
            engine: Some(engine),
            state: AppState::default(),
            dock,
            log_console: LogConsole::new(log_feed, &cc.egui_ctx),
            explorer_options: ExplorerOptions::default(),
            data_source_dialog: None,
            delete_confirmation: None,
            definition_window: None,
            history_window: None,
            rollback_confirmation: None,
            quit_confirmed: false,
            pick_export_path: Box::new(export_dialog::pick_export_path),
            keymap: Keymap::default(),
            keymap_path: None,
            keymap_window_open: false,
            about_open: false,
            settings: AppSettings::default(),
            updates: Updates::default(),
            consoles: Consoles::default(),
            maximize_window,
        })
    }

    /// The open consoles, for tests that look at what a console drew.
    #[cfg(test)]
    pub fn open_consoles(&self) -> impl Iterator<Item = &crate::console::QueryConsole> {
        self.consoles.open_consoles()
    }

    fn drain_engine_events(&mut self) {
        loop {
            let Some(engine) = &mut self.engine else {
                return;
            };
            match engine.try_recv() {
                Ok(Some(event)) => self.apply(&event),
                Ok(None) => return,
                Err(error) => {
                    tracing::error!("{error}");
                    self.shut_down_engine();
                    return;
                }
            }
        }
    }

    fn apply(&mut self, event: &Event) {
        self.state.apply(event);
        if self
            .data_source_dialog
            .as_mut()
            .is_some_and(|dialog| dialog.apply(event))
        {
            self.data_source_dialog = None;
        }
        if let Some(window) = &mut self.definition_window {
            window.apply(event);
        }
        if let Some(window) = &mut self.history_window {
            window.apply(event);
        }
        // The run's statements are recorded by now; show them.
        if matches!(event, Event::RunFinished { .. })
            && let Some(window) = &self.history_window
        {
            self.send(window.search_command());
        }
        self.consoles.apply(event);
        self.settings.apply(event);
        self.updates.apply(event);
        if matches!(event, Event::SettingsLoaded { .. }) {
            let check = self.updates.start(self.settings.settings());
            self.send_if(check);
        }
        if let Event::KeymapLoaded { path, result } = event {
            self.keymap = match result {
                Ok(bindings) => Keymap::from_file(bindings),
                Err(error) => Keymap::unreadable(error.clone()),
            };
            self.keymap_path = Some(path.clone());
        }
        if matches!(event, Event::ConsolesLoaded(_)) {
            self.restore_console_tabs();
        }
    }

    /// Reopens the consoles whose tabs the saved layout holds. The engine reports the data
    /// sources before the console files, so both are known by now.
    fn restore_console_tabs(&mut self) {
        let Self {
            dock,
            consoles,
            state,
            ..
        } = self;
        workspace::restore_consoles(dock, |key| {
            state
                .data_source(key.data_source)
                .and_then(|entry| consoles.reopen(&entry.data_source, key))
                .is_some()
        });
    }

    fn send_all(&self, commands: impl IntoIterator<Item = Command>) {
        for command in commands {
            self.send(command);
        }
    }

    /// Writes the consoles whose edits have settled, and wakes up when the next ones will.
    fn save_consoles(&mut self, ctx: &egui::Context) {
        let (saves, next_due) = self.consoles.due_saves(Instant::now());
        self.send_all(saves);
        if let Some(delay) = next_due {
            ctx.request_repaint_after(delay);
        }
    }

    fn send_if(&self, command: Option<Command>) {
        if let Some(command) = command {
            self.send(command);
        }
    }

    fn send(&self, command: Command) {
        let Some(engine) = &self.engine else {
            tracing::warn!("the engine is not running; ignoring {command:?}");
            return;
        };
        if let Err(error) = engine.send(command) {
            tracing::error!("{error}");
        }
    }

    /// Maximizes the window, once, on first launch.
    ///
    /// Why not size it from the monitor: with several screens, egui reported another screen's
    /// size than the one the window opened on (a laptop's 1512×982 for a window on a 2560×1440
    /// display), so the window came out barely bigger and on the wrong screen. Maximizing lets
    /// the OS fill whichever screen the window is on.
    fn maximize_window_once(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.maximize_window) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
    }

    fn shut_down_engine(&mut self) {
        if let Some(engine) = self.engine.take()
            && let Err(error) = engine.shutdown()
        {
            tracing::error!("engine shutdown failed: {:#}", anyhow::Error::new(error));
        }
        self.state.engine_status = EngineStatus::Stopped;
    }

    fn handle_explorer_action(&mut self, action: ExplorerAction, ctx: &egui::Context) {
        match action {
            ExplorerAction::New(driver) => {
                self.data_source_dialog = Some(DataSourceDialog::create(driver));
            }
            ExplorerAction::Edit(id) => {
                if let Some(entry) = self.state.data_source(id) {
                    self.data_source_dialog = Some(DataSourceDialog::edit(&entry.data_source));
                }
            }
            ExplorerAction::Delete(id) => {
                if let Some(entry) = self.state.data_source(id) {
                    self.delete_confirmation = Some(DeleteConfirmation::new(&entry.data_source));
                }
            }
            ExplorerAction::Connect(id) => self.send(Command::Connect(id)),
            ExplorerAction::Disconnect(id) => self.close_connections(Closing::Disconnect(id), ctx),
            ExplorerAction::Expand(id, path) => {
                let needs_loading = self
                    .state
                    .catalog_mut(id)
                    .is_some_and(|catalog| catalog.expand(&path));
                self.load_children_if(needs_loading, id, path);
            }
            ExplorerAction::Collapse(id, path) => {
                if let Some(catalog) = self.state.catalog_mut(id) {
                    catalog.collapse(&path);
                }
            }
            ExplorerAction::Refresh(id, path) => {
                let needs_loading = self
                    .state
                    .catalog_mut(id)
                    .is_some_and(|catalog| catalog.refresh(&path));
                self.load_children_if(needs_loading, id, path);
            }
            ExplorerAction::ShowDefinition { id, path, title } => {
                self.definition_window = Some(DefinitionWindow::new(id, path.clone(), title));
                self.send(Command::LoadDefinition { id, path });
            }
            ExplorerAction::NewConsole(id) => {
                if let Some(entry) = self.state.data_source(id) {
                    let console = self.consoles.open(&entry.data_source, String::new());
                    workspace::open_console_tab(&mut self.dock, console);
                }
            }
            ExplorerAction::OpenSavedConsole(key) => {
                let reopened = self
                    .state
                    .data_source(key.data_source)
                    .and_then(|entry| self.consoles.reopen(&entry.data_source, &key));
                if let Some(console) = reopened {
                    workspace::open_console_tab(&mut self.dock, console);
                }
            }
            ExplorerAction::OpenData(id, path) => {
                let opened = self
                    .state
                    .data_source(id)
                    .and_then(|entry| self.consoles.open_data(&entry.data_source, &path));
                if let Some((console, command)) = opened {
                    workspace::open_console_tab(&mut self.dock, console);
                    self.send(command);
                }
            }
        }
    }

    /// Loads catalog levels completion asked for, without expanding them in the explorer.
    fn load_catalog(&mut self, id: DataSourceId, paths: Vec<CatalogPath>) {
        for path in paths {
            let needs_loading = self
                .state
                .catalog_mut(id)
                .is_some_and(|catalog| catalog.request(&path));
            self.load_children_if(needs_loading, id, path);
        }
    }

    fn load_children_if(&self, needs_loading: bool, id: DataSourceId, path: CatalogPath) {
        if needs_loading {
            self.send(Command::LoadChildren { id, path });
        }
    }

    fn show_data_source_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self
            .data_source_dialog
            .as_mut()
            .and_then(|dialog| dialog.show(ctx))
        else {
            return;
        };
        match action {
            DialogAction::Test(data_source, password) => self.send(Command::TestConnection {
                data_source,
                password,
            }),
            DialogAction::Save(data_source, password) => self.send(Command::SaveDataSource {
                data_source,
                password,
            }),
            DialogAction::CreateSqliteDatabase(path) => {
                self.send(Command::CreateSqliteDatabase(path));
            }
            DialogAction::Close => self.data_source_dialog = None,
        }
    }

    fn show_delete_confirmation(&mut self, ctx: &egui::Context) {
        let Some(choice) = self
            .delete_confirmation
            .as_ref()
            .and_then(|confirmation| confirmation.show(ctx))
        else {
            return;
        };
        self.delete_confirmation = None;
        if let DeleteChoice::Confirmed(id) = choice {
            self.send(Command::DeleteDataSource(id));
        }
    }

    /// Does what `closing` asks, first asking whether to roll back the open transactions it
    /// would end, if there are any.
    fn close_connections(&mut self, closing: Closing, ctx: &egui::Context) {
        let rolled_back = self.consoles.rolled_back_by(&closing);
        if rolled_back.is_empty() {
            self.finish_closing(closing, ctx);
        } else if self.rollback_confirmation.is_none() {
            self.rollback_confirmation = Some(RollbackConfirmation::new(closing, rolled_back));
        }
    }

    fn finish_closing(&mut self, closing: Closing, ctx: &egui::Context) {
        match closing {
            Closing::Console(key) => {
                let commands = self.consoles.close(&key);
                self.send_all(commands);
                workspace::close_console_tab(&mut self.dock, &key);
            }
            Closing::Disconnect(id) => self.send(Command::Disconnect(id)),
            Closing::Quit => {
                self.quit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Holds the window open while quitting would roll back transactions the user hasn't
    /// agreed to lose.
    fn confirm_quit(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.viewport().close_requested()) || self.quit_confirmed {
            return;
        }
        if !self.consoles.rolled_back_by(&Closing::Quit).is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_connections(Closing::Quit, ctx);
        }
    }

    fn show_rollback_confirmation(&mut self, ctx: &egui::Context) {
        let Some(choice) = self
            .rollback_confirmation
            .as_ref()
            .and_then(|confirmation| confirmation.show(ctx))
        else {
            return;
        };
        let Some(confirmation) = self.rollback_confirmation.take() else {
            return;
        };
        if let RollbackChoice::Proceed = choice {
            self.finish_closing(confirmation.closing, ctx);
        }
    }

    fn export(&self, console: &ConsoleKey, format: ExportFormat) {
        if let Some(path) = (self.pick_export_path)(format) {
            self.send_if(self.consoles.export(console, format, path));
        }
    }

    fn open_history(&mut self, console: ConsoleKey) {
        let Some(entry) = self.state.data_source(console.data_source) else {
            return;
        };
        let (window, search) = HistoryWindow::open(console, entry.data_source.name.clone());
        self.history_window = Some(window);
        self.send(search);
    }

    fn show_history_window(&mut self, ctx: &egui::Context) {
        let Some(action) = self
            .history_window
            .as_mut()
            .and_then(|window| window.show(ctx))
        else {
            return;
        };
        match action {
            HistoryAction::Search(command) => self.send(command),
            HistoryAction::Insert { console, sql } => {
                self.consoles.insert_statement(&console, &sql);
            }
            HistoryAction::Close => self.history_window = None,
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("View", |ui| {
                ui.menu_button("Theme", theme_menu);
                if ui.button("Keymap…").clicked() {
                    self.keymap_window_open = true;
                }
                if ui.button("Settings…").clicked() {
                    self.settings.window_open = true;
                }
                if ui.button("Reset layout").clicked() {
                    self.dock = workspace::default_layout();
                    tracing::info!("layout reset");
                }
            });
            ui.menu_button("Code", |ui| self.code_menu(ui));
            ui.menu_button("Help", |ui| {
                if ui.button("Check for updates…").clicked() {
                    let check = self.updates.check_now();
                    self.send(check);
                }
                if ui.button("About dbdesk").clicked() {
                    self.about_open = true;
                }
            });
        });
    }

    fn show_about(&mut self, ctx: &egui::Context) {
        if self.about_open && about_dialog::show(ctx) {
            self.about_open = false;
        }
    }

    fn show_update_dialog(&mut self, ctx: &egui::Context) {
        if self.updates.dialog_open && update_dialog::show(ctx, self.updates.status()) {
            self.updates.dialog_open = false;
        }
    }

    /// Formatting, for the console whose tab has focus.
    fn code_menu(&mut self, ui: &mut egui::Ui) {
        let console = self.focused_console();
        for (label, scope, action) in [
            ("Format", FormatScope::Current, keymap::Action::Format),
            ("Format all", FormatScope::All, keymap::Action::FormatAll),
        ] {
            let mut button = egui::Button::new(label);
            if let Some(shortcut) = self.keymap.hint(ui.ctx(), action) {
                button = button.shortcut_text(shortcut);
            }
            let response = ui
                .add_enabled(console.is_some(), button)
                .on_hover_text(action.description())
                .on_disabled_hover_text("Select a query console first");
            if response.clicked()
                && let Some(console) = &console
            {
                self.consoles
                    .format(console, scope, &self.settings.format_options());
            }
        }
    }

    /// The console in the dock's focused tab, if that is a console.
    fn focused_console(&mut self) -> Option<ConsoleKey> {
        match self.dock.find_active_focused() {
            Some((_, Tab::QueryConsole(key))) => Some(key.clone()),
            _ => None,
        }
    }

    fn show_settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings.window_open {
            return;
        }
        match settings_window::show(ctx, &self.settings) {
            Some(SettingsWindowAction::Save(settings)) => {
                self.settings.set(settings.clone());
                self.send(Command::SaveSettings(settings));
            }
            Some(SettingsWindowAction::Reload) => self.send(Command::ReloadSettings),
            Some(SettingsWindowAction::Close) => self.settings.window_open = false,
            None => {}
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::for_visuals(ui.visuals());
        let dot = match self.state.engine_status {
            EngineStatus::Starting => palette.warning,
            EngineStatus::Running => palette.success,
            EngineStatus::Stopped => palette.error,
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("●").small().color(dot));
            ui.label(
                RichText::new(format!(
                    "Engine {} · {} connected",
                    self.state.engine_status.label(),
                    self.state.connected_count()
                ))
                .small()
                .color(palette.text_weak),
            );
            let problems = self.keymap.problems().len();
            if problems > 0
                && ui
                    .link(
                        RichText::new(match problems {
                            1 => "keymap.toml: 1 problem".to_owned(),
                            n => format!("keymap.toml: {n} problems"),
                        })
                        .small()
                        .color(palette.warning),
                    )
                    .clicked()
            {
                self.keymap_window_open = true;
            }
            if let UpdateStatus::Available { version, .. } = self.updates.status()
                && ui
                    .link(
                        RichText::new(format!("dbdesk {version} is available"))
                            .small()
                            .color(palette.primary),
                    )
                    .clicked()
            {
                self.updates.dialog_open = true;
            }
        });
    }

    fn show_keymap_window(&mut self, ctx: &egui::Context) {
        if !self.keymap_window_open {
            return;
        }
        match keymap_window::show(ctx, &self.keymap, self.keymap_path.as_deref()) {
            Some(KeymapWindowAction::Reload) => self.send(Command::ReloadKeymap),
            Some(KeymapWindowAction::Create) => self.send(Command::CreateKeymap {
                text: keymap::template(),
            }),
            Some(KeymapWindowAction::Close) => self.keymap_window_open = false,
            None => {}
        }
    }
}

impl eframe::App for DbDeskApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.maximize_window_once(ctx);
        self.drain_engine_events();
        self.log_console.drain();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The base colour shows in the gaps between the dock's islands.
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);
        let bar = Frame::new()
            .fill(ui.visuals().panel_fill)
            .inner_margin(Margin::symmetric(10, 2));
        egui::Panel::top("menu_bar")
            .frame(bar)
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status_bar")
            .frame(bar)
            .show(ui, |ui| self.status_bar(ui));

        let mut explorer_actions = Vec::new();
        let mut console_actions = Vec::new();
        let mut close_requests = Vec::new();
        let palette = Palette::for_visuals(ui.visuals());
        let lint = self.settings.lint_options();
        let (open_consoles, saved_consoles, recent_completions) =
            self.consoles.open_and_saved_mut();
        DockArea::new(&mut self.dock)
            .style(theme::dock_style(ui.style()))
            // Calmer tab bars: tabs close one at a time, and panels don't fold away.
            .show_leaf_collapse_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_inside(
                ui,
                &mut Panels {
                    palette,
                    log_console: &mut self.log_console,
                    data_sources: &self.state.data_sources,
                    saved_consoles,
                    explorer_options: &mut self.explorer_options,
                    explorer_actions: &mut explorer_actions,
                    consoles: open_consoles,
                    keymap: &self.keymap,
                    lint: &lint,
                    recent_completions,
                    console_actions: &mut console_actions,
                    close_requests: &mut close_requests,
                },
            );
        let ctx = ui.ctx().clone();
        for action in explorer_actions {
            self.handle_explorer_action(action, &ctx);
        }
        for (console, action) in console_actions {
            match action {
                ConsoleAction::ShowHistory => self.open_history(console),
                ConsoleAction::Export(format) => self.export(&console, format),
                ConsoleAction::LoadCatalog(paths) => self.load_catalog(console.data_source, paths),
                ConsoleAction::Format(scope) => {
                    self.consoles
                        .format(&console, scope, &self.settings.format_options());
                }
                action => {
                    let command = self.consoles.handle(&console, action);
                    self.send_if(command);
                }
            }
        }
        for console in close_requests {
            self.close_connections(Closing::Console(console), &ctx);
        }
        self.confirm_quit(&ctx);
        self.show_rollback_confirmation(&ctx);

        self.save_consoles(&ctx);
        let confirmed = self.consoles.show_hazard_confirmation(&ctx);
        self.send_if(confirmed);
        self.show_data_source_dialog(&ctx);
        self.show_delete_confirmation(&ctx);
        if self
            .definition_window
            .as_ref()
            .is_some_and(|window| !window.show(&ctx))
        {
            self.definition_window = None;
        }
        self.show_history_window(&ctx);
        self.show_keymap_window(&ctx);
        self.show_about(&ctx);
        self.show_update_dialog(&ctx);
        self.show_settings_window(&ctx);

        // Entries logged on the UI thread don't wake the UI (see `UiWaker`), so collect any
        // logged during this frame and ask for one more frame to show them.
        if self.log_console.drain() {
            ctx.request_repaint();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LAYOUT_STORAGE_KEY, &self.dock);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Sent before the engine stops: it handles every queued command before shutting down.
        let unsaved = self.consoles.unsaved();
        self.send_all(unsaved);
        self.shut_down_engine();
    }
}

/// Follow the OS, or force light or dark. egui keeps the choice with its own saved state.
fn theme_menu(ui: &mut egui::Ui) {
    let current = ui.ctx().options(|options| options.theme_preference);
    for (preference, glyph, label) in [
        (ThemePreference::System, icons::MONITOR, "Match system"),
        (ThemePreference::Light, icons::SUN, "Light"),
        (ThemePreference::Dark, icons::MOON, "Dark"),
    ] {
        let text = widgets::icon_text(ui, glyph, egui::Color32::PLACEHOLDER, label);
        let response = widgets::name(
            ui.selectable_label(current == preference, text),
            egui::WidgetType::RadioButton,
            label,
            true,
        );
        if response.clicked() {
            ui.ctx().set_theme(preference);
            ui.close();
        }
    }
}

/// Lets the engine wake egui without depending on it.
struct EguiRepaint(egui::Context);

impl Repaint for EguiRepaint {
    fn request_repaint(&self) {
        self.0.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use dbdesk_core::{DataSourceId, Password};
    use dbdesk_store::{
        ConsoleFiles, ConsoleName, DataSourceFile, HistoryFile, KeymapFile, SecretError,
        SecretStore, SettingsFile,
    };
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;
    use crate::log_console;
    use crate::test_support::FixedReleases;

    const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

    /// The tests never need a password, and must not touch the real keychain.
    struct NoSecrets;

    impl SecretStore for NoSecrets {
        fn password(&self, _: DataSourceId) -> Result<Option<Password>, SecretError> {
            Ok(None)
        }

        fn set_password(&self, _: DataSourceId, _: &Password) -> Result<(), SecretError> {
            Ok(())
        }

        fn delete_password(&self, _: DataSourceId) -> Result<(), SecretError> {
            Ok(())
        }
    }

    fn engine_config(dir: &Path) -> EngineConfig {
        engine_config_with_releases(dir, FixedReleases::current())
    }

    fn engine_config_with_releases(dir: &Path, releases: FixedReleases) -> EngineConfig {
        EngineConfig {
            data_sources: DataSourceFile::new(dir.join("data_sources.toml")),
            secrets: Arc::new(NoSecrets),
            consoles: ConsoleFiles::new(dir.join("consoles")),
            history: HistoryFile::new(dir.join("history.sqlite")),
            keymap: KeymapFile::new(dir.join("keymap.toml")),
            settings: SettingsFile::new(dir.join("settings.toml")),
            releases: Arc::new(releases),
        }
    }

    /// The app's own default window size, so tests see the layout a new user sees. At
    /// egui_kittest's 800x600 the console is too narrow for the results summary row.
    fn harness_at_default_size() -> egui_kittest::HarnessBuilder<DbDeskApp> {
        Harness::builder().with_size(crate::DEFAULT_WINDOW_SIZE)
    }

    fn start_app(dir: &Path, log_feed: LogFeed) -> Harness<'static, DbDeskApp> {
        let config = engine_config(dir);
        harness_at_default_size().build_eframe(|cc| DbDeskApp::new(cc, log_feed, config).unwrap())
    }

    /// Steps the app until `done` holds; engine events arrive asynchronously, so a fixed
    /// number of steps would be flaky.
    fn step_until(
        harness: &mut Harness<'_, DbDeskApp>,
        what: &str,
        done: impl Fn(&Harness<'_, DbDeskApp>) -> bool,
    ) {
        let deadline = std::time::Instant::now() + WAIT_TIMEOUT;
        while !done(harness) {
            assert!(
                std::time::Instant::now() < deadline,
                "{what} never happened"
            );
            std::thread::sleep(Duration::from_millis(10));
            harness.step();
        }
    }

    fn step_until_visible(harness: &mut Harness<'_, DbDeskApp>, text: &str) {
        step_until(harness, &format!("{text:?} appearing"), |harness| {
            harness.query_by_label_contains(text).is_some()
        });
    }

    /// Saves one SQLite data source named `shop`, over a database with `schema`.
    fn save_shop(dir: &Path, schema: &str) -> dbdesk_core::DataSource {
        let database = dir.join("shop.db");
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute_batch(schema)
            .unwrap();
        let data_source = dbdesk_core::DataSource {
            id: DataSourceId::generate(),
            name: "shop".to_owned(),
            color: None,
            config: dbdesk_core::ConnectionConfig::Sqlite(dbdesk_core::SqliteConfig {
                path: database,
                read_only: false,
            }),
        };
        DataSourceFile::new(dir.join("data_sources.toml"))
            .save(std::slice::from_ref(&data_source))
            .unwrap();
        data_source
    }

    #[test]
    fn shows_every_panel_the_engine_status_and_log_entries() {
        let dir = tempfile::tempdir().unwrap();
        let (layer, feed) = log_console::pipeline();
        // Thread-local, so it only captures events logged on this (the UI) thread.
        let _subscriber =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
        tracing::info!("logged before the window opened");

        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "Engine running");
        step_until_visible(&mut harness, "No data sources yet.");

        // egui_dock doesn't expose tab titles to accessibility, so check each panel's content.
        harness.get_by_label_contains("Welcome to dbdesk");
        harness.get_by_label_contains("logged before the window opened");
    }

    #[test]
    fn adds_a_sqlite_data_source_through_the_dialog() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("chinook.db");
        rusqlite::Connection::open(&database).unwrap();
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "No data sources yet.");

        // The explorer's toolbar and the Welcome tab both offer it; either opens the menu.
        harness
            .get_all_by_label("New data source")
            .next()
            .unwrap()
            .click();
        harness.run();
        harness.get_by_label("SQLite").click();
        harness.run();
        // The native open and save dialogs can't be driven headlessly; check they're offered.
        harness.get_by_label("Browse…");
        harness.get_by_label("New…");
        let file_field = harness.get_by_role_and_label(Role::TextInput, "File");
        file_field.focus();
        harness.run();
        harness
            .get_by_role_and_label(Role::TextInput, "File")
            .type_text(&database.display().to_string());
        harness.run();
        harness.get_by_label("Save").click();

        step_until_visible(&mut harness, "Expand chinook.db");
        let saved = DataSourceFile::new(dir.path().join("data_sources.toml"))
            .load()
            .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name, "chinook.db");
    }

    #[test]
    fn adds_a_mysql_data_source_with_the_servers_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "No data sources yet.");

        harness
            .get_all_by_label("New data source")
            .next()
            .unwrap()
            .click();
        harness.run();
        harness.get_by_label("MySQL / MariaDB").click();
        harness.run();
        harness.get_by_label_contains("New MySQL / MariaDB data source");
        harness.get_by_label("Save").click();

        step_until_visible(&mut harness, "Expand mysql@localhost");
        let saved = DataSourceFile::new(dir.path().join("data_sources.toml"))
            .load()
            .unwrap();
        let [saved] = saved.as_slice() else {
            panic!("expected one data source, got {saved:?}");
        };
        let dbdesk_core::ConnectionConfig::Mysql(config) = &saved.config else {
            panic!("expected a MySQL data source, got {saved:?}");
        };
        let server = &config.server;
        assert_eq!(
            (
                server.host.as_str(),
                server.port,
                server.database.as_str(),
                server.user.as_str()
            ),
            ("localhost", 3306, "mysql", "root")
        );
    }

    #[test]
    fn adds_a_sql_server_data_source_with_the_servers_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "No data sources yet.");

        harness
            .get_all_by_label("New data source")
            .next()
            .unwrap()
            .click();
        harness.run();
        harness.get_by_label("SQL Server").click();
        harness.run();
        harness.get_by_label_contains("New SQL Server data source");
        harness.get_by_label("Save").click();

        step_until_visible(&mut harness, "Expand master@localhost");
        let saved = DataSourceFile::new(dir.path().join("data_sources.toml"))
            .load()
            .unwrap();
        let [saved] = saved.as_slice() else {
            panic!("expected one data source, got {saved:?}");
        };
        let dbdesk_core::ConnectionConfig::SqlServer(config) = &saved.config else {
            panic!("expected a SQL Server data source, got {saved:?}");
        };
        let server = &config.server;
        assert_eq!(
            (
                server.host.as_str(),
                server.port,
                server.database.as_str(),
                server.user.as_str()
            ),
            ("localhost", 1433, "master", "sa")
        );
    }

    #[test]
    fn chooses_a_tls_mode_and_ca_file_for_a_postgres_data_source() {
        let dir = tempfile::tempdir().unwrap();
        let ca_file = dir.path().join("internal-ca.pem");
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "No data sources yet.");

        harness
            .get_all_by_label("New data source")
            .next()
            .unwrap()
            .click();
        harness.run();
        harness.get_by_label("PostgreSQL").click();
        harness.run();
        // Only verify-full reads a CA file, so the row only shows for it.
        assert!(harness.query_by_label("CA file").is_none());
        harness
            .get_by(|node| {
                node.role() == Role::ComboBox && node.value().as_deref() == Some("prefer")
            })
            .click();
        harness.run();
        harness.get_by_label("verify-full").click();
        harness.run();
        harness.get_by_label("Browse…");
        harness
            .get_by_role_and_label(Role::TextInput, "CA file")
            .focus();
        harness.run();
        harness
            .get_by_role_and_label(Role::TextInput, "CA file")
            .type_text(&ca_file.display().to_string());
        harness.run();
        harness.get_by_label("Save").click();

        step_until_visible(&mut harness, "Expand postgres@localhost");
        let saved = DataSourceFile::new(dir.path().join("data_sources.toml"))
            .load()
            .unwrap();
        let [saved] = saved.as_slice() else {
            panic!("expected one data source, got {saved:?}");
        };
        let dbdesk_core::ConnectionConfig::Postgres(config) = &saved.config else {
            panic!("expected a PostgreSQL data source, got {saved:?}");
        };
        assert_eq!(
            config.server.tls,
            dbdesk_core::TlsMode::VerifyFull {
                ca_file: Some(ca_file)
            }
        );
    }

    #[test]
    fn browses_a_sqlite_catalog_down_to_columns() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(
            dir.path(),
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT NOT NULL)",
        );
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);

        for (expand, appears) in [
            ("Expand shop", "Expand main"),
            ("Expand main", "Expand tables"),
            ("Expand tables", "Expand orders"),
            ("Expand orders", "Expand columns"),
            ("Expand columns", "customer"),
        ] {
            step_until_visible(&mut harness, expand);
            harness.get_by_label(expand).click();
            step_until_visible(&mut harness, appears);
        }
        harness.get_by_label("text, not null");
        step_until_visible(&mut harness, "1 connected");
    }

    #[test]
    fn opens_table_data_in_a_console_and_loads_more_rows() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(
            dir.path(),
            "CREATE TABLE orders (id INTEGER PRIMARY KEY);
             WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 600)
             INSERT INTO orders SELECT n FROM s;",
        );
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);

        open_orders(&mut harness, "500 rows, more available");
        harness.get_by_label("Load more rows").click();
        step_until_visible(&mut harness, "600 rows ·");
    }

    #[test]
    fn restores_console_tabs_from_their_files_and_saves_edits() {
        let dir = tempfile::tempdir().unwrap();
        let shop = save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
        let files = ConsoleFiles::new(dir.path().join("consoles"));
        let saved = ConsoleKey {
            data_source: shop.id,
            name: ConsoleName::numbered(1),
        };
        files.save(&saved, "select 41;").unwrap();
        let deleted = ConsoleKey {
            data_source: shop.id,
            name: ConsoleName::numbered(2),
        };
        let (_layer, feed) = log_console::pipeline();
        let config = engine_config(dir.path());
        // As if the saved layout held both tabs, though one file has since been deleted.
        let mut harness = harness_at_default_size().build_eframe(|cc| {
            let mut app = DbDeskApp::new(cc, feed, config).unwrap();
            workspace::open_console_tab(&mut app.dock, saved.clone());
            workspace::open_console_tab(&mut app.dock, deleted.clone());
            app
        });

        let editor = || {
            |node: &egui_kittest::kittest::AccessKitNode<'_>| {
                node.role() == Role::MultilineTextInput
                    && node
                        .value()
                        .is_some_and(|text| text.starts_with("select 41;"))
            }
        };
        step_until(&mut harness, "the console reopening", |harness| {
            harness.query_by(editor()).is_some()
        });
        assert_eq!(
            workspace::console_tabs(&harness.state().dock),
            std::slice::from_ref(&saved)
        );

        harness.get_by(editor()).focus();
        harness.run();
        harness.get_by(editor()).type_text(" select 42;");
        step_until(&mut harness, "the edit being saved", |_| {
            std::fs::read_to_string(files.path(&saved))
                .unwrap()
                .contains("select 42;")
        });
    }

    #[test]
    fn inserts_a_statement_from_the_history_into_the_console() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        open_orders(&mut harness, "0 rows");

        harness.get_by_label("History").click();
        step_until(&mut harness, "the history loading", |harness| {
            harness.query_by_label("Insert").is_some()
        });
        harness.get_by_label("Insert").click();
        harness.run();

        let query = "SELECT * FROM main.orders;\n";
        harness.get_by_value(&query.repeat(2));
    }

    fn count_orders(dir: &Path) -> i64 {
        rusqlite::Connection::open(dir.join("shop.db"))
            .unwrap()
            .query_row("SELECT count(*) FROM orders", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn manual_commit_mode_holds_changes_until_rolled_back_and_warns_before_closing() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "Expand shop");
        harness.get_by_label("shop").click_secondary();
        harness.run();
        // The Welcome tab has a card of the same name; the context menu comes first.
        harness
            .get_all_by_label("New query console")
            .next()
            .unwrap()
            .click();
        harness.run();
        harness
            .get_by(|node| {
                node.role() == Role::ComboBox && node.value().as_deref() == Some("Auto-commit")
            })
            .click();
        harness.run();
        harness.get_by_label("Manual commit").click();
        harness.run();
        let editor = harness.get_by_role(Role::MultilineTextInput);
        editor.focus();
        harness.run();
        harness
            .get_by_role(Role::MultilineTextInput)
            .type_text("INSERT INTO orders VALUES (1)");
        harness.run();

        let run_and_wait_for_transaction = |harness: &mut Harness<'_, DbDeskApp>| {
            harness.get_by_label("Run").click();
            step_until_visible(harness, "Transaction open");
        };
        run_and_wait_for_transaction(&mut harness);
        harness.get_by_label("Rollback").click();
        step_until_visible(&mut harness, "Rolled back");
        assert_eq!(count_orders(dir.path()), 0);

        run_and_wait_for_transaction(&mut harness);
        let key = workspace::console_tabs(&harness.state().dock)[0].clone();
        let ctx = harness.ctx.clone();
        harness
            .state_mut()
            .close_connections(Closing::Console(key), &ctx);
        step_until_visible(&mut harness, "Roll back open transactions?");
        // The modal is measured on its first frame and placed on the next; a click aimed at
        // the first frame's button lands outside it and dismisses the modal. Exactly two
        // frames, not `run()`: a spinner still turning elsewhere (the slower Windows runner
        // got there first) keeps the UI repainting, and `run()` would give up waiting for it
        // to settle.
        harness.run_steps(2);
        assert_eq!(workspace::console_tabs(&harness.state().dock).len(), 1);
        harness.get_by_label("Roll back and close").click();

        step_until(&mut harness, "the console tab closing", |harness| {
            workspace::console_tabs(&harness.state().dock).is_empty()
        });
        step_until(&mut harness, "the connection closing", |_| {
            count_orders(dir.path()) == 0
        });
    }

    /// Opens the `orders` table of a saved `shop` data source in a console, which runs
    /// `SELECT * FROM main.orders`, and waits for its rows.
    fn open_orders(harness: &mut Harness<'_, DbDeskApp>, rows_shown: &str) {
        for expand in ["Expand shop", "Expand main", "Expand tables"] {
            step_until_visible(harness, expand);
            harness.get_by_label(expand).click();
        }
        step_until_visible(harness, "Expand orders");
        harness.get_by_label("orders").click_secondary();
        harness.run();
        harness.get_by_label("Open data").click();
        step_until_visible(harness, rows_shown);
    }

    #[test]
    fn explains_the_statement_at_the_cursor() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        open_orders(&mut harness, "0 rows");

        harness.get_by_label("Explain").click();

        // SQLite's query plan for a full table scan, which is no table's rows to edit.
        step_until_visible(&mut harness, "SCAN main.orders");
        harness.get_by_label("Read-only");
    }

    #[test]
    fn sorts_views_and_exports_the_loaded_rows() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(
            dir.path(),
            r#"CREATE TABLE orders (id INTEGER PRIMARY KEY, note TEXT);
               INSERT INTO orders VALUES (1, 'plain'), (2, '{"gift":true}'), (3, NULL);"#,
        );
        let export_path = dir.path().join("orders.csv");
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        let picked = export_path.clone();
        harness.state_mut().pick_export_path = Box::new(move |_| Some(picked.clone()));
        open_orders(&mut harness, "3 rows");

        harness.get_by_label("id").click();
        harness.run();
        harness.get_by_label("id ▲").click();
        harness.run();
        harness.get_by_label("id ▼");
        harness.get_by_label_contains("Sorted and filtered here, not by the database: 3 of 3");

        harness.get_by_label(r#"{"gift":true}"#).click();
        harness.run();
        harness.key_press(egui::Key::Space);
        harness.run();
        harness.get_by_label("Value of note");
        harness.get_by_value("{\n  \"gift\": true\n}");

        harness.get_by_label("Export").click();
        harness.run();
        harness.get_by_label("CSV…").click();
        step_until_visible(&mut harness, "Exported 3 rows");
        assert_eq!(
            std::fs::read_to_string(&export_path).unwrap(),
            "id,note\r\n3,\r\n2,\"{\"\"gift\"\":true}\"\r\n1,plain\r\n"
        );
    }

    #[test]
    fn runs_with_the_shortcut_keymap_toml_binds_and_reports_its_problems() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
        std::fs::write(
            dir.path().join("keymap.toml"),
            "[bindings]\n\"console.run\" = \"f5\"\n\"console.walk\" = \"f6\"\n",
        )
        .unwrap();
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "keymap.toml: 1 problem");
        step_until_visible(&mut harness, "Expand shop");
        harness.get_by_label("shop").click_secondary();
        harness.run();
        // The Welcome tab has a card of the same name; the context menu comes first.
        harness
            .get_all_by_label("New query console")
            .next()
            .unwrap()
            .click();
        harness.run();
        harness.get_by_role(Role::MultilineTextInput).focus();
        harness.run();
        harness
            .get_by_role(Role::MultilineTextInput)
            .type_text("SELECT 42 AS answer");
        harness.step();

        harness.key_press(egui::Key::F5);

        step_until_visible(&mut harness, "1 rows");
        harness.get_by_label("keymap.toml: 1 problem").click();
        harness.run();
        harness.get_by_label_contains("\"console.walk\" is not an action");
    }

    #[test]
    fn completes_a_column_from_the_catalog_it_loads_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(
            dir.path(),
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT)",
        );
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "Expand shop");
        harness.get_by_label("shop").click_secondary();
        harness.run();
        // The Welcome tab has a card of the same name; the context menu comes first.
        harness
            .get_all_by_label("New query console")
            .next()
            .unwrap()
            .click();
        harness.run();
        harness.get_by_role(Role::MultilineTextInput).focus();
        harness.run();

        harness
            .get_by_role(Role::MultilineTextInput)
            .type_text("SELECT * FROM orders WHERE cus");
        step_until(&mut harness, "the column to be suggested", |harness| {
            harness.query_by_label("customer").is_some()
        });
        harness.key_press(egui::Key::Enter);
        harness.run();

        harness.get_by(|node| {
            node.role() == Role::MultilineTextInput
                && node.value().as_deref() == Some("SELECT * FROM orders WHERE customer")
        });
        assert!(
            !harness.state().state.data_sources.entries()[0]
                .catalog
                .is_expanded(&dbdesk_core::CatalogPath::root()),
            "completion loads the catalog without expanding the explorer"
        );
    }

    /// Opens a console on the shop's data source and focuses its editor.
    fn open_console(harness: &mut Harness<'_, DbDeskApp>) {
        step_until_visible(harness, "Expand shop");
        harness.get_by_label("shop").click_secondary();
        harness.run();
        // The Welcome tab has a card of the same name; the context menu comes first.
        harness
            .get_all_by_label("New query console")
            .next()
            .unwrap()
            .click();
        harness.run();
        harness.get_by_role(Role::MultilineTextInput).focus();
        harness.run();
    }

    fn editor_text(harness: &Harness<'_, DbDeskApp>) -> String {
        harness
            .get_by_role(Role::MultilineTextInput)
            .value()
            .unwrap_or_default()
    }

    #[test]
    fn formats_a_statement_undoes_it_in_one_step_and_follows_the_settings() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        open_console(&mut harness);
        let typed = "select id,customer from orders where id=1";
        harness
            .get_by_role(Role::MultilineTextInput)
            .type_text(typed);
        harness.run();

        harness.key_press_modifiers(
            egui::Modifiers::COMMAND | egui::Modifiers::ALT,
            egui::Key::L,
        );
        harness.run();
        assert_eq!(
            editor_text(&harness),
            "SELECT\n    id,\n    customer\nFROM orders\nWHERE id = 1"
        );
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.run();
        assert_eq!(
            editor_text(&harness),
            typed,
            "one undo restores the typed text"
        );

        harness.get_by_label("View").click();
        harness.run();
        harness.get_by_label("Settings…").click();
        harness.run();
        harness.get_by_label("lower").click();
        harness.run();
        harness.get_by_label("Code").click();
        harness.run();
        harness.get_by_label_contains("Format all").click();
        harness.run();
        assert_eq!(
            editor_text(&harness),
            "select\n    id,\n    customer\nfrom orders\nwhere id = 1"
        );
        step_until(&mut harness, "the settings file", |_| {
            std::fs::read_to_string(dir.path().join("settings.toml"))
                .is_ok_and(|text| text.contains("keyword_case = \"lower\""))
        });
    }

    #[test]
    fn underlines_problems_while_typing_and_explains_them_on_hover() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        open_console(&mut harness);

        harness
            .get_by_role(Role::MultilineTextInput)
            .type_text("select * from orders where id = null;\nselect id from orders where");
        // Checked once typing pauses.
        step_until_visible(&mut harness, "1 error, 1 warning");
        let squiggles: Vec<egui::Rect> = harness
            .state()
            .consoles
            .open_consoles()
            .flat_map(|console| console.squiggle_rects().collect::<Vec<_>>())
            .collect();
        assert_eq!(squiggles.len(), 2, "one squiggle per problem");

        harness.hover_at(squiggles[0].center());
        step_until_visible(&mut harness, "= NULL is never true; use IS NULL");
        harness.get_by_label_contains("null-comparison");
        harness.hover_at(squiggles[1].center());
        step_until_visible(&mut harness, "Syntax error: expected an expression");
    }

    #[test]
    fn completes_tables_after_from_and_an_aliases_columns_after_its_dot() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(
            dir.path(),
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT);
             CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT);",
        );
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        open_console(&mut harness);
        let editor_is = |harness: &Harness<'_, DbDeskApp>, text: &str| {
            harness
                .query_by(|node| {
                    node.role() == Role::MultilineTextInput && node.value().as_deref() == Some(text)
                })
                .is_some()
        };

        harness
            .get_by_role(Role::MultilineTextInput)
            .type_text("SELECT * FROM cus");
        step_until(&mut harness, "the table to be suggested", |harness| {
            harness.query_by_label("customers").is_some()
        });
        assert!(
            harness.query_by_label("customer").is_none(),
            "after FROM, tables are offered, not columns"
        );
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert!(editor_is(&harness, "SELECT * FROM customers"));

        harness
            .get_by_role(Role::MultilineTextInput)
            .type_text(" c WHERE c.na");
        step_until(
            &mut harness,
            "the alias's column to be suggested",
            |harness| harness.query_by_label("name").is_some(),
        );
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert!(editor_is(
            &harness,
            "SELECT * FROM customers c WHERE c.name"
        ));
    }

    #[test]
    fn tells_when_a_newer_release_is_out_and_when_it_is_up_to_date() {
        let dir = tempfile::tempdir().unwrap();
        let (_layer, feed) = log_console::pipeline();
        let config = engine_config_with_releases(dir.path(), FixedReleases::tagged("v99.0.0"));
        let mut harness =
            harness_at_default_size().build_eframe(|cc| DbDeskApp::new(cc, feed, config).unwrap());

        // Checked at start-up: the status bar says so, and its link opens the dialog.
        step_until_visible(&mut harness, "dbdesk 99.0.0 is available");
        harness.get_by_label("dbdesk 99.0.0 is available").click();
        step_until_visible(&mut harness, "dbdesk 99.0.0 is available (you have");
        harness.get_by_label("Open release page");
        drop(harness);

        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "Engine running");
        harness.get_by_label("Help").click();
        harness.run_steps(2);
        harness.get_by_label("Check for updates…").click();
        step_until_visible(&mut harness, "You're up to date");
        assert!(harness.query_by_label_contains("is available").is_none());
    }

    #[test]
    fn edits_a_cell_and_submits_it_as_an_update() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(
            dir.path(),
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT);
             INSERT INTO orders VALUES (1, 'ada'), (2, 'bob');",
        );
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        open_orders(&mut harness, "2 rows");
        let cell_editor = |harness: &Harness<'_, DbDeskApp>| {
            harness
                .query_by(|node| {
                    node.role() == Role::TextInput && node.value().as_deref() == Some("ada")
                })
                .is_some()
        };

        harness.get_by_label("ada").click();
        harness.run();
        // F2 edits once the table has been described; until then it does nothing.
        let deadline = std::time::Instant::now() + WAIT_TIMEOUT;
        while !cell_editor(&harness) {
            assert!(
                std::time::Instant::now() < deadline,
                "the cell editor never opened"
            );
            harness.key_press(egui::Key::F2);
            harness.step();
            std::thread::sleep(Duration::from_millis(10));
        }
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness
            .get_by(|node| node.role() == Role::TextInput && node.value().as_deref() == Some("ada"))
            .type_text("Ada Lovelace");
        harness.key_press(egui::Key::Enter);
        harness.run();
        harness.get_by_label("1 changed cell");
        assert!(
            harness
                .query_by(|node| {
                    node.role() == Role::TextInput
                        && node.value().as_deref() == Some("Ada Lovelace")
                })
                .is_none(),
            "Enter closes the editor and keeps it closed"
        );
        harness.get_by_label("Submit").click();

        // Submit swaps "1 changed cell" for "Saving…" at once; only when that goes too has the
        // engine written the UPDATE. Waiting on the first label alone raced the database read
        // below, and lost on the slower Linux and Windows CI runners.
        step_until(&mut harness, "the edit being saved", |harness| {
            harness.query_by_label("1 changed cell").is_none()
                && harness.query_by_label("Saving…").is_none()
        });
        harness.get_by_label("Ada Lovelace");
        let saved: String = rusqlite::Connection::open(dir.path().join("shop.db"))
            .unwrap()
            .query_row("SELECT customer FROM orders WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(saved, "Ada Lovelace");
    }
}
