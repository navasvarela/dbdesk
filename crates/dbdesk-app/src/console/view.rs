//! Draws a console: toolbar, editor and results, and turns clicks and shortcuts into
//! [`ConsoleAction`]s for the app.

use std::time::Instant;

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{Id, Margin, ScrollArea, TextBuffer, TextEdit, Ui, vec2};

use super::highlight::Marks;
use super::{
    QueryConsole, RunScope, RunState, SubmittedStatement, cell_viewer, highlight, results,
};
use crate::keymap::{Action, Keymap};
use crate::theme::SUCCESS_COLOR;
use dbdesk_core::TransactionStatus;
use dbdesk_engine::{RunId, RunOutcome, SessionState, TransactionEnd, TransactionMode};
use dbdesk_store::ExportFormat;

/// What the console's keys can do while its editor has focus.
const EDITOR_ACTIONS: [Action; 7] = [
    Action::RunCurrent,
    Action::RunAll,
    Action::Explain,
    Action::Cancel,
    Action::ShowHistory,
    Action::Commit,
    Action::Rollback,
];

/// Height the results start at; the user can drag the line above them.
const RESULTS_HEIGHT: f32 = 280.0;
/// The editor and the results each keep at least this share of the console's height, so
/// neither can be dragged (or squeezed by a small window) out of reach.
const MIN_SHARE: f32 = 0.2;
const SPLITTER_HEIGHT: f32 = 6.0;

/// What the user asked a console to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleAction {
    Run(Vec<SubmittedStatement>),
    FetchMore(RunId),
    Cancel(RunId),
    EndTransaction(TransactionEnd),
    /// Saves the rows the grid shows; the app asks where.
    Export(ExportFormat),
    /// Opens the data source's query history, to insert a past statement here.
    ShowHistory,
}

pub fn show(
    ui: &mut Ui,
    console: &mut QueryConsole,
    keymap: &Keymap,
    actions: &mut Vec<ConsoleAction>,
) {
    let editor_id = Id::new(("console_editor", console.id.0));
    shortcuts(ui, console, keymap, editor_id, actions);
    toolbar(ui, console, keymap, actions);
    transaction_bar(ui, console, keymap, actions);
    ui.separator();
    let (editor_height, results_height) = split_heights(ui, console);
    let width = ui.available_width();
    ui.allocate_ui(vec2(width, editor_height), |ui| {
        editor(ui, console, editor_id)
    });
    splitter(ui, console);
    ui.allocate_ui(vec2(width, results_height), |ui| {
        results::show(ui, console, keymap, actions);
    });
    cell_viewer::show(ui.ctx(), console);
}

/// The editor's and the results' heights, from the results height the user last dragged to.
///
/// Laid out by hand rather than with a resizable `egui::Panel`: a panel lays its contents out
/// at full size even when the console is too short for it, and the editor below then covers
/// (and takes the clicks of) the results' summary line.
fn split_heights(ui: &Ui, console: &QueryConsole) -> (f32, f32) {
    let total = (ui.available_height() - SPLITTER_HEIGHT).max(0.0);
    let wanted = ui.data(|data| {
        data.get_temp(results_height_id(console))
            .unwrap_or(RESULTS_HEIGHT)
    });
    let results = wanted.clamp(total * MIN_SHARE, total * (1.0 - MIN_SHARE));
    (total - results, results)
}

fn results_height_id(console: &QueryConsole) -> Id {
    Id::new(("console_results_height", console.id.0))
}

/// The line between editor and results, dragged to resize them.
fn splitter(ui: &mut Ui, console: &QueryConsole) {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), SPLITTER_HEIGHT),
        egui::Sense::drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    if response.dragged() {
        let id = results_height_id(console);
        let height = ui.data(|data| data.get_temp(id).unwrap_or(RESULTS_HEIGHT));
        ui.data_mut(|data| data.insert_temp(id, height - response.drag_delta().y));
    }
    let stroke = if response.hovered() || response.dragged() {
        ui.visuals().widgets.hovered.fg_stroke
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke
    };
    ui.painter().hline(rect.x_range(), rect.center().y, stroke);
}

