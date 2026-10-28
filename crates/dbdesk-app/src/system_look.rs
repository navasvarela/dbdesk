//! The system's own look inside egui, where dbdesk takes it: macOS. Elsewhere [`SystemLook`]
//! has no values, and the bundled faces and brand colours stay.

#[cfg(target_os = "macos")]
pub use crate::macos::look::SystemLook;

#[cfg(not(target_os = "macos"))]
pub enum SystemLook {}

#[cfg(not(target_os = "macos"))]
impl SystemLook {
    pub fn follow(&mut self, _: &egui::Context) {
        match *self {}
    }
}
