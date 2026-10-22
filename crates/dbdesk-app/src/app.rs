use dbdesk_engine::{EngineError, EngineHandle, Repaint};
use egui_dock::{DockArea, DockState, Style};

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
}

impl DbDeskApp {
    pub fn new(cc: &eframe::CreationContext<'_>, log_feed: LogFeed) -> Result<Self, EngineError> {
        let dock = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, LAYOUT_STORAGE_KEY))
            .unwrap_or_else(workspace::default_layout);
        let engine = EngineHandle::start(EguiRepaint(cc.egui_ctx.clone()))?;

        Ok(Self {
            engine: Some(engine),
            state: AppState::default(),
            dock,
            log_console: LogConsole::new(log_feed, &cc.egui_ctx),
        })
    }

    fn drain_engine_events(&mut self) {
        let Some(engine) = &mut self.engine else {
            return;
        };
        loop {
            match engine.try_recv() {
                Ok(Some(event)) => self.state.apply(event),
                Ok(None) => return,
                Err(error) => {
                    tracing::error!("{error}");
                    self.shut_down_engine();
                    return;
                }
            }
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
        ui.label(format!("Engine {}", self.state.engine_status.label()));
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
        DockArea::new(&mut self.dock)
            .style(Style::from_egui(ui.style()))
            .show_inside(
                ui,
                &mut Panels {
                    log_console: &mut self.log_console,
                },
            );

        // Entries logged on the UI thread don't wake the UI (see `UiWaker`), so collect any
        // logged during this frame and ask for one more frame to show them.
        if self.log_console.drain() {
            ui.ctx().request_repaint();
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
    use std::time::{Duration, Instant};

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;
    use crate::log_console;

    const ENGINE_START_TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn shows_every_panel_the_engine_status_and_log_entries() {
        let (layer, feed) = log_console::pipeline();
        // Thread-local, so it only captures events logged on this (the UI) thread.
        let _subscriber =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
        tracing::info!("logged before the window opened");

        let mut harness = Harness::new_eframe(|cc| DbDeskApp::new(cc, feed).unwrap());
        let deadline = Instant::now() + ENGINE_START_TIMEOUT;
        while harness.query_by_label("Engine running").is_none() {
            assert!(
                Instant::now() < deadline,
                "engine never reported it started"
            );
            std::thread::sleep(Duration::from_millis(10));
            harness.step();
        }

        // egui_dock doesn't expose tab titles to accessibility, so check each panel's content.
        harness.get_by_label("No data sources yet.");
        harness.get_by_label("Query consoles will open here.");
        harness.get_by_label_contains("logged before the window opened");
    }
}
