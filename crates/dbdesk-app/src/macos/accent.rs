//! The accent colour the user picked in System Settings › Appearance.

use egui::Color32;
use objc2_app_kit::{NSColor, NSColorSpace};

/// The accent colour in sRGB, as the current appearance shades it. `None` if AppKit can't
/// express it in sRGB.
pub fn read() -> Option<Color32> {
    // A dynamic colour whose colour space AppKit doesn't promise, so it is converted first.
    let accent =
        NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    // Clamped to 0–1 and scaled to 0–255 before the cast, so it always fits in a u8.
    let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(Color32::from_rgb(
        channel(accent.redComponent()),
        channel(accent.greenComponent()),
        channel(accent.blueComponent()),
    ))
}
