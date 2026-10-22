//! Draws a console: toolbar, editor and results, and turns clicks and shortcuts into
//! [`ConsoleAction`]s for the app.

use std::time::Instant;

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{Id, Key, KeyboardShortcut, Modifiers, ScrollArea, TextBuffer, TextEdit, Ui, vec2};

use super::{QueryConsole, RunScope, RunState, SubmittedStatement, highlight, results};
use crate::theme::SUCCESS_COLOR;
use dbdesk_core::TransactionStatus;
use dbdesk_engine::{RunId, RunOutcome, SessionState, TransactionEnd, TransactionMode};

const RUN_CURRENT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);
const RUN_ALL: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Enter);
const CANCEL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Period);
const EXPLAIN: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::ALT), Key::E);

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
    /// Opens the data source's query history, to insert a past statement here.
    ShowHistory,
}

pub fn show(ui: &mut Ui, console: &mut QueryConsole, actions: &mut Vec<ConsoleAction>) {
    let editor_id = Id::new(("console_editor", console.id.0));
    shortcuts(ui, console, editor_id, actions);
    toolbar(ui, console, actions);
    transaction_bar(ui, console, actions);
    ui.separator();
    let (editor_height, results_height) = split_heights(ui, console);
    let width = ui.available_width();
    ui.allocate_ui(vec2(width, editor_height), |ui| {
        editor(ui, console, editor_id)
    });
    splitter(ui, console);
    ui.allocate_ui(vec2(width, results_height), |ui| {
        results::show(ui, console, actions);
    });
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
    console: &mut QueryConsole,
    editor_id: Id,
    actions: &mut Vec<ConsoleAction>,
) {
    if !ui.memory(|memory| memory.has_focus(editor_id)) {
        return;
    }
    // Checked before RUN_CURRENT, which would also match ⇧⌘↵.
    if ui.input_mut(|input| input.consume_shortcut(&RUN_ALL)) {
        push_run(console, RunScope::All, actions);
    } else if ui.input_mut(|input| input.consume_shortcut(&RUN_CURRENT)) {
        push_run(console, RunScope::Current, actions);
    }
    if ui.input_mut(|input| input.consume_shortcut(&EXPLAIN)) {
        push_run(console, RunScope::Explain, actions);
    }
    if ui.input_mut(|input| input.consume_shortcut(&CANCEL))
        && let Some(run) = console.cancellable_run()
    {
        actions.push(ConsoleAction::Cancel(run));
    }
}

fn push_run(console: &QueryConsole, scope: RunScope, actions: &mut Vec<ConsoleAction>) {
    let statements = console.statements_to_run(scope);
    if !statements.is_empty() {
        actions.push(ConsoleAction::Run(statements));
    }
}

fn toolbar(ui: &mut Ui, console: &QueryConsole, actions: &mut Vec<ConsoleAction>) {
    ui.horizontal(|ui| {
        let running = console.is_running();
        if ui
            .add_enabled(!running, egui::Button::new("Run"))
            .on_hover_text(format!(
                "Run the selection or the statement at the cursor ({})",
                ui.ctx().format_shortcut(&RUN_CURRENT)
            ))
            .clicked()
        {
            push_run(console, RunScope::Current, actions);
        }
        if ui
            .add_enabled(!running, egui::Button::new("Run all"))
            .on_hover_text(format!(
                "Run every statement in the console ({})",
                ui.ctx().format_shortcut(&RUN_ALL)
            ))
            .clicked()
        {
            push_run(console, RunScope::All, actions);
        }
        if ui
            .add_enabled(!running, egui::Button::new("Explain"))
            .on_hover_text(format!(
                "Show how the database would run the statement at the cursor ({})",
                ui.ctx().format_shortcut(&EXPLAIN)
            ))
            .clicked()
        {
            push_run(console, RunScope::Explain, actions);
        }
        let cancellable = console.cancellable_run();
        if ui
            .add_enabled(cancellable.is_some(), egui::Button::new("Cancel"))
            .on_hover_text(format!(
                "Stop the running statement or discard unread rows ({})",
                ui.ctx().format_shortcut(&CANCEL)
            ))
            .clicked()
            && let Some(run) = cancellable
        {
            actions.push(ConsoleAction::Cancel(run));
        }
        if ui
            .button("History")
            .on_hover_text("Search the statements run on this data source")
            .clicked()
        {
            actions.push(ConsoleAction::ShowHistory);
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
fn transaction_bar(ui: &mut Ui, console: &mut QueryConsole, actions: &mut Vec<ConsoleAction>) {
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
            for (label, end) in [
                ("Commit", TransactionEnd::Commit),
                ("Rollback", TransactionEnd::Rollback),
            ] {
                if ui.add_enabled(open, egui::Button::new(label)).clicked() {
                    actions.push(ConsoleAction::EndTransaction(end));
                }
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
    let error_marker = console.error_marker.clone();
    let mut layouter = |ui: &Ui, text: &dyn TextBuffer, wrap_width: f32| {
        let mut job =
            highlight::layout_job(text.as_str(), dialect, error_marker.as_ref(), ui.style());
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
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
                .layouter(&mut layouter)
                .show(ui);
            if output.response.changed() {
                console.edited(Instant::now());
            }
            if let Some(cursor) = output.cursor_range {
                console.selection = byte_range(&console.text, &cursor);
            }
        });
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
