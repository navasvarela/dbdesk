use dbdesk_sql::{Dialect, Hazard, hazard};
use dbdesk_store::ConsoleKey;
use egui::{Align, Id, Layout, Modal};

use crate::console::SubmittedStatement;

pub enum HazardChoice {
    RunAnyway,
    Cancel,
}

/// Asks before running statements that change every row of a table.
pub struct HazardConfirmation {
    pub console: ConsoleKey,
    pub statements: Vec<SubmittedStatement>,
    warnings: Vec<String>,
}

/// The outcome of checking statements before running them.
pub enum Review {
    Safe(Vec<SubmittedStatement>),
    NeedsConfirmation(HazardConfirmation),
}

/// Checks each statement for an `UPDATE` or `DELETE` that would change every row.
pub fn review(
    console: ConsoleKey,
    statements: Vec<SubmittedStatement>,
    dialect: Dialect,
) -> Review {
    let warnings: Vec<String> = statements
        .iter()
        .filter_map(|statement| {
            hazard(&statement.sql, dialect).map(|found| warning(found, &statement.sql))
        })
        .collect();
    if warnings.is_empty() {
        Review::Safe(statements)
    } else {
        Review::NeedsConfirmation(HazardConfirmation {
            console,
            statements,
            warnings,
        })
    }
}

impl HazardConfirmation {
    pub fn show(&self, ctx: &egui::Context) -> Option<HazardChoice> {
        let modal = Modal::new(Id::new("hazard_confirmation")).show(ctx, |ui| {
            ui.heading("Run anyway?");
            for warning in &self.warnings {
                ui.label(warning);
            }
            ui.separator();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Run anyway").clicked() {
                    Some(HazardChoice::RunAnyway)
                } else if ui.button("Cancel").clicked() {
                    Some(HazardChoice::Cancel)
                } else {
                    None
                }
            })
            .inner
        });
        if modal.should_close() {
            return Some(HazardChoice::Cancel);
        }
        modal.inner
    }
}

fn warning(found: Hazard, sql: &str) -> String {
    let first_line = sql.lines().next().unwrap_or_default();
    format!("{}\n    {first_line}", found.description())
}
