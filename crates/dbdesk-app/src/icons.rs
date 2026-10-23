//! Which icon, in which colour, stands for each driver, catalog object and action.
//!
//! Icons are Phosphor glyphs (see `fonts`), so they scale with the text and take any colour.
//! Colours are palette roles, never fixed values, so every icon is readable in both themes.

use dbdesk_core::{DriverKind, NodeKind, SourceColor};
use egui::{Color32, RichText};
use egui_phosphor::regular as glyph;

use crate::fonts;
use crate::theme::Palette;

pub use egui_phosphor::regular::{
    ARROWS_CLOCKWISE, BOOK_OPEN, CARET_DOWN, CARET_RIGHT, CHECK_CIRCLE, COPY, EYE, EYE_SLASH,
    FAST_FORWARD, FILE_SQL, LIST_DASHES, MAGNIFYING_GLASS, MONITOR, MOON, PENCIL_SIMPLE, PLAY,
    PLUGS, PLUGS_CONNECTED, PLUS, PROHIBIT, SPARKLE, STOP, SUN, TABLE, TERMINAL_WINDOW, TRASH,
    X_CIRCLE,
};

/// A glyph and the palette role that colours it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icon {
    pub glyph: &'static str,
    pub role: Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Primary,
    Secondary,
    Success,
    Warning,
    Error,
    Weak,
    /// One of the data source colours, for icons that need a hue the palette's roles lack.
    Source(SourceColor),
}

impl Role {
    pub fn color(self, palette: &Palette) -> Color32 {
        match self {
            Self::Primary => palette.primary,
            Self::Secondary => palette.secondary,
            Self::Success => palette.success,
            Self::Warning => palette.warning,
            Self::Error => palette.error,
            Self::Weak => palette.text_weak,
            Self::Source(color) => palette.source(color),
        }
    }
}

impl Icon {
    const fn new(glyph: &'static str, role: Role) -> Self {
        Self { glyph, role }
    }

    pub fn rich_text(self, palette: &Palette) -> RichText {
        RichText::new(self.glyph)
            .family(fonts::icon_family())
            .color(self.role.color(palette))
    }
}

pub fn driver(driver: DriverKind) -> Icon {
    match driver {
        DriverKind::Postgres => Icon::new(glyph::DATABASE, Role::Source(SourceColor::Blue)),
        DriverKind::Sqlite => Icon::new(glyph::FEATHER, Role::Source(SourceColor::Teal)),
    }
}

pub fn node(kind: NodeKind) -> Icon {
    match kind {
        NodeKind::Database => Icon::new(glyph::DATABASE, Role::Secondary),
        NodeKind::Schema => Icon::new(glyph::TREE_STRUCTURE, Role::Secondary),
        NodeKind::Folder(_) => Icon::new(glyph::FOLDER_SIMPLE, Role::Warning),
        NodeKind::Table => Icon::new(glyph::TABLE, Role::Primary),
        NodeKind::View => Icon::new(glyph::EYE, Role::Primary),
        NodeKind::MaterializedView => Icon::new(glyph::STACK, Role::Primary),
        NodeKind::Function => Icon::new(glyph::FUNCTION, Role::Success),
        NodeKind::Sequence => Icon::new(glyph::LIST_NUMBERS, Role::Success),
        NodeKind::Column => Icon::new(glyph::TEXT_COLUMNS, Role::Weak),
        NodeKind::Index => Icon::new(glyph::LIST_MAGNIFYING_GLASS, Role::Warning),
        NodeKind::Constraint => Icon::new(glyph::KEY, Role::Warning),
        NodeKind::Trigger => Icon::new(glyph::LIGHTNING, Role::Error),
    }
}
