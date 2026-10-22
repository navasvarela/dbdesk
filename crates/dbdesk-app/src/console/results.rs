//! The results panel: one tab per statement, a summary line, and a virtualized grid whose
//! loaded rows can be sorted, filtered, selected, copied and exported.

use dbdesk_core::{Cell, ColumnInfo, Row};
use dbdesk_engine::{RunId, StatementSummary};
use dbdesk_sql::first_word;
use dbdesk_store::ExportFormat;
use egui::{Align, Label, Layout, RichText, Sense, TextEdit, Ui};
use egui_extras::{Column, TableBuilder};

use super::grid::{CellPos, GridView, SortKey};
use super::{ConsoleAction, QueryConsole, RunState, StatementView, ViewedCell};

const COLUMN_WIDTH: f32 = 140.0;
const MIN_COLUMN_WIDTH: f32 = 40.0;
const ROW_NUMBER_WIDTH: f32 = 40.0;
const FILTER_WIDTH: f32 = 180.0;

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
    let mut grid_output = GridOutput::default();
    if !statement.columns.is_empty() {
        grid_toolbar(ui, statement, console.export_status.as_ref(), actions);
        grid_output = grid(ui, run_id, statement);
    }
    if let Some(viewed) = grid_output.viewed {
        console.viewed_cell = Some(viewed);
    }
    if (load_clicked || grid_output.last_row_shown)
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

/// The filter, a note when the rows are rearranged, and the export menu.
fn grid_toolbar(
    ui: &mut Ui,
    statement: &mut StatementView,
    export_status: Option<&Result<String, String>>,
    actions: &mut Vec<ConsoleAction>,
) {
    ui.horizontal(|ui| {
        ui.add(
            TextEdit::singleline(statement.grid.filter_mut())
                .hint_text("Filter loaded rows")
                .desired_width(FILTER_WIDTH),
        );
        statement.grid.refresh(&statement.rows);
        if statement.grid.is_rearranged() {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "Sorted and filtered here, not by the database: {} of {} loaded rows",
                    statement.grid.order().len(),
                    statement.rows.len()
                ),
            )
            .on_hover_text("Rows not loaded yet aren't included until you load them");
        }
        ui.menu_button("Export", |ui| {
            for (label, format) in [("CSV…", ExportFormat::Csv), ("JSON…", ExportFormat::Json)]
            {
                if ui.button(label).clicked() {
                    actions.push(ConsoleAction::Export(format));
                }
            }
        })
        .response
        .on_hover_text("Save the rows shown, as sorted and filtered, to a file");
        match export_status {
            Some(Ok(done)) => {
                ui.weak(done);
            }
            Some(Err(error)) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            None => {}
        }
    });
}

#[derive(Default)]
struct GridOutput {
    /// The last loaded row is on screen, which (when more rows exist) is the cue to load the
    /// next page: scrolling to the bottom loads more.
    last_row_shown: bool,
    /// A cell the user asked to see in full.
    viewed: Option<ViewedCell>,
}

/// What the user did to a cell this frame, applied once the table is drawn.
enum CellInput {
    Select(CellPos),
    Extend(CellPos),
    StartDrag(CellPos),
    View(CellPos),
}

fn grid(ui: &mut Ui, run: RunId, statement: &mut StatementView) -> GridOutput {
    let StatementView {
        columns,
        rows,
        grid,
        range,
        ..
    } = statement;
    let row_height = ui.spacing().interact_size.y;
    let mut output = GridOutput::default();
    let mut inputs = Vec::new();
    let mut sort_clicked = None;
    let mut table = TableBuilder::new(ui)
        .id_salt(("results_grid", run.0, range.start))
        .striped(true)
        .resizable(true)
        .auto_shrink(false)
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::initial(ROW_NUMBER_WIDTH).at_least(ROW_NUMBER_WIDTH));
    for _ in columns.iter() {
        table = table.column(
            Column::initial(COLUMN_WIDTH)
                .at_least(MIN_COLUMN_WIDTH)
                .clip(true),
        );
    }
    let grid_view: &GridView = grid;
    table
        .header(row_height, |mut header| {
            header.col(|ui| {
                ui.weak("#");
            });
            for (index, column) in columns.iter().enumerate() {
                header.col(|ui| {
                    if column_header(ui, column, sort_arrow(grid_view.sort(), index)) {
                        sort_clicked = Some(index);
                    }
                });
            }
        })
        .body(|body| {
            let order = grid_view.order();
            body.rows(row_height, order.len(), |mut table_row| {
                let position = table_row.index();
                let loaded_index = order[position];
                let values = &rows[loaded_index];
                table_row.col(|ui| {
                    ui.weak((loaded_index + 1).to_string());
                });
                for (column, value) in values.iter().enumerate() {
                    let cell_pos = CellPos {
                        row: position,
                        column,
                    };
                    table_row.col(|ui| {
                        let context = CellContext {
                            grid: grid_view,
                            rows,
                            row: values,
                        };
                        if let Some(input) = cell(ui, cell_pos, value, &context) {
                            inputs.push(input);
                        }
                    });
                }
                if position + 1 == order.len() {
                    output.last_row_shown = true;
                }
            });
        });

    if let Some(column) = sort_clicked {
        grid.toggle_sort(column);
    }
    if ui.input(|input| input.pointer.any_released()) {
        grid.stop_drag();
    }
    for input in inputs {
        match input {
            CellInput::Select(cell) => grid.select(cell),
            CellInput::Extend(cell) => grid.extend_selection(cell),
            CellInput::StartDrag(cell) => grid.start_drag(cell),
            CellInput::View(cell) => {
                grid.select(cell);
                output.viewed = viewed_cell(columns, rows, grid, cell);
            }
        }
    }
    copy_selection_on_request(ui, grid, rows);
    if let Some(cell) = grid.selected_cell()
        && no_text_field_focused(ui)
        && ui.input(|input| input.key_pressed(egui::Key::Space))
    {
        output.viewed = viewed_cell(columns, rows, grid, cell);
    }
    output
}

