/// Wakes the UI so it drains newly sent [`Event`](crate::Event)s.
///
/// An idle egui app only redraws on input, so without this an event would sit in the channel
/// until the user moved the mouse. The trait keeps this crate independent of egui: the app
/// implements it on top of `egui::Context::request_repaint`.
pub trait Repaint: Send + Sync + 'static {
    fn request_repaint(&self);
}
