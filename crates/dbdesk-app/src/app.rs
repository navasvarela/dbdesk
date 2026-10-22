use dbdesk_engine::{Command, EngineConfig, EngineError, EngineHandle, Event, Repaint};
use egui_dock::{DockArea, DockState, Style};

use crate::data_source_dialog::{DataSourceDialog, DialogAction};
use crate::delete_dialog::{DeleteChoice, DeleteConfirmation};
use crate::explorer::ExplorerAction;
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
    data_source_dialog: Option<DataSourceDialog>,
    delete_confirmation: Option<DeleteConfirmation>,
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
            data_source_dialog: None,
            delete_confirmation: None,
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
        DockArea::new(&mut self.dock)
            .style(Style::from_egui(ui.style()))
            .show_inside(
                ui,
                &mut Panels {
                    log_console: &mut self.log_console,
                    data_sources: &self.state.data_sources,
                    explorer_actions: &mut explorer_actions,
                },
            );
        for action in explorer_actions {
            self.handle_explorer_action(action);
        }

        let ctx = ui.ctx().clone();
        self.show_data_source_dialog(&ctx);
        self.show_delete_confirmation(&ctx);

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
    use std::time::{Duration, Instant};

    use dbdesk_core::{DataSourceId, Password};
    use dbdesk_store::{DataSourceFile, SecretError, SecretStore};
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

    fn start_app(dir: &Path, log_feed: LogFeed) -> Harness<'static, DbDeskApp> {
        let config = EngineConfig {
            data_sources: DataSourceFile::new(dir.join("data_sources.toml")),
            secrets: Arc::new(NoSecrets),
        };
        Harness::new_eframe(|cc| DbDeskApp::new(cc, log_feed, config).unwrap())
    }

    /// Steps the app until a label containing `text` appears; engine events arrive
    /// asynchronously, so a fixed number of steps would be flaky.
    fn step_until_visible(harness: &mut Harness<'_, DbDeskApp>, text: &str) {
        let deadline = Instant::now() + WAIT_TIMEOUT;
        while harness.query_by_label_contains(text).is_none() {
            assert!(Instant::now() < deadline, "{text:?} never appeared");
            std::thread::sleep(Duration::from_millis(10));
            harness.step();
        }
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
        harness.get_by_label("Query consoles will open here.");
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

        step_until_visible(&mut harness, "chinook.db");
        let saved = DataSourceFile::new(dir.path().join("data_sources.toml"))
            .load()
            .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name, "chinook.db");
    }
}
