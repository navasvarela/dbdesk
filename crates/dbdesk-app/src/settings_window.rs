//! View → Settings: how SQL is formatted. Each change is saved to `settings.toml` at once.

use dbdesk_store::{KeywordCase, Settings};
use egui::{Grid, Window};

use crate::settings::{AppSettings, INDENT_WIDTHS};

pub enum SettingsWindowAction {
    /// Write these settings to the file.
    Save(Settings),
    Reload,
    Close,
}

pub fn show(ctx: &egui::Context, settings: &AppSettings) -> Option<SettingsWindowAction> {
    let mut open = true;
    let action = Window::new("Settings")
        .id(egui::Id::new("settings_window"))
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| contents(ui, settings))
        .and_then(|response| response.inner.flatten());
    if open {
        action
    } else {
        Some(SettingsWindowAction::Close)
    }
}

fn contents(ui: &mut egui::Ui, settings: &AppSettings) -> Option<SettingsWindowAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        if let Some(path) = settings.path() {
            ui.weak(path.display().to_string());
        }
        if ui
            .button("Reload")
            .on_hover_text("Read the file again, after editing it by hand")
            .clicked()
        {
            action = Some(SettingsWindowAction::Reload);
        }
    });
    if let Some(error) = settings.error() {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    ui.separator();
    ui.strong("SQL formatting");
    let mut draft = settings.settings().clone();
    Grid::new("format_settings")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Keywords");
            ui.horizontal(|ui| {
                for (case, label) in [
                    (KeywordCase::Upper, "UPPER"),
                    (KeywordCase::Lower, "lower"),
                    (KeywordCase::Preserve, "As typed"),
                ] {
                    ui.radio_value(&mut draft.format.keyword_case, case, label);
                }
            });
            ui.end_row();
            ui.label("Indent");
            egui::ComboBox::from_id_salt("indent_width")
                .selected_text(spaces(draft.format.indent_width))
                .show_ui(ui, |ui| {
                    for width in INDENT_WIDTHS {
                        ui.selectable_value(&mut draft.format.indent_width, width, spaces(width));
                    }
                });
            ui.end_row();
        });
    if &draft != settings.settings() {
        action = Some(SettingsWindowAction::Save(draft));
    }
    action
}

fn spaces(width: u8) -> String {
    match width {
        1 => "1 space".to_owned(),
        n => format!("{n} spaces"),
    }
}