/// Whether keys and ⌘C are free for the grid, rather than typing or copying in a text field.
fn no_text_field_focused(ui: &Ui) -> bool {
    ui.memory(|memory| memory.focused().is_none())
}

fn column_header(ui: &mut Ui, column: &ColumnInfo, arrow: &str) -> bool {
    let hover = match &column.type_name {
        Some(type_name) => format!("{type_name}\nClick to sort the loaded rows"),
        None => "Click to sort the loaded rows".to_owned(),
    };
    ui.add(
        Label::new(RichText::new(format!("{}{arrow}", column.name)).strong())
            .selectable(false)
            .sense(Sense::click()),
    )
    .on_hover_text(hover)
    .clicked()
}

fn sort_arrow(sort: Option<SortKey>, column: usize) -> &'static str {
    match sort {
        Some(SortKey {
            column: sorted,
            descending,
        }) if sorted == column => {
            if descending {
                " ▼"
            } else {
                " ▲"
            }
        }
        _ => "",
    }
}

/// What a cell needs to know besides its own value.
struct CellContext<'a> {
    grid: &'a GridView,
    rows: &'a [Row],
    row: &'a Row,
}

/// Draws a cell, highlighted if selected: click selects it, shift-click or dragging extends
/// the selection, double-click (or Space on a selected cell) opens it in the viewer.
fn cell(ui: &mut Ui, pos: CellPos, value: &Cell, context: &CellContext<'_>) -> Option<CellInput> {
    if context.grid.is_selected(pos) {
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, ui.visuals().selection.bg_fill);
    }
    let sense = Sense::click_and_drag();
    let response = match value {
        None => ui.add(
            Label::new(RichText::new("NULL").italics().weak())
                .selectable(false)
                .sense(sense),
        ),
        Some(text) if looks_numeric(text) => {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(Label::new(text).selectable(false).truncate().sense(sense))
            })
            .inner
        }
        Some(text) => ui.add(Label::new(text).selectable(false).truncate().sense(sense)),
    };
    let mut view = false;
    response.context_menu(|ui| {
        if let Some(tsv) = context.grid.selection_as_tsv(context.rows)
            && ui.button("Copy selection").clicked()
        {
            ui.ctx().copy_text(tsv);
        }
        if ui.button("Copy value").clicked() {
            ui.ctx().copy_text(value.clone().unwrap_or_default());
        }
        if ui.button("Copy row").clicked() {
            ui.ctx().copy_text(row_as_tsv(context.row));
        }
        view = ui.button("View value").clicked();
    });
    if view || response.double_clicked() {
        Some(CellInput::View(pos))
    } else if response.drag_started() {
        Some(CellInput::StartDrag(pos))
    } else if response.clicked() && ui.input(|input| input.modifiers.shift) {
        Some(CellInput::Extend(pos))
    } else if response.clicked() {
        Some(CellInput::Select(pos))
    } else if context.grid.is_dragging() && response.contains_pointer() {
        Some(CellInput::Extend(pos))
    } else {
        None
    }
}

fn viewed_cell(
    columns: &[ColumnInfo],
    rows: &[Row],
    grid: &GridView,
    cell: CellPos,
) -> Option<ViewedCell> {
    let row = rows.get(*grid.order().get(cell.row)?)?;
    Some(ViewedCell {
        column: columns.get(cell.column)?.name.clone(),
        value: row.get(cell.column)?.clone(),
    })
}

/// ⌘C copies the selected cells, unless a text field has focus and copies its own text.
fn copy_selection_on_request(ui: &Ui, grid: &GridView, rows: &[Row]) {
    let requested = ui.input(|input| {
        input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Copy))
    });
    if requested
        && no_text_field_focused(ui)
        && let Some(tsv) = grid.selection_as_tsv(rows)
    {
        ui.ctx().copy_text(tsv);
    }
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
