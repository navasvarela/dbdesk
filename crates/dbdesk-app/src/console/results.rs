//! The results panel: one tab per statement, a summary line, and a virtualized grid whose
//! loaded rows can be sorted, filtered, selected, copied and exported.

use dbdesk_core::{Cell, ColumnInfo, Row};
use dbdesk_engine::{RunId, StatementSummary};
use dbdesk_sql::first_word;
use dbdesk_store::ExportFormat;
use egui::{Align, Label, Layout, RichText, Sense, TextEdit, Ui};
use egui_extras::{Column, TableBuilder};

use super::edit::{CellEditor, EditSession, Editability, PendingEdits, RowRef};
use super::grid::{CellPos, GridView, SortKey};
use super::{ConsoleAction, QueryConsole, RunState, StatementView, ViewedCell};
use crate::keymap::{Action, Keymap};
use crate::theme::{GRID_HEADER_TINT, Palette};

const COLUMN_WIDTH: f32 = 140.0;
const MIN_COLUMN_WIDTH: f32 = 40.0;
const ROW_NUMBER_WIDTH: f32 = 40.0;
const FILTER_WIDTH: f32 = 180.0;
/// How strongly an edited cell is tinted with the warning colour: visible at a glance,
/// faint enough to read the value.
const PENDING_TINT: f32 = 0.25;

/// Statements whose row count means "rows changed", as opposed to DDL, where the database
/// reports 0 or nothing meaningful.
const DATA_CHANGING: &[&str] = &["insert", "update", "delete", "merge", "replace", "copy"];

pub fn show(
    ui: &mut Ui,
    console: &mut QueryConsole,
    keymap: &Keymap,
    actions: &mut Vec<ConsoleAction>,
) {
    let dialect = console.dialect;
    actions.extend(
        console
            .tables_to_describe()
            .into_iter()
            .map(ConsoleAction::DescribeTable),
    );
    actions.extend(console.take_reload().map(ConsoleAction::Run));
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
        grid_output = grid(ui, run_id, statement, keymap);
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
        ui.separator();
        edit_status(ui, statement, actions);
    });
}

/// Whether the rows can be edited, Add row and Delete rows, and the pending changes with
/// Submit and Revert.
fn edit_status(ui: &mut Ui, statement: &mut StatementView, actions: &mut Vec<ConsoleAction>) {
    let StatementView {
        edits, grid: view, ..
    } = statement;
    if let Editability::ReadOnly(reason) = &edits.editability {
        ui.weak("Read-only").on_hover_text(reason);
        return;
    }
    if edits.submitting {
        ui.spinner();
        ui.weak("Saving…");
        return;
    }
    if edits.can_change_rows() {
        if ui
            .button("Add row")
            .on_hover_text("Add a row, every column at its default until you type into it")
            .clicked()
        {
            edits.add_row();
        }
        let selected = selected_rows(view, edits.pending.new_rows());
        if ui
            .add_enabled(!selected.is_empty(), egui::Button::new("Delete rows"))
            .on_hover_text("Mark the selected cells' rows for deletion")
            .on_disabled_hover_text("Select a cell in each row to delete")
            .clicked()
        {
            edits.delete_rows(&selected);
        }
    }
    if let Some(summary) = pending_summary(&edits.pending) {
        ui.colored_label(ui.visuals().warn_fg_color, summary);
        if ui
            .button("Submit")
            .on_hover_text("Write the changes, one statement per row, all or none")
            .clicked()
        {
            actions.push(ConsoleAction::SubmitEdits);
        }
        if ui.button("Revert").clicked() {
            edits.revert();
        }
    }
    if let Some(error) = &edits.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
}

