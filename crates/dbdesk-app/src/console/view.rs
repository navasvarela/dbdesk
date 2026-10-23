//! Draws a console: toolbar, editor and results, and turns clicks and shortcuts into
//! [`ConsoleAction`]s for the app.

use egui::text::CCursorRange;
use egui::{Id, Key, KeyboardShortcut, Modifiers, ScrollArea, TextBuffer, TextEdit, Ui, vec2};

use super::{QueryConsole, RunScope, RunState, SubmittedStatement, highlight, results};
use crate::icons;
use crate::theme::Palette;
use crate::widgets::{self, IconButton};
use dbdesk_engine::{RunId, RunOutcome, SessionState};

const RUN_CURRENT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);
const RUN_ALL: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Enter);
const CANCEL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Period);

/// Height the results panel starts at; the user can drag it.
const RESULTS_HEIGHT: f32 = 280.0;

/// What the user asked a console to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleAction {
    Run(Vec<SubmittedStatement>),
    FetchMore(RunId),
    Cancel(RunId),
}

pub fn show(ui: &mut Ui, console: &mut QueryConsole, actions: &mut Vec<ConsoleAction>) {
    let editor_id = Id::new(("console_editor", console.id.0));
    shortcuts(ui, console, editor_id, actions);
    toolbar(ui, console, actions);
    ui.separator();
    egui::Panel::bottom(Id::new(("console_results", console.id.0)))
        .resizable(true)
        .default_size(RESULTS_HEIGHT)
        .show(ui, |ui| results::show(ui, console, actions));
    editor(ui, console, editor_id);
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
    let palette = Palette::for_visuals(ui.visuals());
    widgets::toolbar(ui, |ui| {
        let running = console.is_running();
        if IconButton::new(icons::PLAY, "Run")
            .color(palette.success)
            .enabled(!running)
            .tooltip(format!(
                "Run the selection or the statement at the cursor ({})",
                ui.ctx().format_shortcut(&RUN_CURRENT)
            ))
            .show(ui)
            .clicked()
        {
            push_run(console, RunScope::Current, actions);
        }
        if IconButton::new(icons::FAST_FORWARD, "Run all")
            .color(palette.success)
            .enabled(!running)
            .tooltip(format!(
                "Run every statement in the console ({})",
                ui.ctx().format_shortcut(&RUN_ALL)
            ))
            .show(ui)
            .clicked()
        {
            push_run(console, RunScope::All, actions);
        }
        let cancellable = console.cancellable_run();
        if IconButton::new(icons::STOP, "Cancel")
            .color(palette.error)
            .enabled(cancellable.is_some())
            .tooltip(format!(
                "Stop the running statement or discard unread rows ({})",
                ui.ctx().format_shortcut(&CANCEL)
            ))
            .show(ui)
            .clicked()
            && let Some(run) = cancellable
        {
            actions.push(ConsoleAction::Cancel(run));
        }
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);
        run_status(ui, console, palette);
    });
}

fn run_status(ui: &mut Ui, console: &QueryConsole, palette: &Palette) {
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
            let (glyph, color, text) = match outcome {
                RunOutcome::Completed => (
                    icons::CHECK_CIRCLE,
                    palette.success,
                    format!("Done in {millis} ms"),
                ),
                RunOutcome::Failed => (icons::X_CIRCLE, palette.error, "Failed".to_owned()),
                RunOutcome::Cancelled => (icons::PROHIBIT, palette.warning, "Cancelled".to_owned()),
            };
            widgets::status(ui, glyph, color, &text);
        }
        (SessionState::Connected(server), None) => {
            ui.weak(format!("Connected to {server}"));
        }
        (_, None) => {}
    }
}

fn editor(ui: &mut Ui, console: &mut QueryConsole, editor_id: Id) {
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
                .desired_width(f32::INFINITY)
                .min_size(vec2(0.0, ui.available_height()))
                .layouter(&mut layouter)
                .show(ui);
            if output.response.changed() {
                console.error_marker = None;
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