/// Shortcuts act only while this console's editor has focus, and are consumed before the
/// editor sees them, so ⌘↵ runs instead of inserting a newline.
fn shortcuts(
    ui: &mut Ui,
    console: &QueryConsole,
    keymap: &Keymap,
    editor_id: Id,
    actions: &mut Vec<ConsoleAction>,
) {
    if !ui.memory(|memory| memory.has_focus(editor_id)) {
        return;
    }
    for action in ui.input_mut(|input| keymap.consume(input, &EDITOR_ACTIONS)) {
        perform(console, action, actions);
    }
}

/// Does what a toolbar button or a shortcut asks, if it can be done now.
fn perform(console: &QueryConsole, action: Action, actions: &mut Vec<ConsoleAction>) {
    if !is_enabled(console, action) {
        return;
    }
    let scope = match action {
        Action::RunCurrent => RunScope::Current,
        Action::RunAll => RunScope::All,
        Action::Explain => RunScope::Explain,
        Action::Cancel => {
            actions.extend(console.cancellable_run().map(ConsoleAction::Cancel));
            return;
        }
        Action::ShowHistory => {
            actions.push(ConsoleAction::ShowHistory);
            return;
        }
        Action::Commit => {
            actions.push(ConsoleAction::EndTransaction(TransactionEnd::Commit));
            return;
        }
        Action::Rollback => {
            actions.push(ConsoleAction::EndTransaction(TransactionEnd::Rollback));
            return;
        }
        // The grid handles its own keys.
        Action::ViewCell => return,
    };
    let statements = console.statements_to_run(scope);
    if !statements.is_empty() {
        actions.push(ConsoleAction::Run(statements));
    }
}

fn is_enabled(console: &QueryConsole, action: Action) -> bool {
    match action {
        Action::RunCurrent | Action::RunAll | Action::Explain => !console.is_running(),
        Action::Cancel => console.cancellable_run().is_some(),
        Action::Commit | Action::Rollback => console.has_open_transaction(),
        Action::ShowHistory | Action::ViewCell => true,
    }
}

/// A toolbar button for `action`, described with its shortcut, if it has one.
fn action_button(
    ui: &mut Ui,
    console: &QueryConsole,
    keymap: &Keymap,
    (label, action): (&str, Action),
    actions: &mut Vec<ConsoleAction>,
) {
    let hint = match keymap.hint(ui.ctx(), action) {
        Some(shortcut) => format!("{} ({shortcut})", action.description()),
        None => action.description().to_owned(),
    };
    if ui
        .add_enabled(is_enabled(console, action), egui::Button::new(label))
        .on_hover_text(hint)
        .clicked()
    {
        perform(console, action, actions);
    }
}

fn toolbar(ui: &mut Ui, console: &QueryConsole, keymap: &Keymap, actions: &mut Vec<ConsoleAction>) {
    ui.horizontal(|ui| {
        for button in [
            ("Run", Action::RunCurrent),
            ("Run all", Action::RunAll),
            ("Explain", Action::Explain),
            ("Cancel", Action::Cancel),
            ("History", Action::ShowHistory),
        ] {
            action_button(ui, console, keymap, button, actions);
        }
        ui.separator();
        run_status(ui, console);
        if let Some(error) = console.save_error() {
            ui.colored_label(ui.visuals().error_fg_color, "Not saved")
                .on_hover_text(error);
        }
    });
}

