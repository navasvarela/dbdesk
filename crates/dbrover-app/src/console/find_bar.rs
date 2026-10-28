//! Draws the find bar above the editor, and turns its fields, buttons and keys into searches
//! and replacements.

use std::time::Instant;

use egui::{Id, Key, Response, TextEdit, Ui, WidgetType};

use super::{Found, QueryConsole};
use crate::icons;
use crate::keymap::{Action, Keymap};
use crate::theme::Palette;
use crate::widgets::{self, IconButton};

/// What the find bar's keys can do while one of its fields has focus.
const FIELD_ACTIONS: [Action; 4] = [
    Action::Find,
    Action::Replace,
    Action::FindNext,
    Action::FindPrevious,
];

const FIELD_WIDTH: f32 = 240.0;

/// Draws the bar, if it is open, and returns the matches it found, for the editor to highlight.
pub(super) fn show(ui: &mut Ui, console: &mut QueryConsole, keymap: &Keymap) -> Found {
    if !console.find_bar().open {
        return Found::default();
    }
    let find_id = Id::new(("find_field", console.id.0));
    let replace_id = Id::new(("replace_field", console.id.0));
    field_shortcuts(ui, console, keymap, [find_id, replace_id]);
    if !console.find_bar().open {
        return Found::default();
    }
    ui.horizontal(|ui| find_row(ui, console, keymap, find_id));
    if console.find_bar().replacing {
        ui.horizontal(|ui| replace_row(ui, console, replace_id));
    }
    console.found()
}

/// The keymap's find shortcuts, while the find or replace field has focus: the editor's own
/// shortcuts only act while it has focus.
fn field_shortcuts(ui: &mut Ui, console: &mut QueryConsole, keymap: &Keymap, fields: [Id; 2]) {
    if !fields
        .iter()
        .any(|id| ui.memory(|memory| memory.has_focus(*id)))
    {
        return;
    }
    for action in ui.input_mut(|input| keymap.consume(input, &FIELD_ACTIONS)) {
        match action {
            Action::Find => console.open_find(false),
            Action::Replace => console.open_find(true),
            Action::FindNext => console.find_next(true),
            Action::FindPrevious => console.find_next(false),
            _ => {}
        }
    }
}

fn find_row(ui: &mut Ui, console: &mut QueryConsole, keymap: &Keymap, find_id: Id) {
    let palette = &Palette::for_visuals(ui.visuals());
    let bar = console.find_bar_mut();
    let field = ui.add(
        TextEdit::singleline(&mut bar.query)
            .id(find_id)
            .hint_text("Find")
            .desired_width(FIELD_WIDTH),
    );
    let field = widgets::name(field, WidgetType::TextEdit, "Find", true);
    if std::mem::take(&mut bar.focus) {
        field.request_focus();
    }
    if field.changed() {
        bar.replaced = None;
    }
    // A click on any of the bar's buttons gives the keyboard focus back to the find field, so
    // Enter, Escape and the find shortcuts keep working.
    let mut clicked = false;
    for (selected, text, label) in [
        (&mut bar.options.match_case, "Aa", "Match case"),
        (&mut bar.options.whole_word, "W", "Whole word"),
        (&mut bar.options.regex, ".*", "Regex"),
    ] {
        let toggle = ui.toggle_value(selected, text).on_hover_text(label);
        clicked |= widgets::name(toggle, WidgetType::Checkbox, label, true).clicked();
    }
    // A changed search makes "Replaced 3" stale.
    if clicked {
        bar.replaced = None;
    }
    let previous = hinted_button(ui, keymap, icons::ARROW_UP, Action::FindPrevious);
    let next = hinted_button(ui, keymap, icons::ARROW_DOWN, Action::FindNext);
    let close = IconButton::new(icons::X, "Close find").show(ui);
    let (enter, shift, escape) = ui.input(|input| {
        (
            input.key_pressed(Key::Enter),
            input.modifiers.shift,
            input.key_pressed(Key::Escape),
        )
    });
    if field.lost_focus() && escape || close.clicked() {
        console.close_find();
        return;
    }
    if field.lost_focus() && enter {
        console.find_next(!shift);
        field.request_focus();
    }
    if previous.clicked() {
        console.find_next(false);
    }
    if next.clicked() {
        console.find_next(true);
    }
    if clicked || previous.clicked() || next.clicked() {
        console.find_bar_mut().focus = true;
    }
    status(ui, console, palette);
}

/// "3 of 12", "No results", how many were replaced, or why the regular expression is invalid.
fn status(ui: &mut Ui, console: &QueryConsole, palette: &Palette) {
    let found = console.found();
    if let Some(error) = &found.error {
        ui.colored_label(palette.error, "Invalid regular expression")
            .on_hover_text(error);
        return;
    }
    if let Some(count) = console.find_bar().replaced {
        ui.weak(match count {
            1 => "Replaced 1".to_owned(),
            n => format!("Replaced {n}"),
        });
        return;
    }
    if console.find_bar().query.is_empty() {
        return;
    }
    match (found.current, found.matches.len()) {
        (_, 0) => ui.colored_label(palette.warning, "No results"),
        (Some(current), count) => ui.weak(format!("{} of {count}", current + 1)),
        (None, 1) => ui.weak("1 match"),
        (None, count) => ui.weak(format!("{count} matches")),
    };
}

fn replace_row(ui: &mut Ui, console: &mut QueryConsole, replace_id: Id) {
    let bar = console.find_bar_mut();
    let field = ui.add(
        TextEdit::singleline(&mut bar.replacement)
            .id(replace_id)
            .hint_text("Replace")
            .desired_width(FIELD_WIDTH),
    );
    let field = widgets::name(field, WidgetType::TextEdit, "Replace", true);
    let replace = ui
        .button("Replace")
        .on_hover_text("Replace the selected match and go to the next");
    let replace_all = ui
        .button("Replace all")
        .on_hover_text("Replace every match");
    let (enter, escape) = ui.input(|input| {
        (
            input.key_pressed(Key::Enter),
            input.key_pressed(Key::Escape),
        )
    });
    if field.lost_focus() && escape {
        console.close_find();
        return;
    }
    if replace.clicked() || field.lost_focus() && enter {
        console.replace_current(Instant::now());
        if !replace.clicked() {
            field.request_focus();
        }
    }
    if replace_all.clicked() {
        let count = console.replace_all(Instant::now());
        console.find_bar_mut().replaced = Some(count);
    }
}

/// An icon button for `action`, with its shortcut in the tooltip.
fn hinted_button(ui: &mut Ui, keymap: &Keymap, glyph: &str, action: Action) -> Response {
    let hint = match keymap.hint(ui.ctx(), action) {
        Some(shortcut) => format!("{} ({shortcut})", action.description()),
        None => action.description().to_owned(),
    };
    IconButton::new(glyph, action.description())
        .tooltip(hint)
        .show(ui)
}
