//! View → Settings: how SQL is formatted and what the console checks while typing. Each
//! change is saved to `settings.toml` at once.

use dbdesk_sql::LintRule;
use dbdesk_store::{KeywordCase, Settings};
use egui::{Grid, RichText, Window};

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

/// The window's content width. Fixed, because an auto-sized window and its wrapping labels
/// would otherwise keep resizing each other.
const WIDTH: f32 = 560.0;

fn contents(ui: &mut egui::Ui, settings: &AppSettings) -> Option<SettingsWindowAction> {
    ui.set_width(WIDTH);
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
    for problem in settings.problems() {
        ui.colored_label(ui.visuals().warn_fg_color, problem);
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
    ui.separator();
    lint_settings(ui, &mut draft);
    if &draft != settings.settings() {
        action = Some(SettingsWindowAction::Save(draft));
    }
    action
}

/// The rules the console checks while typing, each switchable off, and whether to ask the
/// database.
fn lint_settings(ui: &mut egui::Ui, draft: &mut Settings) {
    ui.strong("Problems while typing");
    ui.checkbox(
        &mut draft.lint.ask_database,
        "Ask the database to prepare statements (connected consoles; nothing runs)",
    );
    for rule in LintRule::ALL {
        let mut enabled = !draft.lint.disabled.iter().any(|id| id == rule.id());
        let label = format!("{} ({})", rule.description(), rule.id());
        if ui.checkbox(&mut enabled, RichText::new(label)).changed() {
            if enabled {
                draft.lint.disabled.retain(|id| id != rule.id());
            } else {
                draft.lint.disabled.push(rule.id().to_owned());
            }
        }
    }
}

fn spaces(width: u8) -> String {
    match width {
        1 => "1 space".to_owned(),
        n => format!("{n} spaces"),
    }
}
