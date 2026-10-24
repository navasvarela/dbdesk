use dbdesk_sql::{Dialect, Hazard, commits_implicitly, hazard};
use dbdesk_store::ConsoleKey;
use egui::{Align, Id, Layout, Modal};

use crate::console::SubmittedRun;

pub enum HazardChoice {
    RunAnyway,
    Cancel,
}

/// Shown for a statement that commits on its own (DDL on MySQL) when the user may be counting
/// on Rollback.
const COMMITS_AT_ONCE: &str = "This commits at once, together with anything the open \
     transaction holds, so Rollback can't undo it.";

/// Asks before running statements that change every row of a table, or that commit by
/// themselves while the user may be counting on Rollback.
pub struct HazardConfirmation {
    pub console: ConsoleKey,
    pub run: SubmittedRun,
    warnings: Vec<String>,
}

/// The outcome of checking statements before running them.
pub enum Review {
    Safe(SubmittedRun),
    NeedsConfirmation(HazardConfirmation),
}

/// What the console the statements run in adds to the review.
#[derive(Debug, Clone, Copy)]
pub struct ReviewContext {
    pub dialect: Dialect,
    /// The console commits by hand, or a transaction is open: a statement that commits by
    /// itself would undo what the user expects Rollback to do.
    pub relies_on_rollback: bool,
}

/// Checks each statement for an `UPDATE` or `DELETE` that would change every row, and, when
/// the user relies on Rollback, for one that commits by itself.
pub fn review(console: ConsoleKey, run: SubmittedRun, context: ReviewContext) -> Review {
    let warnings: Vec<String> = run
        .statements
        .iter()
        .flat_map(|statement| statement_warnings(&statement.sql, context))
        .collect();
    if warnings.is_empty() {
        Review::Safe(run)
    } else {
        Review::NeedsConfirmation(HazardConfirmation {
            console,
            run,
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

fn statement_warnings(sql: &str, context: ReviewContext) -> Vec<String> {
    let rows = hazard(sql, context.dialect).map(Hazard::description);
    let commit = (context.relies_on_rollback && commits_implicitly(sql, context.dialect))
        .then_some(COMMITS_AT_ONCE);
    rows.into_iter()
        .chain(commit)
        .map(|description| warning(description, sql))
        .collect()
}

fn warning(description: &str, sql: &str) -> String {
    let first_line = sql.lines().next().unwrap_or_default();
    format!("{description}\n    {first_line}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_of_an_implicit_commit_only_where_rollback_was_counted_on() {
        let context = |dialect, relies_on_rollback| ReviewContext {
            dialect,
            relies_on_rollback,
        };
        let cases = [
            ("CREATE TABLE t (n int)", context(Dialect::Mysql, true), 1),
            ("CREATE TABLE t (n int)", context(Dialect::Mysql, false), 0),
            (
                "CREATE TEMPORARY TABLE t (n int)",
                context(Dialect::Mysql, true),
                0,
            ),
            (
                "CREATE TABLE t (n int)",
                context(Dialect::Postgres, true),
                0,
            ),
            ("DELETE FROM t", context(Dialect::Mysql, true), 1),
        ];
        for (sql, context, expected) in cases {
            let warnings = statement_warnings(sql, context);
            assert_eq!(warnings.len(), expected, "{sql} {context:?}: {warnings:?}");
        }
        assert!(
            statement_warnings("DROP TABLE t", context(Dialect::Mysql, true))[0]
                .starts_with(COMMITS_AT_ONCE)
        );
    }
}
