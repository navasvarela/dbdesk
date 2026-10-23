//! What the app shows of each database engine: its logo and the logo's colours. The one
//! place the app looks these up per engine, so a new engine adds a row here and its SVG.

use dbdesk_core::DriverKind;
use egui::Color32;

/// A database's logo and the colours it is tinted with.
pub struct DatabaseBrand {
    /// The image loader's cache key for the logo.
    pub logo_uri: &'static str,
    /// A Simple Icons SVG; see `assets/icons/databases/README.md` for where each comes from
    /// and how to add one.
    pub logo_svg: &'static [u8],
    /// The brand's own colour, from Simple Icons, for the light theme.
    pub light: Color32,
    /// A lighter shade of the same hue for the dark theme, where the brand colour would be
    /// too dark to see.
    pub dark: Color32,
}

pub fn of(driver: DriverKind) -> DatabaseBrand {
    match driver {
        DriverKind::Postgres => DatabaseBrand {
            logo_uri: "bytes://logos/postgresql.svg",
            logo_svg: include_bytes!("../../../assets/icons/databases/postgresql.svg"),
            light: Color32::from_rgb(0x41, 0x69, 0xE1),
            dark: Color32::from_rgb(0x7F, 0xA2, 0xF5),
        },
        DriverKind::Sqlite => DatabaseBrand {
            logo_uri: "bytes://logos/sqlite.svg",
            logo_svg: include_bytes!("../../../assets/icons/databases/sqlite.svg"),
            light: Color32::from_rgb(0x00, 0x3B, 0x57),
            dark: Color32::from_rgb(0x5A, 0xB0, 0xE0),
        },
    }
}
