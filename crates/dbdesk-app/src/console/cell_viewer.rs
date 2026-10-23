//! A window showing one cell's whole value, for text too long for the grid. JSON objects and
//! arrays are pretty-printed.

use egui::{Id, RichText, ScrollArea, TextEdit, Window};

use super::{QueryConsole, ViewedCell};

const DEFAULT_SIZE: [f32; 2] = [520.0, 360.0];

/// Draws the console's cell viewer, if one is open, and closes it when the user does.
pub fn show(ctx: &egui::Context, console: &mut QueryConsole) {
    let Some(viewed) = &console.viewed_cell else {
        return;
    };
    let mut open = true;
    Window::new(format!("Value of {}", viewed.column))
        .id(Id::new(("cell_viewer", console.id.0)))
        .open(&mut open)
        .default_size(DEFAULT_SIZE)
        .show(ctx, |ui| contents(ui, viewed));
    if !open {
        console.viewed_cell = None;
    }
}

fn contents(ui: &mut egui::Ui, viewed: &ViewedCell) {
    let Some(value) = &viewed.value else {
        ui.label(RichText::new("NULL").italics().weak());
        return;
    };
    let pretty = pretty_json(value);
    ui.horizontal(|ui| {
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(value.clone());
        }
        if pretty.is_some() {
            ui.weak("JSON, formatted for reading; Copy copies it as stored.");
        }
    });
    let mut shown = pretty.as_deref().unwrap_or(value);
    ScrollArea::both().auto_shrink(false).show(ui, |ui| {
        // A `&str` buffer makes the editor read-only but still selectable.
        ui.add(
            TextEdit::multiline(&mut shown)
                .code_editor()
                .desired_width(f32::INFINITY),
        );
    });
}

/// `value` indented, if it is a JSON object or array. Scalars are left alone: a number or a
/// quoted word looks the same either way.
fn pretty_json(value: &str) -> Option<String> {
    let trimmed = value.trim_start();
    if !trimmed.starts_with(['{', '[']) {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(value).ok()?;
    serde_json::to_string_pretty(&parsed).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_prints_objects_and_arrays_keeping_key_order() {
        assert_eq!(
            pretty_json(r#"{"b":1,"a":[true,null]}"#).as_deref(),
            Some("{\n  \"b\": 1,\n  \"a\": [\n    true,\n    null\n  ]\n}")
        );
        for not_json in ["42", "\"word\"", "{not json", "plain text"] {
            assert_eq!(pretty_json(not_json), None, "{not_json}");
        }
    }
}
