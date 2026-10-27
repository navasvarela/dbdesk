//! View → Settings: how SQL is formatted, what the console checks while typing, whether to
//! look for updates, and how long the query history keeps statements, each change saved to
//! `settings.toml` at once; and the MCP defaults, saved to `projects.toml` with their own
//! button.

use std::num::NonZeroU32;

use dbdesk_sql::LintRule;
use dbdesk_store::{HistorySettings, KeywordCase, McpPermissions, Settings};
use egui::{Grid, RichText, Window};

use crate::mcp_settings::McpDefaultsEditor;
use crate::settings::{AppSettings, INDENT_WIDTHS};

pub enum SettingsWindowAction {
    /// Write these settings to the file.
    Save(Settings),
    /// Write these MCP defaults to `projects.toml`.
    SaveMcpDefaults(McpPermissions),
    Reload,
    Close,
}

pub fn show(
    ctx: &egui::Context,
    settings: &AppSettings,
    mcp: &mut McpDefaultsEditor,
) -> Option<SettingsWindowAction> {
    let mut open = true;
    let action = Window::new("Settings")
        .id(egui::Id::new("settings_window"))
        .open(&mut open)
        // The MCP section made the window taller than a small screen.
        .vscroll(true)
        .resizable(false)
        .show(ctx, |ui| {
            let action = contents(ui, settings);
            ui.separator();
            ui.strong("MCP");
            action.or_else(|| mcp.show(ui).map(SettingsWindowAction::SaveMcpDefaults))
        })
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
    ui.separator();
    ui.strong("Updates");
    ui.checkbox(
        &mut draft.updates.check_on_start,
        "Check for a new release when dbdesk starts",
    )
    .on_hover_text("Asks GitHub once per start. Help › Check for updates works either way.");
    ui.separator();
    history_settings(ui, &mut draft.history);
    if &draft != settings.settings() {
        action = Some(SettingsWindowAction::Save(draft));
    }
    action
}

/// How long the query history keeps statements. Chosen from a few lengths rather than typed:
/// every change is saved, and prunes, at once, so typing 365 would delete everything older
/// than 3 days on the way.
fn history_settings(ui: &mut egui::Ui, history: &mut HistorySettings) {
    ui.strong("Query history");
    Grid::new("history_settings")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Keep statements for");
            limit_choice(
                ui,
                "history_max_age",
                &mut history.max_age_days,
                &AGES,
                |days| format!("{days} days"),
            );
            ui.end_row();
            ui.label("Keep per data source");
            limit_choice(
                ui,
                "history_max_count",
                &mut history.max_per_data_source,
                &COUNTS,
                |count| format!("the newest {count}"),
            );
            ui.end_row();
        });
    ui.weak("Older statements are deleted at start-up and when this changes.");
}

/// The lengths offered for the history's age, in days.
const AGES: [(u32, &str); 4] = [
    (30, "30 days"),
    (90, "90 days"),
    (365, "a year"),
    (730, "two years"),
];

/// The counts offered per data source.
const COUNTS: [(u32, &str); 3] = [
    (10_000, "the newest 10,000"),
    (100_000, "the newest 100,000"),
    (1_000_000, "the newest 1,000,000"),
];

/// A menu of `choices` and no limit. A value set by hand in `settings.toml` shows as it is.
fn limit_choice(
    ui: &mut egui::Ui,
    id: &str,
    limit: &mut Option<NonZeroU32>,
    choices: &[(u32, &str)],
    custom: impl Fn(u32) -> String,
) {
    let label = |value: Option<NonZeroU32>| match value {
        None => "no limit".to_owned(),
        Some(value) => choices
            .iter()
            .find(|(choice, _)| *choice == value.get())
            .map_or_else(|| custom(value.get()), |(_, label)| (*label).to_owned()),
    };
    egui::ComboBox::from_id_salt(id)
        .selected_text(label(*limit))
        .show_ui(ui, |ui| {
            ui.selectable_value(limit, None, "no limit");
            for &(choice, text) in choices {
                ui.selectable_value(limit, NonZeroU32::new(choice), text);
            }
        });
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
