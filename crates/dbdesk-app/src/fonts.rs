//! The bundled typefaces: Inter for the interface, JetBrains Mono for SQL and results, and
//! Phosphor for icons. All three are embedded, so the app looks the same on every machine.
//! Licences are next to the font files in `assets/fonts/`.
//!
//! Icons have a family of their own, [`icon`], rather than being a fallback of the text
//! families as egui-phosphor sets them up. Inter has its own glyphs at hundreds of the
//! private-use code points Phosphor uses, so as a fallback behind Inter many icons would come
//! out as stray accented letters. Phosphor can't go first either: it maps a–z to the blank
//! glyphs its ligatures are built from.

use egui::{FontData, FontDefinitions, FontFamily, FontId};

const INTER: &str = "Inter";
const INTER_SEMIBOLD: &str = "Inter SemiBold";
const JETBRAINS_MONO: &str = "JetBrains Mono";
const PHOSPHOR: &str = "Phosphor";

/// Text sizes in points.
const BODY_SIZE: f32 = 13.0;
const SMALL_SIZE: f32 = 11.0;
const HEADING_SIZE: f32 = 20.0;
const MONOSPACE_SIZE: f32 = 12.5;

/// The icon font. Text in it that isn't an icon falls back to Inter.
pub fn icon(size: f32) -> FontId {
    FontId::new(size, icon_family())
}

pub fn icon_family() -> FontFamily {
    FontFamily::Name(PHOSPHOR.into())
}

/// A semibold Inter face, for panel titles and headings. egui has no font weights, so it is
/// a family of its own.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(INTER_SEMIBOLD.into()))
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
    ctx.all_styles_mut(|style| {
        use egui::TextStyle;
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(SMALL_SIZE)),
            (TextStyle::Body, FontId::proportional(BODY_SIZE)),
            (TextStyle::Button, FontId::proportional(BODY_SIZE)),
            (TextStyle::Heading, semibold(HEADING_SIZE)),
            (TextStyle::Monospace, FontId::monospace(MONOSPACE_SIZE)),
        ]
        .into();
    });
}

fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (
            INTER,
            &include_bytes!("../../../assets/fonts/inter/Inter-Regular.ttf")[..],
        ),
        (
            INTER_SEMIBOLD,
            &include_bytes!("../../../assets/fonts/inter/Inter-SemiBold.ttf")[..],
        ),
        (
            JETBRAINS_MONO,
            &include_bytes!("../../../assets/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf")[..],
        ),
        (PHOSPHOR, egui_phosphor::Variant::Regular.font_bytes()),
    ] {
        fonts
            .font_data
            .insert(name.to_owned(), FontData::from_static(bytes).into());
    }
    // egui's own fonts stay behind ours as fallbacks, for symbols and emoji ours lack.
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, INTER.to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, JETBRAINS_MONO.to_owned());
    let proportional = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut semibold = vec![INTER_SEMIBOLD.to_owned()];
    semibold.extend(proportional.into_iter().filter(|name| name != INTER));
    fonts
        .families
        .insert(FontFamily::Name(INTER_SEMIBOLD.into()), semibold);
    fonts
        .families
        .insert(icon_family(), vec![PHOSPHOR.to_owned(), INTER.to_owned()]);
    fonts
}
