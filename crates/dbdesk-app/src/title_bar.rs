//! The strip along the top of a window whose content runs under a transparent title bar (macOS,
//! D5c in `docs/macos-look.md`): it leaves room for the window buttons, shows the title, moves
//! the window when dragged and zooms it when double-clicked, as the title bar did.

use egui::{Align, Label, Layout, Rect, RichText, Sense, Ui, UiBuilder, ViewportCommand, vec2};

/// As tall as a macOS title bar, so the window buttons sit centred in the strip.
pub const HEIGHT: f32 = 28.0;
/// The width of the close, minimize and zoom buttons at the strip's left, with their margins.
pub const WINDOW_BUTTONS_WIDTH: f32 = 78.0;

/// Draws the strip in `ui`, then `contents` (the egui menu bar, when there is no native one)
/// after the window buttons.
pub fn show(ui: &mut Ui, title: &str, contents: impl FnOnce(&mut Ui)) {
    let strip = ui.max_rect();
    // Registered before anything drawn on the strip, so the menu's buttons, drawn on top, still
    // get their own clicks.
    let background = ui.interact(strip, ui.id().with("title_bar"), Sense::click_and_drag());
    if background.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if background.double_clicked() {
        let maximized = ui.input(|input| input.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }

    let menu_rect = strip.with_min_x(strip.left() + window_buttons_inset(ui));
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(menu_rect)
            .layout(Layout::left_to_right(Align::Center)),
        contents,
    );
    // Over the middle third, clear of the menu at the left.
    let title_rect = Rect::from_center_size(strip.center(), vec2(strip.width() / 3.0, HEIGHT));
    ui.put(
        title_rect,
        Label::new(RichText::new(title).color(ui.visuals().weak_text_color()))
            .selectable(false)
            .truncate(),
    );
}

/// In full screen the window buttons hide, and so does the room left for them.
fn window_buttons_inset(ui: &Ui) -> f32 {
    let full_screen = ui.input(|input| input.viewport().fullscreen.unwrap_or(false));
    if full_screen {
        0.0
    } else {
        WINDOW_BUTTONS_WIDTH
    }
}
