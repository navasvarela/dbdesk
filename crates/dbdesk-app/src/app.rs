use dbdesk_core::{CatalogPath, DataSourceId};
use dbdesk_engine::{Command, EngineConfig, EngineError, EngineHandle, Event, Repaint};
use egui::{Frame, Margin, RichText, ThemePreference};
use egui_dock::{DockArea, DockState};

use crate::consoles::Consoles;
use crate::data_source_dialog::{DataSourceDialog, DialogAction};
use crate::definition_window::DefinitionWindow;
use crate::delete_dialog::{DeleteChoice, DeleteConfirmation};
use crate::explorer::{ExplorerAction, ExplorerOptions};
use crate::log_console::{LogConsole, LogFeed};
use crate::state::{AppState, EngineStatus};
use crate::theme::{self, Palette};
use crate::workspace::{self, Panels, Tab};
use crate::{fonts, icons, widgets};

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
            .map_or_else(workspace::default_layout, workspace::without_consoles);
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
                    let console = self.consoles.open(entry, String::new());
                    workspace::open_console_tab(&mut self.dock, console);
                }
            }
            ExplorerAction::OpenData(id, path) => {
                let opened = self
                    .state
                    .data_source(id)
                    .and_then(|entry| self.consoles.open_data(entry, &path));
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
                ui.menu_button("Theme", theme_menu);
                if ui.button("Reset layout").clicked() {
                    self.dock = workspace::default_layout();
                    tracing::info!("layout reset");
                }
            });
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
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
        });
    }
}

impl eframe::App for DbDeskApp {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
        let mut closed_consoles = Vec::new();
        let palette = Palette::for_visuals(ui.visuals());
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
                    explorer_options: &mut self.explorer_options,
                    explorer_actions: &mut explorer_actions,
                    consoles: self.consoles.open_mut(),
                    console_actions: &mut console_actions,
                    closed_consoles: &mut closed_consoles,
                },
            );
        for action in explorer_actions {
            self.handle_explorer_action(action);
        }
        for (console, action) in console_actions {
            let command = self.consoles.handle(console, action);
            self.send_if(command);
        }
        for console in closed_consoles {
            let command = self.consoles.close(console);
            self.send(command);
        }

        let ctx = ui.ctx().clone();
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

    use dbdesk_core::DataSourceId;
    use dbdesk_store::DataSourceFile;
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::{Harness, HarnessBuilder};
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;
    use crate::log_console;
    use crate::test_support::{MemorySecrets, step_until_visible};

    fn start_app(dir: &Path, log_feed: LogFeed) -> Harness<'static, DbDeskApp> {
        crate::test_support::start_app(
            HarnessBuilder::default().with_size(crate::DEFAULT_WINDOW_SIZE),
            dir,
            log_feed,
            MemorySecrets::default(),
        )
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
        let database = dir.path().join("shop.db");
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute_batch("CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT NOT NULL)")
            .unwrap();
        DataSourceFile::new(dir.path().join("data_sources.toml"))
            .save(&[dbdesk_core::DataSource {
                id: DataSourceId::generate(),
                name: "shop".to_owned(),
                color: None,
                config: dbdesk_core::ConnectionConfig::Sqlite(dbdesk_core::SqliteConfig {
                    path: database,
                    read_only: true,
                }),
            }])
            .unwrap();
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
        let database = dir.path().join("shop.db");
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute_batch(
                "CREATE TABLE orders (id INTEGER PRIMARY KEY);
                 WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 600)
                 INSERT INTO orders SELECT n FROM s;",
            )
            .unwrap();
        DataSourceFile::new(dir.path().join("data_sources.toml"))
            .save(&[dbdesk_core::DataSource {
                id: DataSourceId::generate(),
                name: "shop".to_owned(),
                color: None,
                config: dbdesk_core::ConnectionConfig::Sqlite(dbdesk_core::SqliteConfig {
                    path: database,
                    read_only: true,
                }),
            }])
            .unwrap();
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
}
