//! The connection explorer panel: the list of data sources and their connection state.
//! It only reports what the user asked for; the app turns that into engine commands.

use dbdesk_core::{DataSourceId, DriverKind};
use dbdesk_engine::SessionState;
use egui::{Color32, Label, ScrollArea, Sense, TextStyle, Ui, Visuals, vec2};

use crate::state::{DataSourceEntry, DataSources};
use crate::theme::SUCCESS_COLOR;

const STATUS_DOT_RADIUS: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplorerAction {
    New(DriverKind),
    Edit(DataSourceId),
    Delete(DataSourceId),
    Connect(DataSourceId),
    Disconnect(DataSourceId),
}

pub fn show(ui: &mut Ui, data_sources: &DataSources, actions: &mut Vec<ExplorerAction>) {
    toolbar(ui, actions);
    ui.separator();
    match data_sources {
        DataSources::Loading => {
            ui.weak("Loading data sources…");
        }
        DataSources::Unavailable(error) => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Can't read data sources: {error}"),
            );
        }
        DataSources::Loaded(entries) if entries.is_empty() => {
            ui.weak("No data sources yet. Add one with +.");
        }
        DataSources::Loaded(entries) => {
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for entry in entries {
                    row(ui, entry, actions);
                }
            });
        }
    }
}

fn toolbar(ui: &mut Ui, actions: &mut Vec<ExplorerAction>) {
    ui.horizontal(|ui| {
        ui.menu_button("+", |ui| {
            for driver in DriverKind::ALL {
                if ui.button(driver.display_name()).clicked() {
                    actions.push(ExplorerAction::New(driver));
                }
            }
        })
        .response
        .on_hover_text("New data source");
    });
}

fn row(ui: &mut Ui, entry: &DataSourceEntry, actions: &mut Vec<ExplorerAction>) {
    let response = ui
        .horizontal(|ui| {
            status_dot(ui, &entry.session);
            ui.add(
                Label::new(&entry.data_source.name)
                    .selectable(false)
                    .sense(Sense::click()),
            )
        })
        .inner
        .on_hover_text(tooltip(entry));

    if response.double_clicked() {
        actions.push(toggle_connection(entry));
    }
    response.context_menu(|ui| context_menu(ui, entry, actions));
}

fn context_menu(ui: &mut Ui, entry: &DataSourceEntry, actions: &mut Vec<ExplorerAction>) {
    let id = entry.data_source.id;
    let toggle = toggle_connection(entry);
    let toggle_label = match toggle {
        ExplorerAction::Disconnect(_) => "Disconnect",
        _ => "Connect",
    };
    if ui.button(toggle_label).clicked() {
        actions.push(toggle);
    }
    ui.separator();
    if ui.button("Edit…").clicked() {
        actions.push(ExplorerAction::Edit(id));
    }
    if ui.button("Delete…").clicked() {
        actions.push(ExplorerAction::Delete(id));
    }
}

/// Disconnect if connected or connecting, connect otherwise (including after a failure).
fn toggle_connection(entry: &DataSourceEntry) -> ExplorerAction {
    let id = entry.data_source.id;
    match entry.session {
        SessionState::Connecting | SessionState::Connected(_) => ExplorerAction::Disconnect(id),
        SessionState::Disconnected | SessionState::Failed(_) => ExplorerAction::Connect(id),
    }
}

fn status_dot(ui: &mut Ui, session: &SessionState) {
    let size = vec2(
        STATUS_DOT_RADIUS * 2.0,
        ui.text_style_height(&TextStyle::Body),
    );
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().circle_filled(
        rect.center(),
        STATUS_DOT_RADIUS,
        status_color(session, ui.visuals()),
    );
}

fn status_color(session: &SessionState, visuals: &Visuals) -> Color32 {
    match session {
        SessionState::Disconnected => visuals.weak_text_color(),
        SessionState::Connecting => visuals.warn_fg_color,
        SessionState::Connected(_) => SUCCESS_COLOR,
        SessionState::Failed(_) => visuals.error_fg_color,
    }
}

fn tooltip(entry: &DataSourceEntry) -> String {
    let config = &entry.data_source.config;
    let status = match &entry.session {
        SessionState::Disconnected => "Disconnected".to_owned(),
        SessionState::Connecting => "Connecting…".to_owned(),
        SessionState::Connected(server) => format!("Connected to {server}"),
        SessionState::Failed(error) => format!("Connection failed: {error}"),
    };
    format!("{} · {config}\n{status}", config.driver().display_name())
}
