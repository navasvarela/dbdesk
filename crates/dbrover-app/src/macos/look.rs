//! macOS's look inside egui: the system's faces, and selection and focus in the user's accent
//! colour, followed when it changes (D5d in `docs/macos-look.md`).

use egui::{Color32, Theme};

use super::accent;
use crate::{fonts, theme};

pub struct SystemLook {
    accent: Option<Color32>,
    theme: Theme,
    focused: bool,
}

impl SystemLook {
    /// Replaces the bundled faces and the brand's selection colour with the system's.
    pub fn install(ctx: &egui::Context) -> Self {
        fonts::install_system(ctx);
        let accent = accent::read();
        theme::install_with_accent(ctx, accent);
        Self {
            accent,
            theme: ctx.theme(),
            focused: true,
        }
    }

    /// Reads the accent colour again whenever the user may have changed it: when the window
    /// comes back to the front (after a visit to System Settings), or when the theme changes,
    /// since macOS shades the accent per appearance. AppKit also posts a notification, but
    /// observing it takes `unsafe` code, which the workspace forbids.
    pub fn follow(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|input| input.viewport().focused.unwrap_or(true));
        let came_to_front = focused && !self.focused;
        self.focused = focused;
        let theme = ctx.theme();
        if !came_to_front && theme == self.theme {
            return;
        }
        self.theme = theme;
        let accent = accent::read();
        if accent != self.accent {
            self.accent = accent;
            theme::install_with_accent(ctx, accent);
        }
    }
}
