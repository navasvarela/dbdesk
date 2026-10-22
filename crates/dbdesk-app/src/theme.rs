use egui::Color32;

/// egui's visuals have no "success" colour; this green reads well on both light and dark
/// themes. Used for connected data sources and successful connection tests.
pub const SUCCESS_COLOR: Color32 = Color32::from_rgb(70, 170, 90);