/// The transaction mode, Commit and Rollback, and whether a transaction is pending. The
/// buttons show in manual mode, or whenever a transaction is open (one the user began with
/// their own `BEGIN`, say).
fn transaction_bar(
    ui: &mut Ui,
    console: &mut QueryConsole,
    keymap: &Keymap,
    actions: &mut Vec<ConsoleAction>,
) {
    ui.horizontal(|ui| {
        let open = console.has_open_transaction();
        ui.add_enabled_ui(!open, |ui| {
            egui::ComboBox::from_id_salt(("transaction_mode", console.id.0))
                .selected_text(mode_label(console.transactions))
                .show_ui(ui, |ui| {
                    for mode in [TransactionMode::Auto, TransactionMode::Manual] {
                        ui.selectable_value(&mut console.transactions, mode, mode_label(mode));
                    }
                })
        })
        .response
        .on_disabled_hover_text("Commit or roll back the open transaction first");
        if console.transactions == TransactionMode::Manual || open {
            for button in [("Commit", Action::Commit), ("Rollback", Action::Rollback)] {
                action_button(ui, console, keymap, button, actions);
            }
        }
        transaction_status(ui, console);
    });
}

fn mode_label(mode: TransactionMode) -> &'static str {
    match mode {
        TransactionMode::Auto => "Auto-commit",
        TransactionMode::Manual => "Manual commit",
    }
}

fn transaction_status(ui: &mut Ui, console: &QueryConsole) {
    match (console.transaction, &console.transaction_ended) {
        (TransactionStatus::InTransaction, _) => {
            ui.colored_label(ui.visuals().warn_fg_color, "● Transaction open")
                .on_hover_text("Changes stay pending until you commit or roll back");
        }
        (TransactionStatus::Failed, _) => {
            ui.colored_label(ui.visuals().error_fg_color, "● Transaction failed")
                .on_hover_text(
                    "A statement failed, so the database only accepts Rollback now \
                     (Commit rolls back too)",
                );
        }
        (TransactionStatus::Idle, Some(Ok(TransactionEnd::Commit))) => {
            ui.colored_label(SUCCESS_COLOR, "Committed");
        }
        (TransactionStatus::Idle, Some(Ok(TransactionEnd::Rollback))) => {
            ui.weak("Rolled back");
        }
        (TransactionStatus::Idle, Some(Err(_)) | None) => {}
    }
    // A failed commit can leave the transaction open (SQLite's "database is locked", say),
    // so the error shows whatever the status.
    if let Some(Err(error)) = &console.transaction_ended {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
}

fn run_status(ui: &mut Ui, console: &QueryConsole) {
    match (&console.session, console.run.as_ref().map(|run| run.state)) {
        (SessionState::Connecting, _) => {
            ui.spinner();
            ui.weak("Connecting…");
        }
        (_, Some(RunState::Running)) => {
            ui.spinner();
            ui.weak("Running…");
        }
        (_, Some(RunState::Finished { outcome, elapsed })) => {
            let millis = elapsed.as_millis();
            match outcome {
                RunOutcome::Completed => {
                    ui.colored_label(SUCCESS_COLOR, format!("Done in {millis} ms"));
                }
                RunOutcome::Failed => {
                    ui.colored_label(ui.visuals().error_fg_color, "Failed");
                }
                RunOutcome::Cancelled => {
                    ui.colored_label(ui.visuals().warn_fg_color, "Cancelled");
                }
            }
        }
        (SessionState::Connected(server), None) => {
            ui.weak(format!("Connected to {server}"));
        }
        (_, None) => {}
    }
}

/// Applies a cursor move requested from outside the editor, and focuses it.
fn move_cursor(ui: &Ui, console: &mut QueryConsole, editor_id: Id) {
    let Some(offset) = console.pending_cursor.take() else {
        return;
    };
    let char_index = console
        .text
        .get(..offset)
        .unwrap_or(&console.text)
        .chars()
        .count();
    let mut state = TextEditState::load(ui.ctx(), editor_id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(char_index))));
    state.store(ui.ctx(), editor_id);
    ui.memory_mut(|memory| memory.request_focus(editor_id));
}

