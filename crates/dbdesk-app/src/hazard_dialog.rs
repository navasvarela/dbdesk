use dbdesk_sql::{Dialect, Hazard, StatementKind, classify, commits_implicitly, hazard};
use dbdesk_store::ConsoleKey;
use egui::{Align, Id, Layout, Modal};

use crate::console::{ChangeCount, SubmittedRun};

pub enum HazardChoice {
    RunAnyway,
    Cancel,
}

/// Shown for a statement that commits on its own (DDL on MySQL) when the user may be counting
/// on Rollback.
const COMMITS_AT_ONCE: &str = "This commits at once, together with anything the open \
     transaction holds, so Rollback can't undo it.";

/// Asks before running statements that change every row of a table, or that commit by
/// themselves while the user may be counting on Rollback; and, on a careful data source,
/// before any change at all.
pub struct HazardConfirmation {
    pub console: ConsoleKey,
    pub pending: Pending,
    heading: String,
    warnings: Vec<String>,
}

/// What runs once the user confirms.
pub enum Pending {
    Run(SubmittedRun),
    /// The grid's pending edits.
    Edits,
}

/// The outcome of checking statements before running them.
pub enum Review {
    Safe(SubmittedRun),
    NeedsConfirmation(HazardConfirmation),
}

/// What the console the statements run in adds to the review.
#[derive(Debug, Clone, Copy)]
pub struct ReviewContext<'a> {
    pub dialect: Dialect,
    /// The console commits by hand, or a transaction is open: a statement that commits by
    /// itself would undo what the user expects Rollback to do.
    pub relies_on_rollback: bool,
    /// The data source's name, when it is careful: then every change asks first.
    pub careful: Option<&'a str>,
}

/// Checks each statement for an `UPDATE` or `DELETE` that would change every row, and, when
/// the user relies on Rollback, for one that commits by itself.
pub fn review(console: ConsoleKey, run: SubmittedRun, context: ReviewContext<'_>) -> Review {
    let warnings: Vec<String> = run
        .statements
        .iter()
        .flat_map(|statement| statement_warnings(&statement.sql, context))
        .collect();
    if warnings.is_empty() {
        return Review::Safe(run);
    }
    let heading = match context.careful {
        Some(data_source) => format!("Run on \u{201c}{data_source}\u{201d}?"),
        None => "Run anyway?".to_owned(),
    };
    Review::NeedsConfirmation(HazardConfirmation {
        console,
        pending: Pending::Run(run),
        heading,
        warnings,
    })
}

/// Asks before applying the grid's pending changes to a careful data source.
pub fn confirm_edits(
    console: ConsoleKey,
    data_source: &str,
    changes: ChangeCount,
) -> HazardConfirmation {
    HazardConfirmation {
        console,
        pending: Pending::Edits,
        heading: format!("Apply edits to \u{201c}{data_source}\u{201d}?"),
        warnings: vec![format!(
            "\u{201c}{data_source}\u{201d} is careful: this {}.",
            changes.describe()
        )],
    }
}

impl HazardConfirmation {
    pub fn show(&self, ctx: &egui::Context) -> Option<HazardChoice> {
        let confirm = match self.pending {
            Pending::Run(_) => "Run anyway",
            Pending::Edits => "Apply",
        };
        let modal = Modal::new(Id::new("hazard_confirmation")).show(ctx, |ui| {
            ui.heading(&self.heading);
            for warning in &self.warnings {
                ui.label(warning);
            }
            ui.separator();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(confirm).clicked() {
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

fn statement_warnings(sql: &str, context: ReviewContext<'_>) -> Vec<String> {
    let careful = context
        .careful
        .and_then(|data_source| careful_warning(sql, context.dialect, data_source));
    let rows = hazard(sql, context.dialect).map(Hazard::description);
    let commit = (context.relies_on_rollback && commits_implicitly(sql, context.dialect))
        .then_some(COMMITS_AT_ONCE);
    careful
        .into_iter()
        .chain(rows.into_iter().chain(commit).map(str::to_owned))
        .map(|description| warning(&description, sql))
        .collect()
}

/// On a careful data source, what a statement that may change something does. Reading and
/// ending a transaction ask nothing: whatever the transaction changed asked already.
fn careful_warning(sql: &str, dialect: Dialect, data_source: &str) -> Option<String> {
    let what = match classify(sql, dialect) {
        Ok(StatementKind::Read | StatementKind::Transaction) => return None,
        Ok(kind) => format!("this statement {}", kind.description()),
        Err(_) => "what this statement does can't be told".to_owned(),
    };
    Some(format!(
        "\u{201c}{data_source}\u{201d} is careful, and {what}."
    ))
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
            careful: None,
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

    #[test]
    fn a_careful_data_source_asks_before_every_change_naming_itself() {
        let context = ReviewContext {
            dialect: Dialect::Postgres,
            relies_on_rollback: false,
            careful: Some("production"),
        };
        let cases = [
            ("SELECT * FROM orders", None),
            ("COMMIT", None),
            (
                "INSERT INTO orders VALUES (1)",
                Some("\u{201c}production\u{201d} is careful, and this statement changes data."),
            ),
            (
                "DROP TABLE orders",
                Some(
                    "\u{201c}production\u{201d} is careful, and this statement changes the schema.",
                ),
            ),
        ];
        for (sql, expected) in cases {
            let warnings = statement_warnings(sql, context);
            assert_eq!(
                warnings
                    .first()
                    .map(|w| w.lines().next().unwrap_or_default()),
                expected,
                "{sql}"
            );
        }
        assert_eq!(statement_warnings("DELETE FROM orders", context).len(), 2);
    }
}
