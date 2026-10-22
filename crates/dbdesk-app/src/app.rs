use std::time::Instant;

use dbdesk_core::{CatalogPath, DataSourceId};
use dbdesk_engine::{Command, EngineConfig, EngineError, EngineHandle, Event, Repaint};
use egui_dock::{DockArea, DockState, Style};

use crate::consoles::Consoles;
use crate::data_source_dialog::{DataSourceDialog, DialogAction};
use crate::definition_window::DefinitionWindow;
use crate::delete_dialog::{DeleteChoice, DeleteConfirmation};
use crate::explorer::{ExplorerAction, ExplorerOptions};
use crate::log_console::{LogConsole, LogFeed};
use crate::state::{AppState, EngineStatus};
use crate::workspace::{self, Panels, Tab};

const LAYOUT_STORAGE_KEY: &str = "dock_layout";

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
    consoles: Consoles,
}

impl DbDeskApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        log_feed: LogFeed,
        engine_config: EngineConfig,
    ) -> Result<Self, EngineError> {
        let dock = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, LAYOUT_STORAGE_KEY))
            .unwrap_or_else(workspace::default_layout);
        let engine = EngineHandle::start(engine_config, EguiRepaint(cc.egui_ctx.clone()))?;

        Ok(Self {
            engine: Some(engine),
            state: AppState::default(),
            dock,
            log_console: LogConsole::new(log_feed, &cc.egui_ctx),
            explorer_options: ExplorerOptions::default(),
            data_source_dialog: None,
            delete_confirmation: None,
            definition_window: None,
            consoles: Consoles::default(),
        })
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
        self.consoles.apply(event);
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

    fn shut_down_engine(&mut self) {
        if let Some(engine) = self.engine.take()
            && let Err(error) = engine.shutdown()
        {
            tracing::error!("engine shutdown failed: {:#}", anyhow::Error::new(error));
        }
        self.state.engine_status = EngineStatus::Stopped;
    }

    fn handle_explorer_action(&mut self, action: ExplorerAction) {
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
            ExplorerAction::Disconnect(id) => self.send(Command::Disconnect(id)),
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

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("View", |ui| {
                if ui.button("Reset layout").clicked() {
                    self.dock = workspace::default_layout();
                    tracing::info!("layout reset");
                }
            });
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.label(format!(
            "Engine {} · {} connected",
            self.state.engine_status.label(),
            self.state.connected_count()
        ));
    }
}

impl eframe::App for DbDeskApp {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_engine_events();
        self.log_console.drain();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status_bar").show(ui, |ui| self.status_bar(ui));

        let mut explorer_actions = Vec::new();
        let mut console_actions = Vec::new();
        let mut closed_consoles = Vec::new();
        let (open_consoles, saved_consoles) = self.consoles.open_and_saved_mut();
        DockArea::new(&mut self.dock)
            .style(Style::from_egui(ui.style()))
            .show_inside(
                ui,
                &mut Panels {
                    log_console: &mut self.log_console,
                    data_sources: &self.state.data_sources,
                    saved_consoles,
                    explorer_options: &mut self.explorer_options,
                    explorer_actions: &mut explorer_actions,
                    consoles: open_consoles,
                    console_actions: &mut console_actions,
                    closed_consoles: &mut closed_consoles,
                },
            );
        for action in explorer_actions {
            self.handle_explorer_action(action);
        }
        for (console, action) in console_actions {
            let command = self.consoles.handle(&console, action);
            self.send_if(command);
        }
        for console in closed_consoles {
            let commands = self.consoles.close(&console);
            self.send_all(commands);
        }

        let ctx = ui.ctx().clone();
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
        ConsoleFiles, ConsoleKey, ConsoleName, DataSourceFile, SecretError, SecretStore,
    };
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;
    use crate::log_console;

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
        EngineConfig {
            data_sources: DataSourceFile::new(dir.join("data_sources.toml")),
            secrets: Arc::new(NoSecrets),
            consoles: ConsoleFiles::new(dir.join("consoles")),
        }
    }

    fn start_app(dir: &Path, log_feed: LogFeed) -> Harness<'static, DbDeskApp> {
        let config = engine_config(dir);
        Harness::new_eframe(|cc| DbDeskApp::new(cc, log_feed, config).unwrap())
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
        harness.get_by_label_contains("Query consoles open here");
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

        harness.get_by_label("+").click();
        harness.run();
        harness.get_by_label("SQLite").click();
        harness.run();
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
        for expand in ["Expand shop", "Expand main", "Expand tables"] {
            step_until_visible(&mut harness, expand);
            harness.get_by_label(expand).click();
        }
        step_until_visible(&mut harness, "Expand orders");

        harness.get_by_label("orders").click_secondary();
        harness.run();
        harness.get_by_label("Open data").click();

        step_until_visible(&mut harness, "500 rows, more available");
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
        let mut harness = Harness::new_eframe(|cc| {
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
}