fn editor(ui: &mut Ui, console: &mut QueryConsole, editor_id: Id) {
    move_cursor(ui, console, editor_id);
    let dialect = console.dialect;
    let marks = Marks {
        error: console.error_marker.clone(),
        current_statement: console.current_statement(),
    };
    let mut layouter = |ui: &Ui, text: &dyn TextBuffer, wrap_width: f32| {
        let mut job = highlight::layout_job(text.as_str(), dialect, &marks, ui.style());
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let gutter_width = gutter_width(ui, &console.text);
    ScrollArea::vertical()
        .id_salt(("console_editor_scroll", console.id.0))
        .auto_shrink(false)
        .show(ui, |ui| {
            let output = TextEdit::multiline(&mut console.text)
                .id(editor_id)
                .code_editor()
                .hint_text("SELECT …")
                // Fills the height given by `min_size`; one row keeps it from overflowing
                // that height when the console is short.
                .desired_rows(1)
                .desired_width(f32::INFINITY)
                .min_size(vec2(0.0, ui.available_height()))
                .margin(Margin {
                    left: gutter_width,
                    ..TEXT_MARGIN
                })
                .layouter(&mut layouter)
                .show(ui);
            line_numbers(ui, &output);
            if output.response.changed() {
                console.edited(Instant::now());
            }
            if let Some(cursor) = output.cursor_range {
                console.selection = byte_range(&console.text, &cursor);
            }
        });
}

/// The editor's own margin around the text; the line numbers widen its left side.
const TEXT_MARGIN: Margin = Margin::symmetric(4, 2);
/// Space between the line numbers and the text.
const GUTTER_GAP: f32 = 12.0;

/// The editor's left margin: room for the largest line number, so the text doesn't shift
/// until a line count needs another digit.
fn gutter_width(ui: &Ui, text: &str) -> i8 {
    let lines = text.lines().count().max(1) + usize::from(text.ends_with('\n'));
    let digits = lines.to_string().len();
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let digit_width = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
    let width = f32::from(TEXT_MARGIN.left) + digits as f32 * digit_width + GUTTER_GAP;
    // Margins are whole points in an `i8`; 127 points hold over a dozen digits.
    width.ceil().min(f32::from(i8::MAX)) as i8
}

/// Numbers each line of the editor's text in the gutter. A line that wraps is numbered on
/// its first row only.
fn line_numbers(ui: &Ui, output: &egui::text_edit::TextEditOutput) {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let color = ui.visuals().weak_text_color();
    let right_edge = output.galley_pos.x - GUTTER_GAP;
    let rows = &output.galley.rows;
    let ends: Vec<bool> = rows.iter().map(|row| row.ends_with_newline).collect();
    for (line, row_index) in line_starts(&ends).into_iter().enumerate() {
        let y = output.galley_pos.y + rows[row_index].rect().center().y;
        ui.painter().text(
            egui::pos2(right_edge, y),
            egui::Align2::RIGHT_CENTER,
            (line + 1).to_string(),
            font.clone(),
            color,
        );
    }
}

/// The indices of the rows that start a line, given whether each row ends with a line break
/// (rather than wrapping onto the next).
fn line_starts(ends_with_newline: &[bool]) -> Vec<usize> {
    (0..ends_with_newline.len())
        .filter(|&row| row == 0 || ends_with_newline[row - 1])
        .collect()
}

/// Converts the editor's character-index cursor into byte offsets into `text`.
fn byte_range(text: &str, cursor: &CCursorRange) -> std::ops::Range<usize> {
    let byte = |char_index: usize| {
        text.char_indices()
            .nth(char_index)
            .map_or(text.len(), |(byte, _)| byte)
    };
    byte(cursor.secondary.index.0)..byte(cursor.primary.index.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_only_the_first_row_of_a_wrapped_line() {
        // Line 1 wraps onto a second row; lines 2 and 3 fit on one row each.
        assert_eq!(line_starts(&[false, true, true, false]), [0, 2, 3]);
        assert_eq!(line_starts(&[false]), [0]);
        assert!(line_starts(&[]).is_empty());
    }
}
