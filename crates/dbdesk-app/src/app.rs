use std::path::PathBuf;
use std::time::Instant;

use dbdesk_core::{CatalogPath, DataSource, DataSourceId};
use dbdesk_engine::{
    Command, EngineConfig, EngineError, EngineHandle, Event, PasswordKept, Repaint,
};
use dbdesk_store::{ConsoleKey, ExportFormat, McpPermissions};
use egui::{Frame, Margin, RichText, ThemePreference};
use egui_dock::{DockArea, DockState};

use crate::about_dialog;
use crate::console::{ConsoleAction, FormatScope};
use crate::consoles::Consoles;
use crate::data_source_dialog::{DataSourceDialog, DialogAction};
use crate::data_source_form::McpPasswordChange;
use crate::definition_window::DefinitionWindow;
use crate::delete_dialog::{DeleteChoice, DeleteConfirmation};
use crate::explorer::{ExplorerAction, ExplorerOptions};
use crate::export_dialog;
use crate::export_sources_dialog::{ExportAction, ExportSourcesDialog};
use crate::file_picker;
use crate::history_window::{HistoryAction, HistoryWindow};
use crate::import_dialog::{ImportAction, ImportDialog};
use crate::keymap::{self, Keymap};
use crate::keymap_window::{self, KeymapWindowAction};
use crate::log_console::{LogConsole, LogFeed};
use crate::mcp_settings::McpDefaultsEditor;
use crate::password_prompt::{PasswordPrompt, PromptAction};
use crate::project_dialog::{
    DeleteProjectChoice, DeleteProjectConfirmation, ProjectDialogAction, ProjectNameDialog,
};
use crate::project_settings::{ProjectSettingsAction, ProjectSettingsDialog};
use crate::rollback_dialog::{Closing, RollbackChoice, RollbackConfirmation};
use crate::settings::AppSettings;
use crate::settings_window::{self, SettingsWindowAction};
use crate::state::{AppState, EngineStatus};
use crate::theme::{self, Palette};
use crate::update_dialog::{self, UpdateAction};
use crate::updates::Updates;
use crate::workspace::{self, Panels, Tab};
use crate::{fonts, icons, relaunch, widgets};

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
    project_dialog: Option<ProjectNameDialog>,
    project_settings: Option<ProjectSettingsDialog>,
    /// View › Settings › MCP's draft, kept while the window is closed.
    mcp_defaults: McpDefaultsEditor,
    delete_project_confirmation: Option<DeleteProjectConfirmation>,
    definition_window: Option<DefinitionWindow>,
    history_window: Option<HistoryWindow>,
    rollback_confirmation: Option<RollbackConfirmation>,
    password_prompt: Option<PasswordPrompt>,
    export_sources_dialog: Option<ExportSourcesDialog>,
    import_dialog: Option<ImportDialog>,
    /// Ask where to export data sources to and which file to import. The platform's file
    /// dialogs, except in tests.
    pick_data_sources_export: Box<dyn Fn() -> Option<PathBuf>>,
    pick_data_sources_import: Box<dyn Fn() -> Option<PathBuf>>,
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
        // Asked before the engine takes the config: it only looks at how this copy was
        // installed, so the update dialog knows at once whether to offer installing.
        let install_support = engine_config.updater.support();
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
            project_dialog: None,
            project_settings: None,
            mcp_defaults: McpDefaultsEditor::new(&McpPermissions::default()),
            delete_project_confirmation: None,
            definition_window: None,
            history_window: None,
            rollback_confirmation: None,
            password_prompt: None,
            export_sources_dialog: None,
            import_dialog: None,
            pick_data_sources_export: Box::new(file_picker::pick_data_sources_export),
            pick_data_sources_import: Box::new(file_picker::pick_data_sources_import),
            quit_confirmed: false,
            pick_export_path: Box::new(export_dialog::pick_export_path),
            keymap: Keymap::default(),
            keymap_path: None,
            keymap_window_open: false,
            about_open: false,
            settings: AppSettings::default(),
            updates: Updates::new(install_support),
            consoles: Consoles::default(),
            maximize_window,
        })
    }

    /// Makes the export and import file dialogs answer `path`, for tests and screenshots.
    #[cfg(test)]
    pub fn answer_file_dialogs_with(&mut self, path: &std::path::Path) {
        let export = path.to_owned();
        let import = path.to_owned();
        self.pick_data_sources_export = Box::new(move || Some(export.clone()));
        self.pick_data_sources_import = Box::new(move || Some(import.clone()));
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
        match event {
            Event::ProjectsLoaded(projects) => self.mcp_defaults.follow(&projects.mcp_defaults),
            Event::McpDefaultsSaved(defaults) => self.mcp_defaults.follow(defaults),
            Event::McpDefaultsSaveFailed { error } => self.mcp_defaults.failed(error),
            _ => {}
        }
        if self
            .data_source_dialog
            .as_mut()
            .is_some_and(|dialog| dialog.apply(event))
        {
            self.data_source_dialog = None;
        }
        if self
            .project_dialog
            .as_mut()
            .is_some_and(|dialog| dialog.apply(event))
        {
            self.project_dialog = None;
        }
        if self
            .project_settings
            .as_mut()
            .is_some_and(|dialog| dialog.apply(event))
        {
            self.project_settings = None;
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
        self.apply_to_password_prompt(event);
        if let Some(dialog) = &mut self.export_sources_dialog {
            dialog.apply(event);
        }
        if let Some(dialog) = &mut self.import_dialog {
            dialog.apply(event);
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

    /// Opens the prompt when a connection needs a password, unless one is open already (the
    /// explorer and a console may both have asked), and connects again once it is given.
    fn apply_to_password_prompt(&mut self, event: &Event) {
        if let Event::PasswordNeeded {
            data_source,
            reason,
        } = event
            && self.password_prompt.is_none()
            && let Some(entry) = self.state.data_source(*data_source)
        {
            self.password_prompt = Some(PasswordPrompt::new(&entry.data_source, reason.clone()));
        }
        let Some(prompt) = &self.password_prompt else {
            return;
        };
        let Some(kept) = prompt.apply(event) else {
            return;
        };
        let data_source = prompt.data_source();
        self.password_prompt = None;
        if let PasswordKept::SaveFailed(error) = kept {
            tracing::warn!(
                "the password was not saved to the keychain, so it is kept until dbdesk quits: \
                 {error}"
            );
        }
        self.send(Command::Connect(data_source));
    }

    fn show_password_prompt(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &mut self.password_prompt else {
            return;
        };
        let data_source = prompt.data_source();
        let Some(action) = prompt.show(ctx) else {
            return;
        };
        match action {
            PromptAction::Provide { password, save } => self.send(Command::ProvidePassword {
                data_source,
                password,
                save,
            }),
            PromptAction::Cancel => self.password_prompt = None,
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
            ExplorerAction::NewProject { then_move } => {
                self.project_dialog = Some(ProjectNameDialog::create(then_move));
            }
            ExplorerAction::RenameProject(id) => {
                if let Some(project) = self.state.projects.get(id) {
                    self.project_dialog = Some(ProjectNameDialog::rename(project));
                }
            }
            ExplorerAction::ProjectSettings(id) => {
                if let Some(project) = self.state.projects.get(id) {
                    self.project_settings = Some(ProjectSettingsDialog::new(
                        project,
                        &self.state.mcp_defaults,
                    ));
                }
            }
            ExplorerAction::DeleteProject(id) => {
                if let Some(project) = self.state.projects.get(id) {
                    let members = self
                        .state
                        .data_sources
                        .entries()
                        .iter()
                        .filter(|entry| entry.data_source.project == Some(id))
                        .count();
                    self.delete_project_confirmation =
                        Some(DeleteProjectConfirmation::new(project, members));
                }
            }
            ExplorerAction::MoveToProject(data_source, project) => {
                self.send(Command::MoveToProject {
                    data_source,
                    project,
                });
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
            DialogAction::Save(data_source, password, mcp_password) => {
                let id = data_source.id;
                self.send(Command::SaveDataSource {
                    data_source,
                    password,
                });
                let password = match mcp_password {
                    McpPasswordChange::Keep => return,
                    McpPasswordChange::Set(password) => Some(password),
                    McpPasswordChange::Remove => None,
                };
                self.send(Command::SaveMcpPassword { id, password });
            }
            DialogAction::CreateDatabaseFile(file) => {
                self.send(Command::CreateDatabaseFile(file));
            }
            DialogAction::Close => self.data_source_dialog = None,
        }
    }

    fn show_project_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self
            .project_dialog
            .as_mut()
            .and_then(|dialog| dialog.show(ctx))
        else {
            return;
        };
        match action {
            ProjectDialogAction::Create {
                id,
                name,
                then_move,
            } => {
                self.send(Command::CreateProject { id, name });
                // Commands are handled in order, so the project exists by the time this runs.
                if let Some(data_source) = then_move {
                    self.send(Command::MoveToProject {
                        data_source,
                        project: Some(id),
                    });
                }
            }
            ProjectDialogAction::Rename { id, name } => {
                self.send(Command::RenameProject { id, name });
            }
            ProjectDialogAction::Close => self.project_dialog = None,
        }
    }

    fn show_project_settings(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.project_settings else {
            return;
        };
        let members: Vec<&DataSource> = self
            .state
            .data_sources
            .entries()
            .iter()
            .map(|entry| &entry.data_source)
            .filter(|data_source| data_source.project == Some(dialog.project()))
            .collect();
        let Some(action) = dialog.show(ctx, &self.state.mcp_defaults, &members) else {
            return;
        };
        match action {
            ProjectSettingsAction::Save {
                id,
                mcp_enabled,
                overrides,
            } => self.send(Command::SaveProjectMcp {
                id,
                mcp_enabled,
                overrides,
            }),
            ProjectSettingsAction::Close => self.project_settings = None,
        }
    }

    fn show_delete_project_confirmation(&mut self, ctx: &egui::Context) {
        let Some(choice) = self
            .delete_project_confirmation
            .as_ref()
            .and_then(|confirmation| confirmation.show(ctx))
        else {
            return;
        };
        self.delete_project_confirmation = None;
        if let DeleteProjectChoice::Confirmed(id) = choice {
            self.send(Command::DeleteProject(id));
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
        match choice {
            RollbackChoice::Proceed => self.finish_closing(confirmation.closing, ctx),
            RollbackChoice::Cancel if matches!(confirmation.closing, Closing::Quit) => {
                self.updates.cancel_restart();
            }
            RollbackChoice::Cancel => {}
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
            HistoryAction::Search(query) => self.send(Command::SearchHistory(query)),
            HistoryAction::Insert { console, sql } => {
                self.consoles.insert_statement(&console, &sql);
            }
            HistoryAction::Close => self.history_window = None,
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| self.file_menu(ui));
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

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        let has_data_sources = !self.state.data_sources.entries().is_empty();
        if ui
            .add_enabled(has_data_sources, egui::Button::new("Export data sources…"))
            .on_hover_text("Write data sources and projects to a file, without passwords")
            .on_disabled_hover_text("There are no data sources to export yet")
            .clicked()
        {
            self.export_sources_dialog = Some(ExportSourcesDialog::new());
        }
        if ui
            .button("Import data sources…")
            .on_hover_text("Add or update data sources from an exported file")
            .clicked()
            && let Some(path) = (self.pick_data_sources_import)()
        {
            let (dialog, read) = ImportDialog::open(path);
            self.import_dialog = Some(dialog);
            self.send(read);
        }
    }

    fn show_export_sources_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.export_sources_dialog else {
            return;
        };
        let data_sources: Vec<_> = self
            .state
            .data_sources
            .entries()
            .iter()
            .map(|entry| entry.data_source.clone())
            .collect();
        match dialog.show(ctx, self.state.projects.projects(), &data_sources) {
            Some(ExportAction::ChooseFile) => {
                if let Some(path) = (self.pick_data_sources_export)() {
                    let command = dialog.command(path);
                    self.send(command);
                }
            }
            Some(ExportAction::Close) => self.export_sources_dialog = None,
            None => {}
        }
    }

    fn show_import_dialog(&mut self, ctx: &egui::Context) {
        match self
            .import_dialog
            .as_mut()
            .and_then(|dialog| dialog.show(ctx))
        {
            Some(ImportAction::Apply(plan)) => self.send(Command::ApplyImport(plan)),
            Some(ImportAction::Close) => self.import_dialog = None,
            None => {}
        }
    }

    fn show_about(&mut self, ctx: &egui::Context) {
        if self.about_open && about_dialog::show(ctx) {
            self.about_open = false;
        }
    }

    fn show_update_dialog(&mut self, ctx: &egui::Context) {
        if !self.updates.dialog_open {
            return;
        }
        match update_dialog::show(ctx, &self.updates) {
            Some(UpdateAction::Close) => self.updates.dialog_open = false,
            Some(UpdateAction::Install) => {
                let install = self.updates.install();
                self.send_if(install);
            }
            // Quits the usual way, so open transactions are still confirmed first; `main`
            // starts the installed copy once this one has stopped.
            Some(UpdateAction::Restart) if self.updates.request_restart() => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(UpdateAction::Restart) | None => {}
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
        match settings_window::show(ctx, &self.settings, &mut self.mcp_defaults) {
            Some(SettingsWindowAction::SaveMcpDefaults(defaults)) => {
                self.send(Command::SaveMcpDefaults(defaults));
            }
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
            if let Some(notice) = self.updates.notice()
                && ui
                    .link(RichText::new(notice).small().color(palette.primary))
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
                    projects: self.state.projects.projects(),
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
        self.show_project_dialog(&ctx);
        self.show_project_settings(&ctx);
        self.show_delete_project_confirmation(&ctx);
        self.show_password_prompt(&ctx);
        self.show_export_sources_dialog(&ctx);
        self.show_import_dialog(&ctx);
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
        if self.updates.restart_after_quit() {
            relaunch::request();
        }
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
        ConsoleFiles, ConsoleName, DataSourceFile, HistoryFile, KeymapFile, ProjectFile,
        SecretError, SecretStore, SettingsFile,
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

        fn mcp_password(&self, _: DataSourceId) -> Result<Option<Password>, SecretError> {
            Ok(None)
        }

        fn set_mcp_password(&self, _: DataSourceId, _: &Password) -> Result<(), SecretError> {
            Ok(())
        }

        fn delete_mcp_password(&self, _: DataSourceId) -> Result<(), SecretError> {
            Ok(())
        }
    }

    fn engine_config(dir: &Path) -> EngineConfig {
        engine_config_with_releases(dir, FixedReleases::current())
    }

    fn engine_config_with_releases(dir: &Path, releases: FixedReleases) -> EngineConfig {
        EngineConfig {
            data_sources: DataSourceFile::new(dir.join("data_sources.toml")),
            projects: ProjectFile::new(dir.join("projects.toml")),
            secrets: Arc::new(NoSecrets),
            consoles: ConsoleFiles::new(dir.join("consoles")),
            history: HistoryFile::new(dir.join("history.sqlite")),
            keymap: KeymapFile::new(dir.join("keymap.toml")),
            settings: SettingsFile::new(dir.join("settings.toml")),
            releases: Arc::new(releases),
            updater: Arc::new(crate::test_support::FakeInstaller::default()),
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
            safety: dbdesk_core::SafetyLevel::Normal,
            project: None,
            mcp_login: None,
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
                server.port.clone(),
                server.database.as_str(),
                server.user.as_str()
            ),
            ("localhost", 3306.into(), "mysql", "root")
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
                server.port.clone(),
                server.database.as_str(),
                server.user.as_str()
            ),
            ("localhost", 1433.into(), "master", "sa")
        );
    }

    #[test]
    fn adds_an_in_memory_duckdb_data_source() {
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
        harness.get_by_label("DuckDB").click();
        harness.run();
        harness.get_by_label_contains("New DuckDB data source");
        harness.get_by_label("In memory").click();
        harness.run();
        harness.get_by_label("Save").click();

        step_until_visible(&mut harness, "Expand memory");
        let saved = DataSourceFile::new(dir.path().join("data_sources.toml"))
            .load()
            .unwrap();
        let [saved] = saved.as_slice() else {
            panic!("expected one data source, got {saved:?}");
        };
        assert_eq!(
            saved.config,
            dbdesk_core::ConnectionConfig::DuckDb(dbdesk_core::DuckDbConfig {
                database: dbdesk_core::DuckDbDatabase::InMemory,
            })
        );
    }

    #[test]
    fn adds_a_clickhouse_data_source_whose_port_follows_its_tls_mode() {
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
        harness.get_by_label("ClickHouse").click();
        harness.run();
        harness.get_by_label_contains("New ClickHouse data source");
        harness.get_by_label("TLS").click();
        harness.run();
        assert!(harness.query_by_label("prefer").is_none());
        harness.get_by_label("require").click();
        harness.run();
        harness.get_by_label("Save").click();

        step_until_visible(&mut harness, "Expand default@localhost");
        let saved = DataSourceFile::new(dir.path().join("data_sources.toml"))
            .load()
            .unwrap();
        let [saved] = saved.as_slice() else {
            panic!("expected one data source, got {saved:?}");
        };
        let dbdesk_core::ConnectionConfig::ClickHouse(config) = &saved.config else {
            panic!("expected a ClickHouse data source, got {saved:?}");
        };
        let server = &config.server;
        assert_eq!(
            (
                server.port.clone(),
                server.database.as_str(),
                server.user.as_str(),
                &server.tls
            ),
            (
                8443.into(),
                "default",
                "default",
                &dbdesk_core::TlsMode::Require
            )
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

    /// Replaces the explorer filter's text, `old`, with `new`. The field is found by its
    /// placeholder while empty, and by its text otherwise.
    fn replace_filter(harness: &mut Harness<'_, DbDeskApp>, old: &str, new: &str) {
        let old = old.to_owned();
        let filter = move || {
            let old = old.clone();
            egui_kittest::kittest::By::new().predicate(move |node| {
                node.role() == Role::TextInput
                    && if old.is_empty() {
                        node.placeholder() == Some("Filter loaded nodes")
                    } else {
                        node.value().as_deref() == Some(old.as_str())
                    }
            })
        };
        harness.get(filter()).focus();
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.key_press(egui::Key::Backspace);
        harness.run();
        if !new.is_empty() {
            harness
                .get(
                    egui_kittest::kittest::By::new()
                        .predicate(|node| node.placeholder() == Some("Filter loaded nodes")),
                )
                .type_text(new);
        }
        harness.run();
    }

    fn saved_project_of(dir: &Path, name: &str) -> Option<dbdesk_core::ProjectId> {
        DataSourceFile::new(dir.join("data_sources.toml"))
            .load()
            .unwrap()
            .into_iter()
            .find(|data_source| data_source.name == name)
            .and_then(|data_source| data_source.project)
    }

    fn saved_project(dir: &Path) -> dbdesk_store::Project {
        let projects = ProjectFile::new(dir.join("projects.toml")).load().unwrap();
        let [project] = projects.projects.as_slice() else {
            panic!("expected one project, got {projects:?}");
        };
        project.clone()
    }

    #[test]
    fn sets_a_projects_mcp_permissions_warning_about_shared_logins() {
        let dir = tempfile::tempdir().unwrap();
        let mut shop = save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
        let project = dbdesk_store::Project::new(
            dbdesk_core::ProjectId::generate(),
            "Analytics".to_owned(),
            dbdesk_store::ProjectSlug::from_name("Analytics"),
        );
        ProjectFile::new(dir.path().join("projects.toml"))
            .save(&dbdesk_store::Projects {
                projects: vec![project.clone()],
                ..dbdesk_store::Projects::default()
            })
            .unwrap();
        shop.project = Some(project.id);
        let warehouse = dbdesk_core::DataSource {
            id: DataSourceId::generate(),
            name: "warehouse".to_owned(),
            config: dbdesk_core::ConnectionConfig::Postgres(dbdesk_core::PostgresConfig {
                server: dbdesk_core::ServerConfig {
                    host: "warehouse.invalid".to_owned(),
                    port: 5432.into(),
                    database: "warehouse".to_owned(),
                    user: "owner".to_owned(),
                    tls: dbdesk_core::TlsMode::Prefer,
                    ssh: None,
                },
            }),
            ..shop.clone()
        };
        DataSourceFile::new(dir.path().join("data_sources.toml"))
            .save(&[shop, warehouse])
            .unwrap();
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "Collapse Analytics");

        harness.get_by_label("Analytics").click_secondary();
        harness.run();
        harness.get_by_label("Project settings…").click();
        harness.run();
        harness.get_by_label_contains("dbdesk mcp --project analytics");
        harness
            .get_by_label("Allow MCP clients to use this project")
            .click();
        harness.run();
        assert!(
            harness.query_by_label_contains("No MCP login").is_none(),
            "reading needs no warning"
        );
        harness.get_by_label("Write").click();
        harness.run();
        harness.get_by_label_contains("No MCP login on warehouse.");
        harness.get_by_label("Save").click();
        step_until(&mut harness, "the project being saved", |harness| {
            harness
                .query_by_label("Allow MCP clients to use this project")
                .is_none()
        });
        let saved = saved_project(dir.path());
        assert!(saved.mcp_enabled);
        assert_eq!(saved.mcp.write, Some(true));
        assert_eq!(
            saved.mcp.schema, None,
            "untouched settings stay at the default"
        );

        harness.get_by_label("Analytics").click_secondary();
        harness.run();
        harness.get_by_label("Project settings…").click();
        harness.run();
        harness.get_by_label("Reset Write").click();
        harness.run();
        harness.get_by_label("Save").click();
        step_until(&mut harness, "the reset being saved", |_| {
            saved_project(dir.path()).mcp.write.is_none()
        });
    }

    #[test]
    fn saves_the_mcp_defaults_from_the_settings_window() {
        let dir = tempfile::tempdir().unwrap();
        save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "Expand shop");

        harness.get_by_label("View").click();
        harness.run();
        harness.get_by_label("Settings…").click();
        harness.run();
        // The window scrolls, and the MCP section is at its bottom, out of reach of clicks.
        harness.get_by_label("Save MCP defaults").scroll_to_me();
        harness.run();
        harness.get_by_label("Schema").click();
        harness.run();
        harness.get_by_label("Only").click();
        harness.run();
        harness
            .get_by_role_and_label(Role::TextInput, "Allowed schemas")
            .focus();
        harness.run();
        harness
            .get_by_role_and_label(Role::TextInput, "Allowed schemas")
            .type_text("public, reports");
        harness.run();
        harness.get_by_label("Save MCP defaults").click();
        step_until(&mut harness, "the defaults being saved", |_| {
            ProjectFile::new(dir.path().join("projects.toml"))
                .load()
                .is_ok_and(|projects| projects.mcp_defaults.schema)
        });
        let defaults = ProjectFile::new(dir.path().join("projects.toml"))
            .load()
            .unwrap()
            .mcp_defaults;
        assert_eq!(
            defaults.schemas,
            dbdesk_store::SchemaScope::Only(vec!["public".to_owned(), "reports".to_owned()])
        );
        assert!(defaults.read && !defaults.write, "only what was changed");
    }

    #[test]
    fn copies_a_projects_mcp_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let mut shop = save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
        let project = dbdesk_store::Project::new(
            dbdesk_core::ProjectId::generate(),
            "Shop EU".to_owned(),
            dbdesk_store::ProjectSlug::from_name("Shop EU"),
        );
        ProjectFile::new(dir.path().join("projects.toml"))
            .save(&dbdesk_store::Projects {
                projects: vec![project.clone()],
                ..dbdesk_store::Projects::default()
            })
            .unwrap();
        shop.project = Some(project.id);
        DataSourceFile::new(dir.path().join("data_sources.toml"))
            .save(&[shop])
            .unwrap();
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "Collapse Shop EU");

        harness.get_by_label("Shop EU").click_secondary();
        harness.run();
        harness.get_by_label("Copy MCP configuration").click();
        harness.step();

        let copied = harness
            .output()
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .expect("the configuration is copied");
        let parsed: serde_json::Value = serde_json::from_str(&copied).unwrap();
        let entry = &parsed["mcpServers"]["dbdesk-shop-eu"];
        assert_eq!(
            entry["args"],
            serde_json::json!(["mcp", "--project", "shop-eu"])
        );
        let command = std::path::PathBuf::from(entry["command"].as_str().unwrap());
        assert!(command.is_absolute(), "{copied}");
    }

    #[test]
    fn groups_data_sources_into_projects_that_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let shop = save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
        let warehouse = dbdesk_core::DataSource {
            id: DataSourceId::generate(),
            name: "warehouse".to_owned(),
            ..shop.clone()
        };
        DataSourceFile::new(dir.path().join("data_sources.toml"))
            .save(&[shop, warehouse])
            .unwrap();
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "Expand warehouse");
        assert!(
            harness.query_by_label("No project").is_none(),
            "without projects there are no group headers"
        );

        // A new project from a data source's menu takes that data source with it.
        harness.get_by_label("shop").click_secondary();
        harness.run();
        harness.get_by_label("Move to project").hover();
        harness.run();
        harness.get_by_label("New project…").click();
        harness.run();
        harness
            .get_by_role_and_label(Role::TextInput, "Name")
            .type_text("Analytics");
        harness.run();
        harness.get_by_label("Create").click();
        step_until_visible(&mut harness, "Collapse Analytics");
        step_until(&mut harness, "shop moving into the project", |_| {
            saved_project_of(dir.path(), "shop").is_some()
        });
        harness.get_by_label("Collapse No project");
        assert_eq!(saved_project_of(dir.path(), "warehouse"), None);

        harness.get_by_label("Analytics").click_secondary();
        harness.run();
        harness.get_by_label("Rename…").click();
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness
            .get_by_role_and_label(Role::TextInput, "Name")
            .type_text("Data");
        harness.key_press(egui::Key::Enter);
        step_until_visible(&mut harness, "Collapse Data");

        // The filter matches project names: a matching project shows all its data sources.
        replace_filter(&mut harness, "", "dat");
        step_until(&mut harness, "warehouse being filtered out", |harness| {
            harness.query_by_label("Expand warehouse").is_none()
        });
        harness.get_by_label("Expand shop");
        replace_filter(&mut harness, "dat", "ware");
        step_until(&mut harness, "the project being filtered out", |harness| {
            harness.query_by_label("Collapse Data").is_none()
        });
        harness.get_by_label("Expand warehouse");
        replace_filter(&mut harness, "ware", "");
        drop(harness);

        let projects = ProjectFile::new(dir.path().join("projects.toml"))
            .load()
            .unwrap();
        let [project] = projects.projects.as_slice() else {
            panic!("expected one project, got {projects:?}");
        };
        assert_eq!(
            (
                project.name.as_str(),
                project.slug.as_str(),
                project.mcp_enabled
            ),
            ("Data", "analytics", false)
        );
        assert_eq!(saved_project_of(dir.path(), "shop"), Some(project.id));

        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "Collapse Data");
        harness.get_by_label("Data").click_secondary();
        harness.run();
        harness.get_by_label("Delete…").click();
        harness.run();
        harness.get_by_label_contains("Its data source moves to No project");
        harness.get_by_label("Delete").click();
        step_until(&mut harness, "the project going away", |harness| {
            harness.query_by_label("Collapse Data").is_none()
        });
        harness.get_by_label("Expand shop");
        step_until(&mut harness, "shop leaving the project", |_| {
            saved_project_of(dir.path(), "shop").is_none()
        });
    }

    #[test]
    fn asks_for_the_password_a_data_source_has_none_saved_for_then_connects() {
        let dir = tempfile::tempdir().unwrap();
        // A server that hangs up at once: the second attempt fails as soon as it is made. A
        // port nothing listens on can hang until the connect timeout on CI runners.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || listener.incoming().for_each(drop));
        let data_source = dbdesk_core::DataSource {
            id: DataSourceId::generate(),
            name: "shared".to_owned(),
            color: None,
            safety: dbdesk_core::SafetyLevel::Normal,
            project: None,
            mcp_login: None,
            config: dbdesk_core::ConnectionConfig::Postgres(dbdesk_core::PostgresConfig {
                server: dbdesk_core::ServerConfig {
                    host: "127.0.0.1".to_owned(),
                    port: port.into(),
                    database: "app".to_owned(),
                    user: "juan".to_owned(),
                    tls: dbdesk_core::TlsMode::Disable,
                    ssh: None,
                },
            }),
        };
        DataSourceFile::new(dir.path().join("data_sources.toml"))
            .save(std::slice::from_ref(&data_source))
            .unwrap();
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);

        step_until_visible(&mut harness, "Expand shared");
        harness.get_by_label("Expand shared").click();
        step_until_visible(&mut harness, "Connect to shared");
        harness.get_by_label_contains("no password is saved");
        let save = harness.get_by_label("Save to keychain");
        assert_eq!(
            egui_kittest::kittest::NodeT::accesskit_node(&save).toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        harness
            .get_by_role_and_label(Role::PasswordInput, "Password")
            .type_text("typed once");
        harness.run();
        harness.get_by_label("Connect").click();

        step_until(&mut harness, "the prompt closing", |harness| {
            harness
                .query_by_label_contains("Connect to shared")
                .is_none()
        });
        step_until(&mut harness, "connecting with the password", |harness| {
            harness
                .state()
                .state
                .data_source(data_source.id)
                .is_some_and(|entry| {
                    matches!(&entry.session, dbdesk_engine::SessionState::Failed(error)
                        if error.starts_with("could not connect"))
                })
        });
    }

    #[test]
    fn exports_a_project_with_its_data_sources_and_no_password() {
        let dir = tempfile::tempdir().unwrap();
        let shop = save_shop(dir.path(), "CREATE TABLE t (id INTEGER)");
        let export_path = dir.path().join("team.toml");
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        let picked = export_path.clone();
        harness.state_mut().pick_data_sources_export = Box::new(move || Some(picked.clone()));
        step_until_visible(&mut harness, "Expand shop");

        harness.get_by_label("File").click();
        harness.run();
        harness.get_by_label("Export data sources…").click();
        harness.run();
        harness
            .get_by_role_and_label(Role::CheckBox, "shop")
            .click();
        harness.run();
        harness.get_by_label_contains("1 data source chosen");
        harness.get_by_label("Export…").click();

        step_until_visible(&mut harness, "Exported 1 data source to");
        let text = std::fs::read_to_string(&export_path).unwrap();
        assert!(text.contains(&shop.id.to_string()), "{text}");
        assert!(text.contains(r#"path = "shop.db""#), "{text}");
    }

    #[test]
    fn previews_an_import_then_imports_what_was_chosen() {
        let dir = tempfile::tempdir().unwrap();
        let shop = save_shop(dir.path(), "CREATE TABLE t (id INTEGER)");
        let analytics = dbdesk_store::Project::new(
            dbdesk_core::ProjectId::generate(),
            "Analytics".to_owned(),
            dbdesk_store::ProjectSlug::from_name("Analytics"),
        );
        let changed_shop = dbdesk_core::DataSource {
            color: Some(dbdesk_core::SourceColor::Green),
            ..shop.clone()
        };
        let warehouse = dbdesk_core::DataSource {
            id: DataSourceId::generate(),
            name: "warehouse".to_owned(),
            project: Some(analytics.id),
            config: dbdesk_core::ConnectionConfig::Sqlite(dbdesk_core::SqliteConfig {
                path: dir.path().join("elsewhere/warehouse.db"),
                read_only: true,
            }),
            ..shop.clone()
        };
        let export_path = dir.path().join("team.toml");
        dbdesk_store::write_export(
            &export_path,
            &[changed_shop, warehouse],
            std::slice::from_ref(&analytics),
            None,
        )
        .unwrap();
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        let picked = export_path.clone();
        harness.state_mut().pick_data_sources_import = Box::new(move || Some(picked.clone()));
        step_until_visible(&mut harness, "Expand shop");

        harness.get_by_label("File").click();
        harness.run();
        harness.get_by_label("Import data sources…").click();
        step_until_visible(&mut harness, "Imports 1 project and 2 data sources.");
        harness.get_by_label_contains("Colour: none → Green");
        harness.get_by_label_contains("not found on this machine");
        for (entry, choice) in [
            ("Analytics", "Import"),
            ("shop", "Update"),
            ("warehouse", "Import"),
        ] {
            let label = format!("Import choice for {entry}");
            let combo = harness.get_by_label(&label);
            assert_eq!(combo.value().as_deref(), Some(choice), "{entry}");
        }
        harness
            .get_by_label("Import choice for shop")
            .click_accesskit();
        harness.run();
        harness.get_by_label("Skip").click_accesskit();
        harness.run();
        harness.get_by_label_contains("Imports 1 project and 1 data source.");
        harness.get_by_label("Import").click();

        step_until_visible(&mut harness, "Imported 1 project and 1 data source.");
        let saved = DataSourceFile::new(dir.path().join("data_sources.toml"))
            .load()
            .unwrap();
        assert_eq!(saved.len(), 2, "{saved:#?}");
        assert_eq!(saved[0], shop);
        assert_eq!(saved[1].name, "warehouse");
        assert_eq!(saved[1].project, Some(analytics.id));
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

    #[test]
    fn marks_and_filters_statements_from_mcp_clients() {
        let dir = tempfile::tempdir().unwrap();
        let shop = save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
        let from_assistant = "SELECT 'from an assistant'";
        dbdesk_store::HistoryFile::new(dir.path().join("history.sqlite"))
            .open()
            .unwrap()
            .record(&dbdesk_store::HistoryEntry {
                data_source: shop.id,
                executed_at: jiff::Timestamp::UNIX_EPOCH,
                sql: from_assistant.to_owned(),
                duration: None,
                rows: Some(1),
                outcome: dbdesk_store::HistoryOutcome::Ok,
                source: dbdesk_store::HistorySource::Mcp,
            })
            .unwrap();
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        open_orders(&mut harness, "0 rows");

        harness.get_by_label("History").click();
        step_until(&mut harness, "the history loading", |harness| {
            harness.query_all_by_label("Insert").count() == 2
        });
        harness.get_by_label("MCP");
        harness
            .get_by(|node| {
                node.role() == Role::ComboBox && node.value().as_deref() == Some("All sources")
            })
            .click();
        harness.run();
        harness.get_by_label("MCP clients").click();
        step_until(&mut harness, "the MCP statements alone", |harness| {
            harness.query_all_by_label("Insert").count() == 1
        });
        harness.get_by_label(from_assistant);
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
        // F5 runs what the console holds, so wait until the typed text has reached it: one frame
        // wasn't always enough on a busy CI runner, and F5 then ran an empty console.
        step_until(&mut harness, "the query to be typed", |harness| {
            editor_text(harness) == "SELECT 42 AS answer"
        });

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

        // The fake installer "installs" at once; the dialog and the status bar then ask for a
        // restart, which this test doesn't take, since it would close the harness's window.
        // A modal that has just opened is laid out again on its next frame; a click before then
        // can land outside it, which closes it.
        harness.run_steps(2);
        harness.get_by_label("Install and restart").click();
        step_until_visible(&mut harness, "dbdesk 99.0.0 is installed.");
        harness.get_by_label("Restart now");
        harness.get_by_label("Restart to finish updating");
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

    /// Edits the `ada` cell of the open orders to `Ada Lovelace`, leaving it to be submitted.
    fn rename_ada(harness: &mut Harness<'_, DbDeskApp>) {
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
        while !cell_editor(harness) {
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
    }

    /// Saves `shop` over a database of two orders, at `safety`.
    fn save_orders_at(dir: &Path, safety: dbdesk_core::SafetyLevel) {
        let shop = save_shop(
            dir,
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT);
             INSERT INTO orders VALUES (1, 'ada'), (2, 'bob');",
        );
        DataSourceFile::new(dir.join("data_sources.toml"))
            .save(&[dbdesk_core::DataSource { safety, ..shop }])
            .unwrap();
    }

    fn customer_of_order_1(dir: &Path) -> String {
        rusqlite::Connection::open(dir.join("shop.db"))
            .unwrap()
            .query_row("SELECT customer FROM orders WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    #[test]
    fn a_careful_data_source_asks_before_a_grid_edit_naming_itself() {
        let dir = tempfile::tempdir().unwrap();
        save_orders_at(dir.path(), dbdesk_core::SafetyLevel::Careful);
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        open_orders(&mut harness, "2 rows");
        harness.get_by_label_contains("Careful");
        harness.get_by(|node| {
            node.role() == Role::ComboBox && node.value().as_deref() == Some("Manual commit")
        });

        rename_ada(&mut harness);
        harness.get_by_label("Submit").click();
        step_until_visible(&mut harness, "Apply edits to \u{201c}shop\u{201d}?");
        harness
            .get_by_label_contains("\u{201c}shop\u{201d} is careful: this updates 1 edited row.");
        assert_eq!(
            customer_of_order_1(dir.path()),
            "ada",
            "nothing runs before the answer"
        );
        // The dialog settles where it is centred before a pointer can find its buttons.
        harness.run();
        harness.get_by_label("Apply").click();

        step_until(&mut harness, "the edit being saved", |harness| {
            harness.query_by_label("1 changed cell").is_none()
                && harness.query_by_label("Saving…").is_none()
        });
        // Manual commit mode opened a transaction with the first statement, so the edit waits
        // in it for Commit.
        harness.get_by_label("● Transaction open");
        assert_eq!(customer_of_order_1(dir.path()), "ada");
        harness.get_by_label("Commit").click();
        step_until_visible(&mut harness, "Committed");
        assert_eq!(customer_of_order_1(dir.path()), "Ada Lovelace");
    }

    #[test]
    fn a_read_only_data_source_says_why_it_refused_a_statement() {
        let dir = tempfile::tempdir().unwrap();
        save_orders_at(dir.path(), dbdesk_core::SafetyLevel::ReadOnly);
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        open_console(&mut harness);
        harness.get_by_label_contains("Read-only");

        harness
            .get_by_role(Role::MultilineTextInput)
            .type_text("UPDATE orders SET customer = 'x' WHERE id = 1");
        harness.run();
        harness.get_by_label("Run all").click();

        step_until_visible(
            &mut harness,
            "Refused: this statement changes data, and \u{201c}shop\u{201d} is read-only.",
        );
        assert_eq!(customer_of_order_1(dir.path()), "ada");
    }

    #[test]
    fn red_suggests_careful_in_the_data_source_dialog() {
        let dir = tempfile::tempdir().unwrap();
        let shop = save_shop(dir.path(), "CREATE TABLE t (id INTEGER)");
        let (_layer, feed) = log_console::pipeline();
        let mut harness = start_app(dir.path(), feed);
        step_until_visible(&mut harness, "Expand shop");
        harness.get_by_label("shop").click_secondary();
        harness.run();
        harness.get_by_label("Edit…").click();
        harness.run();

        harness.get_by_label("Red").click();
        harness.run();
        harness.get_by_label("Red usually marks production.");
        harness.get_by_label("Make it careful").click();
        harness.run();
        assert!(
            harness
                .query_by_label("Red usually marks production.")
                .is_none()
        );
        harness.get_by_label("Save").click();

        step_until(&mut harness, "the data source being saved", |_| {
            DataSourceFile::new(dir.path().join("data_sources.toml"))
                .load()
                .is_ok_and(|saved| {
                    saved.first().map(|ds| ds.safety) == Some(dbdesk_core::SafetyLevel::Careful)
                })
        });
        let saved = DataSourceFile::new(dir.path().join("data_sources.toml"))
            .load()
            .unwrap();
        assert_eq!(saved[0].id, shop.id);
        assert_eq!(saved[0].color, Some(dbdesk_core::SourceColor::Red));
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
        rename_ada(&mut harness);
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
