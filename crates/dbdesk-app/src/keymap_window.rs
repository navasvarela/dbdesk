//! View → Keymap: every action's id and shortcuts, what `keymap.toml` got wrong, and buttons
//! to create and reload the file.

use std::path::Path;

use egui::{Grid, RichText, ScrollArea, Window};

use crate::keymap::{Action, Keymap};

const DEFAULT_SIZE: [f32; 2] = [620.0, 380.0];

pub enum KeymapWindowAction {
    Reload,
    /// Write a starting `keymap.toml`, if there is none.
    Create,
    Close,
}

/// Draws the window. `path` is where the keymap lives, once the engine has said.
pub fn show(
    ctx: &egui::Context,
    keymap: &Keymap,
    path: Option<&Path>,
) -> Option<KeymapWindowAction> {
    let mut open = true;
    let action = Window::new("Keymap")
        .id(egui::Id::new("keymap_window"))
        .open(&mut open)
        .default_size(DEFAULT_SIZE)
        .show(ctx, |ui| contents(ui, keymap, path))
        .and_then(|response| response.inner.flatten());
    if open {
        action
    } else {
        Some(KeymapWindowAction::Close)
    }
}

fn contents(ui: &mut egui::Ui, keymap: &Keymap, path: Option<&Path>) -> Option<KeymapWindowAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        if let Some(path) = path {
            ui.weak(path.display().to_string());
        }
        if ui.button("Reload").clicked() {
            action = Some(KeymapWindowAction::Reload);
        }
        if ui
            .button("Create keymap.toml")
            .on_hover_text("Write a file listing every action, to edit. An existing file is kept.")
            .clicked()
        {
            action = Some(KeymapWindowAction::Create);
        }
    });
    for problem in keymap.problems() {
        ui.colored_label(ui.visuals().error_fg_color, problem);
    }
    ui.separator();
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        Grid::new("keymap_bindings").striped(true).show(ui, |ui| {
            for action in Action::ALL {
                ui.label(RichText::new(action.id()).monospace());
                let shortcuts: Vec<String> = keymap
                    .bindings(action)
                    .iter()
                    .map(|shortcut| ui.ctx().format_shortcut(shortcut))
                    .collect();
                if shortcuts.is_empty() {
                    ui.weak("unbound");
                } else {
                    ui.label(shortcuts.join(", "));
                }
                ui.weak(action.description());
                ui.end_row();
            }
        });
    });
    action
}