/// "2 changed cells · 1 new row · 1 deleted row", leaving out what there is none of; `None`
/// with nothing pending.
fn pending_summary(pending: &PendingEdits) -> Option<String> {
    let count = pending.count();
    let plural = |count: usize, one: &str, many: &str| match count {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {many}")),
    };
    let parts: Vec<String> = [
        plural(pending.changed_cells(), "changed cell", "changed cells"),
        plural(count.inserted, "new row", "new rows"),
        plural(count.deleted, "deleted row", "deleted rows"),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The rows the selection covers.
fn selected_rows(view: &GridView, new_rows: usize) -> Vec<RowRef> {
    view.selected_rows()
        .into_iter()
        .flatten()
        .filter_map(|position| row_at(view.order(), new_rows, position))
        .collect()
}

/// The row at a display position: the loaded rows in display order, then the added ones.
fn row_at(order: &[usize], new_rows: usize, position: usize) -> Option<RowRef> {
    match order.get(position) {
        Some(&loaded) => Some(RowRef::Loaded(loaded)),
        None => {
            let added = position - order.len();
            (added < new_rows).then_some(RowRef::New(added))
        }
    }
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
    /// Start typing into this cell.
    Edit {
        row: RowRef,
        column: usize,
    },
    SetNull {
        row: RowRef,
        column: usize,
    },
    CommitEdit,
    CancelEdit,
    Delete(RowRef),
    /// Unmark a loaded row marked for deletion.
    Restore(usize),
}

fn grid(ui: &mut Ui, run: RunId, statement: &mut StatementView, keymap: &Keymap) -> GridOutput {
    let StatementView {
        columns,
        rows,
        grid,
        range,
        edits,
        ..
    } = statement;
    let row_height = ui.spacing().interact_size.y;
    let palette = Palette::for_visuals(ui.visuals());
    let header_fill = palette.tint(palette.primary, GRID_HEADER_TINT);
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
    let defaults = new_row_defaults(columns, edits);
    table
        .header(row_height, |mut header| {
            header.col(|ui| {
                fill_header_cell(ui, header_fill);
                ui.weak("#");
            });
            for (index, column) in columns.iter().enumerate() {
                header.col(|ui| {
                    fill_header_cell(ui, header_fill);
                    if column_header(ui, column, sort_arrow(grid_view.sort(), index)) {
                        sort_clicked = Some(index);
                    }
                });
            }
        })
        .body(|body| {
            let order = grid_view.order();
            let new_rows = edits.pending.new_rows();
            body.rows(row_height, order.len() + new_rows, |mut table_row| {
                let position = table_row.index();
                let Some(row) = row_at(order, new_rows, position) else {
                    return;
                };
                table_row.col(|ui| row_number(ui, row));
                for column in 0..columns.len() {
                    let cell_pos = CellPos {
                        row: position,
                        column,
                    };
                    table_row.col(|ui| {
                        let input = match &mut edits.editing {
                            Some(editor) if editor.row == row && editor.column == column => {
                                cell_editor(ui, editor)
                            }
                            _ => {
                                let context = CellContext {
                                    grid: grid_view,
                                    rows,
                                    row,
                                    edits,
                                    defaults: &defaults,
                                    palette,
                                };
                                cell(ui, cell_pos, &context)
                            }
                        };
                        inputs.extend(input);
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
    // The Enter that closes an editor is still in this frame's input; it mustn't reopen it.
    let mut editor_closed = false;
    for input in inputs {
        match input {
            CellInput::Select(cell) => grid.select(cell),
            CellInput::Extend(cell) => grid.extend_selection(cell),
            CellInput::StartDrag(cell) => grid.start_drag(cell),
            CellInput::View(cell) => {
                grid.select(cell);
                output.viewed = viewed_cell(columns, rows, grid, cell);
            }
            CellInput::Edit { row, column } => start_editing(edits, rows, row, column),
            CellInput::SetNull { row, column } => edits.pending.set(rows, row, column, None),
            CellInput::CommitEdit => {
                editor_closed = true;
                if let Some(editor) = edits.editing.take() {
                    commit_edit(edits, rows, editor);
                }
            }
            CellInput::CancelEdit => {
                editor_closed = true;
                edits.editing = None;
            }
            CellInput::Delete(row) => edits.delete_rows(&[row]),
            CellInput::Restore(row) => edits.pending.restore(row),
        }
    }
    if let Some(cell) = grid.selected_cell()
        && !editor_closed
        && edits.editing.is_none()
        && no_text_field_focused(ui)
        && !ui
            .input_mut(|input| keymap.consume(input, &[Action::EditCell]))
            .is_empty()
        && let Some(row) = row_at(grid.order(), edits.pending.new_rows(), cell.row)
        && edits.is_editable(row, cell.column)
    {
        start_editing(edits, rows, row, cell.column);
    }
    copy_selection_on_request(ui, grid, rows);
    if let Some(cell) = grid.selected_cell()
        && no_text_field_focused(ui)
        && !ui
            .input_mut(|input| keymap.consume(input, &[Action::ViewCell]))
            .is_empty()
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

fn start_editing(edits: &mut EditSession, rows: &[Row], row: RowRef, column: usize) {
    let current = edits.pending.get(row, column).or_else(|| match row {
        RowRef::Loaded(loaded) => rows.get(loaded).and_then(|values| values.get(column)),
        RowRef::New(_) => None,
    });
    edits.editing = Some(CellEditor {
        row,
        column,
        text: current.cloned().flatten().unwrap_or_default(),
        focused: false,
    });
}

/// Keeps the editor's text as a pending change. Closing an added row's cell left at its
/// default without typing anything keeps the default, rather than setting it to empty text.
fn commit_edit(edits: &mut EditSession, rows: &[Row], editor: CellEditor) {
    let untouched_default = matches!(editor.row, RowRef::New(_))
        && editor.text.is_empty()
        && edits.pending.get(editor.row, editor.column).is_none();
    if !untouched_default {
        edits
            .pending
            .set(rows, editor.row, editor.column, Some(editor.text));
    }
}

/// The text field a cell turns into while edited. Enter or clicking away keeps the text as a
/// pending change; Escape drops it.
fn cell_editor(ui: &mut Ui, editor: &mut CellEditor) -> Option<CellInput> {
    let response = ui.add(TextEdit::singleline(&mut editor.text).desired_width(f32::INFINITY));
    if !editor.focused {
        response.request_focus();
        editor.focused = true;
        return None;
    }
    if !response.lost_focus() {
        return None;
    }
    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        Some(CellInput::CancelEdit)
    } else {
        Some(CellInput::CommitEdit)
    }
}

/// What an added row's cell holds until something is typed into it.
enum NewCell {
    /// The column's default, as the database keeps its expression.
    Default(String),
    /// No default: `NULL`, unless the column refuses it.
    Null,
    /// Not a column of the table, so not part of the row.
    Absent,
}

/// What each result column of an added row starts as.
fn new_row_defaults(columns: &[ColumnInfo], edits: &EditSession) -> Vec<NewCell> {
    let Some(target) = edits.target() else {
        return Vec::new();
    };
    columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let described = target
                .table
                .columns
                .iter()
                .find(|described| described.name == column.name);
            match described {
                Some(_) if !target.is_editable(index) => NewCell::Absent,
                Some(described) => match &described.default {
                    Some(default) => NewCell::Default(default.clone()),
                    None => NewCell::Null,
                },
                None => NewCell::Absent,
            }
        })
        .collect()
}

/// The row number, or `+` for an added row.
fn row_number(ui: &mut Ui, row: RowRef) {
    let text = match row {
        RowRef::Loaded(loaded) => (loaded + 1).to_string(),
        RowRef::New(_) => "+".to_owned(),
    };
    ui.label(
        RichText::new(text)
            .monospace()
            .color(ui.visuals().weak_text_color()),
    );
}

/// What a cell needs to know besides its own position.
struct CellContext<'a> {
    grid: &'a GridView,
    rows: &'a [Row],
    row: RowRef,
    edits: &'a EditSession,
    /// What an added row's cells start as, by column.
    defaults: &'a [NewCell],
    palette: &'a Palette,
}

impl CellContext<'_> {
    /// The loaded value of a cell of this row; `NULL` for an added row, which has none.
    fn loaded(&self, column: usize) -> &Cell {
        const NONE: &Cell = &None;
        match self.row {
            RowRef::Loaded(row) => self
                .rows
                .get(row)
                .and_then(|values| values.get(column))
                .unwrap_or(NONE),
            RowRef::New(_) => NONE,
        }
    }
}

/// What the user picked from a cell's context menu.
#[derive(Default)]
struct MenuChoice {
    view: bool,
    edit: bool,
    set_null: bool,
    delete: bool,
    restore: bool,
}

/// Draws a cell, highlighted if selected, and its pending value if it was edited: click
/// selects it, shift-click or dragging extends the selection, double-click edits an editable
/// cell and opens any other in the viewer (as Space does). A row marked for deletion is struck
/// through, and an added row's untouched cells show what the database will put there.
fn cell(ui: &mut Ui, pos: CellPos, context: &CellContext<'_>) -> Option<CellInput> {
    let (row, column) = (context.row, pos.column);
    let pending = context.edits.pending.get(row, column);
    let deleted = context.edits.pending.is_deleted(row);
    let tint = if deleted {
        Some(context.palette.error)
    } else if matches!(row, RowRef::New(_)) {
        Some(context.palette.success)
    } else {
        pending.map(|_| ui.visuals().warn_fg_color)
    };
    if context.grid.is_selected(pos) {
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, ui.visuals().selection.bg_fill);
    } else if let Some(tint) = tint {
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, tint.gamma_multiply(PENDING_TINT));
    }
    let value = match (pending, row) {
        (Some(value), _) => Some(value),
        (None, RowRef::Loaded(_)) => Some(context.loaded(column)),
        (None, RowRef::New(_)) => None,
    };
    let response = match value {
        Some(value) => value_label(ui, value, deleted),
        None => new_cell_label(ui, context.defaults.get(column)),
    };
    let editable = context.edits.is_editable(row, column);
    let choice = cell_menu(&response, context, value, editable);
    if choice.edit || (editable && response.double_clicked()) {
        Some(CellInput::Edit { row, column })
    } else if choice.set_null {
        Some(CellInput::SetNull { row, column })
    } else if choice.delete {
        Some(CellInput::Delete(row))
    } else if let (true, RowRef::Loaded(loaded)) = (choice.restore, row) {
        Some(CellInput::Restore(loaded))
    } else if choice.view || response.double_clicked() {
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

/// The cell's context menu: copying, viewing, editing the value, and deleting or restoring
/// its row.
fn cell_menu(
    response: &egui::Response,
    context: &CellContext<'_>,
    value: Option<&Cell>,
    editable: bool,
) -> MenuChoice {
    let mut choice = MenuChoice::default();
    response.context_menu(|ui| {
        if let Some(tsv) = context.grid.selection_as_tsv(context.rows)
            && ui.button("Copy selection").clicked()
        {
            ui.ctx().copy_text(tsv);
        }
        if ui.button("Copy value").clicked() {
            ui.ctx()
                .copy_text(value.cloned().flatten().unwrap_or_default());
        }
        if let RowRef::Loaded(loaded) = context.row
            && let Some(values) = context.rows.get(loaded)
            && ui.button("Copy row").clicked()
        {
            ui.ctx().copy_text(row_as_tsv(values));
        }
        choice.view = ui.button("View value").clicked();
        if editable {
            ui.separator();
            choice.edit = ui.button("Edit value").clicked();
            choice.set_null = ui.button("Set NULL").clicked();
        }
        if context.edits.can_change_rows() {
            ui.separator();
            match context.row {
                RowRef::Loaded(_) if context.edits.pending.is_deleted(context.row) => {
                    choice.restore = ui.button("Restore row").clicked();
                }
                RowRef::Loaded(_) => choice.delete = ui.button("Delete row").clicked(),
                RowRef::New(_) => choice.delete = ui.button("Remove row").clicked(),
            }
        }
    });
    choice
}

/// A value: `NULL` in italics, numbers right-aligned, and struck through in a row marked for
/// deletion.
fn value_label(ui: &mut Ui, value: &Cell, deleted: bool) -> egui::Response {
    let sense = Sense::click_and_drag();
    let styled = |text: RichText| if deleted { text.strikethrough() } else { text };
    match value {
        None => ui.add(
            Label::new(styled(RichText::new("NULL").monospace().italics().weak()))
                .selectable(false)
                .sense(sense),
        ),
        Some(text) if looks_numeric(text) => {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(
                    Label::new(styled(RichText::new(text).monospace()))
                        .selectable(false)
                        .truncate()
                        .sense(sense),
                )
            })
            .inner
        }
        Some(text) => ui.add(
            Label::new(styled(RichText::new(text).monospace()))
                .selectable(false)
                .truncate()
                .sense(sense),
        ),
    }
}

/// An added row's cell nothing was typed into: the column's default, `NULL`, or nothing for a
/// result column that isn't the table's.
fn new_cell_label(ui: &mut Ui, new: Option<&NewCell>) -> egui::Response {
    let (text, hover) = match new {
        Some(NewCell::Default(expression)) => ("DEFAULT", format!("The default: {expression}")),
        Some(NewCell::Null) => (
            "NULL",
            "No default, so NULL unless you type a value".to_owned(),
        ),
        Some(NewCell::Absent) | None => ("", "Not a column of the table".to_owned()),
    };
    ui.add(
        Label::new(RichText::new(text).monospace().italics().weak())
            .selectable(false)
            .sense(Sense::click_and_drag()),
    )
    .on_hover_text(hover)
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

/// Tints a header cell, extending over the column gaps so the header reads as one band.
fn fill_header_cell(ui: &Ui, fill: egui::Color32) {
    let rect = ui
        .max_rect()
        .expand2(egui::vec2(ui.spacing().item_spacing.x, 0.0));
    ui.painter().rect_filled(rect, 0.0, fill);
}

/// Right-align numbers, the way spreadsheets and database tools do. A text heuristic, because the
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
