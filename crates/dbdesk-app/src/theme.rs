use egui::{Color32, Visuals};

/// egui's visuals have no "success" colour; this green reads well on both light and dark
/// themes. Used for connected data sources and successful connection tests.
pub const SUCCESS_COLOR: Color32 = Color32::from_rgb(70, 170, 90);

/// Colours for SQL syntax highlighting.
pub struct SyntaxColors {
    pub keyword: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub identifier: Color32,
    pub plain: Color32,
}

impl SyntaxColors {
    /// Palettes picked by eye to stay readable on egui's default light and dark backgrounds.
    pub fn for_visuals(visuals: &Visuals) -> Self {
        if visuals.dark_mode {
            Self {
                keyword: Color32::from_rgb(198, 120, 221),
                string: Color32::from_rgb(152, 195, 121),
                number: Color32::from_rgb(209, 154, 102),
                comment: visuals.weak_text_color(),
                identifier: Color32::from_rgb(97, 175, 239),
                plain: visuals.text_color(),
            }
        } else {
            Self {
                keyword: Color32::from_rgb(0, 51, 179),
                string: Color32::from_rgb(6, 125, 23),
                number: Color32::from_rgb(23, 80, 235),
                comment: visuals.weak_text_color(),
                identifier: Color32::from_rgb(135, 16, 148),
                plain: visuals.text_color(),
            }
        }
    }
}
