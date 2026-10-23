//! The dbdesk mark, embedded for the window icon and the Welcome tab. The sources and the
//! script that renders them live in `assets/brand/`.

/// The app icon at 256 px: large enough for a sharp macOS Dock icon at 2x, small enough that
/// decoding it at start-up costs nothing noticeable.
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../../../assets/brand/icon-256.png");

/// The bare mark at 256 px, sharp at the Welcome tab's size on 2x screens.
const LOGO_PNG: &[u8] = include_bytes!("../../../assets/brand/logo-256.png");

/// The mark as an image widget. Needs egui_extras' image loaders, installed at start-up.
pub fn logo() -> egui::Image<'static> {
    egui::Image::from_bytes("bytes://dbdesk-logo.png", LOGO_PNG)
}

/// The window and Dock icon, or `None` if the embedded PNG doesn't decode, in which case the
/// window falls back to the platform's default icon rather than refusing to start.
pub fn window_icon() -> Option<egui::IconData> {
    eframe::icon_data::from_png_bytes(WINDOW_ICON_PNG)
        .inspect_err(|error| tracing::warn!(%error, "the embedded window icon does not decode"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_icon_decodes_to_a_square_rgba_image() {
        let icon = window_icon().expect("the embedded icon is a valid PNG");
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }
}
