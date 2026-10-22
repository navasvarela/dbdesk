//! The results panel: one tab per statement, a summary line, and a virtualized grid.

use dbdesk_core::{Cell, Row};
use dbdesk_engine::{RunId, StatementSummary};
use dbdesk_sql::first_word;
use egui::{Align, Label, Layout, RichText, Ui};
use egui_extras::{Column, TableBuilder};

use super::{ConsoleAction, QueryConsole, RunState, StatementView};

const COLUMN_WIDTH: f32 = 140.0;
const MIN_COLUMN_WIDTH: f32 = 40.0;
const ROW_NUMBER_WIDTH: f32 = 40.0;

/// Statements whose row count means "rows changed", as opposed to DDL, where the database
/// reports 0 or nothing meaningful.
const DATA_CHANGING: &[&str] = &["insert", "update", "delete", "merge", "replace", "copy"];

pub fn show(ui: &mut Ui, console: &mut QueryConsole, actions: &mut Vec<ConsoleAction>) {
    let dialect = console.dialect;
    let Some(run) = console.run.as_mut() else {
        ui.weak("Run the statement at the cursor with ⌘↵, or the whole console with ⇧⌘↵.");
        return;
    };
    if run.statements.len() > 1 {
        statement_tabs(ui, &mut run.selected, &run.statements);
    }
    let run_id = run.id;
    let running = run.state == RunState::Running;
    let Some(statement) = run.statements.get_mut(run.selected) else {
        return;
    };
    let load_clicked = summary(ui, statement, running, dialect);
    let last_row_shown = !statement.columns.is_empty() && grid(ui, run_id, statement);
    if (load_clicked || last_row_shown)
        && let Some(run) = console.request_more()
    {
        actions.push(ConsoleAction::FetchMore(run));
    }
}

fn statement_tabs(ui: &mut Ui, selected: &mut usize, statements: &[StatementView]) {
    ui.horizontal_wrapped(|ui| {
        for (index, statement) in statements.iter().enumerate() {
            let mut label = RichText::new(format!("{}", index + 1));
            if matches!(statement.result, Some(Err(_))) {
                label = label.color(ui.visuals().error_fg_color);
            }
            ui.selectable_value(selected, index, label)
                .on_hover_text(&statement.sql);
        }
    });
    ui.separator();
}

/// The line above the grid. Returns whether "Load more rows" was clicked. It lives here rather
/// than under the grid, because the grid takes all the remaining height.
fn summary(
    ui: &mut Ui,
    statement: &StatementView,
    running: bool,
    dialect: dbdesk_sql::Dialect,
) -> bool {
    ui.horizontal(|ui| {
        status(ui, statement, running, dialect);
        statement.more
            && ui
                .add_enabled(!statement.fetching, egui::Button::new("Load more rows"))
                .clicked()
    })
    .inner
}

fn status(ui: &mut Ui, statement: &StatementView, running: bool, dialect: dbdesk_sql::Dialect) {
    match &statement.result {
        Some(Err(failure)) => {
            ui.add(
                Label::new(RichText::new(&failure.message).color(ui.visuals().error_fg_color))
                    .wrap(),
            );
        }
        Some(Ok(summary)) if statement.columns.is_empty() => {
            let changes_data = first_word(&statement.sql, dialect)
                .is_some_and(|word| DATA_CHANGING.contains(&word.as_str()));
            let text = match summary.rows_changed {
                Some(rows) if changes_data => format!("{rows} rows affected"),
                _ => "Done".to_owned(),
            };
            ui.label(format!("{text} · {} ms", summary.elapsed.as_millis()));
        }
        Some(Ok(summary)) => {
            ui.label(row_count(statement, Some(summary)));
        }
        None if !statement.columns.is_empty() => {
            ui.label(row_count(statement, None));
            if statement.fetching {
                ui.spinner();
            }
        }
        None if running => {
            ui.spinner();
            ui.weak("Running…");
        }
        None => {
            ui.weak("Not run");
        }
    }
}

fn row_count(statement: &StatementView, summary: Option<&StatementSummary>) -> String {
    let rows = statement.rows.len();
    let note = if statement.more {
        ", more available"
    } else if summary.is_some_and(|summary| summary.truncated) {
        ", rest discarded"
    } else {
        ""
    };
    let elapsed = summary.map_or(String::new(), |summary| {
        format!(" · {} ms", summary.elapsed.as_millis())
    });
    format!("{rows} rows{note}{elapsed}")
}

/// Draws the rows. Returns whether the last loaded row is on screen, which (when more rows
/// exist) is the cue to load the next page: scrolling to the bottom loads more.
fn grid(ui: &mut Ui, run: RunId, statement: &StatementView) -> bool {
    let row_height = ui.spacing().interact_size.y;
    let mut last_row_visible = false;
    let mut table = TableBuilder::new(ui)
        .id_salt(("results_grid", run.0, statement.range.start))
        .striped(true)
        .resizable(true)
        .auto_shrink(false)
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::initial(ROW_NUMBER_WIDTH).at_least(ROW_NUMBER_WIDTH));
    for _ in &statement.columns {
        table = table.column(
            Column::initial(COLUMN_WIDTH)
                .at_least(MIN_COLUMN_WIDTH)
                .clip(true),
        );
    }
    table
        .header(row_height, |mut header| {
            header.col(|ui| {
                ui.weak("#");
            });
            for column in &statement.columns {
                header.col(|ui| {
                    let label = ui.strong(&column.name);
                    if let Some(type_name) = &column.type_name {
                        label.on_hover_text(type_name);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_height, statement.rows.len(), |mut row| {
                let index = row.index();
                let values = &statement.rows[index];
                row.col(|ui| {
                    ui.weak((index + 1).to_string());
                });
                for value in values {
                    row.col(|ui| cell(ui, value, values));
                }
                if index + 1 == statement.rows.len() {
                    last_row_visible = true;
                }
            });
        });

    last_row_visible
}

fn cell(ui: &mut Ui, value: &Cell, row: &Row) {
    let response = match value {
        None => ui.add(
            Label::new(RichText::new("NULL").italics().weak())
                .selectable(false)
                .sense(egui::Sense::click()),
        ),
        Some(text) if looks_numeric(text) => {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(Label::new(text).truncate().sense(egui::Sense::click()))
            })
            .inner
        }
        Some(text) => ui.add(Label::new(text).truncate().sense(egui::Sense::click())),
    };
    response.context_menu(|ui| {
        if ui.button("Copy value").clicked() {
            ui.ctx().copy_text(value.clone().unwrap_or_default());
        }
        if ui.button("Copy row").clicked() {
            ui.ctx().copy_text(row_as_tsv(row));
        }
    });
}

/// Right-align numbers, the way spreadsheets and DataGrip do. A text heuristic, because the
/// text protocol doesn't say which columns are numeric.
fn looks_numeric(text: &str) -> bool {
    !text.is_empty() && text.parse::<f64>().is_ok()
}

fn row_as_tsv(row: &Row) -> String {
    row.iter()
        .map(|cell| cell.as_deref().unwrap_or("NULL"))
        .collect::<Vec<_>>()
        .join("\t")
}
