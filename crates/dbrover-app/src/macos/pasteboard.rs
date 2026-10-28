//! The general pasteboard's text, for Edit › Paste. egui-winit reads it itself when ⌘V reaches
//! the window, but a menu item with ⌘V as its key equivalent takes the key first, and
//! egui-winit's clipboard isn't reachable from the app.

use objc2_app_kit::NSPasteboard;
use objc2_foundation::ns_string;

/// The pasteboard's plain text, if it holds any.
pub fn text() -> Option<String> {
    // `NSPasteboardTypeString`'s value. Reading AppKit's constant itself takes `unsafe`, which
    // the workspace forbids.
    let plain_text = ns_string!("public.utf8-plain-text");
    NSPasteboard::generalPasteboard()
        .stringForType(plain_text)
        .map(|text| text.to_string())
}
