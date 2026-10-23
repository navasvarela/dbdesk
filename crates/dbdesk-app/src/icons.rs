//! Which icon, in which colour, stands for each driver, catalog object and action.
//!
//! Icons are Phosphor glyphs (see `fonts`), so they scale with the text and take any colour.
//! Colours are palette roles, never fixed values, so every icon is readable in both themes.

use dbdesk_core::{DriverKind, NodeKind, SourceColor};
use egui::{Color32, Image, RichText, Vec2};
use egui_phosphor::regular as glyph;

use crate::fonts;
use crate::theme::Palette;

pub use egui_phosphor::regular::{
    ARROWS_CLOCKWISE, BOOK_OPEN, CARET_DOWN, CARET_RIGHT, CHECK_CIRCLE, CLOCK_COUNTER_CLOCKWISE,
    COPY, EYE, EYE_SLASH, FAST_FORWARD, FILE_SQL, LIST_DASHES, MAGNIFYING_GLASS, MONITOR, MOON,
    PENCIL_SIMPLE, PLAY, PLUGS, PLUGS_CONNECTED, PLUS, PROHIBIT, SPARKLE, STOP, SUN, TABLE,
    TERMINAL_WINDOW, TRASH, TREE_VIEW, X_CIRCLE,
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

/// A database's logo, tinted for the current theme. The SVGs are Simple Icons; see
/// `assets/icons/databases/README.md` for where they come from and how to add one. Needs
/// egui_extras' image loaders, installed at start-up.
pub fn driver_logo(driver: DriverKind, palette: &Palette) -> Image<'static> {
    let (uri, svg): (&str, &'static [u8]) = match driver {
        DriverKind::Postgres => (
            "bytes://logos/postgresql.svg",
            include_bytes!("../../../assets/icons/databases/postgresql.svg"),
        ),
        DriverKind::Sqlite => (
            "bytes://logos/sqlite.svg",
            include_bytes!("../../../assets/icons/databases/sqlite.svg"),
        ),
    };
    Image::from_bytes(uri, svg).tint(palette.driver(driver))
}

/// A database's logo at `size` points square, the size of the text it sits beside.
pub fn driver_logo_sized(driver: DriverKind, palette: &Palette, size: f32) -> Image<'static> {
    driver_logo(driver, palette).fit_to_exact_size(Vec2::splat(size))
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

#[cfg(test)]
mod tests {
    use egui::load::{ImagePoll, SizeHint};

    use super::*;

    #[test]
    fn every_driver_logo_renders_to_a_visible_image() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        for driver in DriverKind::ALL {
            let image = driver_logo(driver, &Palette::LIGHT);
            let source = image.source(&ctx);
            let uri = source.uri().expect("logos are loaded by URI");
            if let egui::ImageSource::Bytes { uri, bytes } = &source {
                ctx.include_bytes(uri.clone(), bytes.clone());
            }
            let poll = ctx
                .try_load_image(
                    uri,
                    SizeHint::Size {
                        width: 32,
                        height: 32,
                        maintain_aspect_ratio: true,
                    },
                )
                .unwrap_or_else(|error| panic!("{driver:?}: {error}"));
            let ImagePoll::Ready { image } = poll else {
                panic!("{driver:?}: the SVG loader answers synchronously");
            };
            let opaque = image.pixels.iter().filter(|pixel| pixel.a() > 200).count();
            assert!(opaque > 50, "{driver:?}: only {opaque} opaque pixels");
        }
    }
}
