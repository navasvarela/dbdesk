//! Waking the app when a pause measured on the wall clock ends: typing settling before the
//! explorer's filter reaches the server, a console's edits settling before they are saved.

use std::time::Duration;

/// How much longer than one predicted frame the shortest wait asks for. Any length above zero
/// keeps the request a delayed one (see [`request_repaint_after_pause`]); a millisecond is short
/// enough not to show.
const PAST_THE_NEXT_FRAME: Duration = Duration::from_millis(1);

/// Asks for a frame once `left` of a pause has passed.
///
/// egui takes a frame's predicted length off every delay, and treats a request for less than a
/// frame as one to repaint at once. On screen that is the same thing, since the next frame comes
/// within milliseconds. The UI tests, though, step frames of a quarter of a second while barely
/// any real time passes, so for the last quarter second of a pause every frame asked for another
/// at once, and `Harness::run` gave up after its few steps. Waiting at least one frame keeps the
/// request delayed: on screen the frame still comes next, and in the tests `run` returns while
/// the test waits for the pause as it waits for the engine, with `step_until`.
pub fn request_repaint_after_pause(ctx: &egui::Context, left: Duration) {
    let frame =
        Duration::try_from_secs_f32(ctx.input(|input| input.predicted_dt)).unwrap_or_default();
    ctx.request_repaint_after(left.max(frame + PAST_THE_NEXT_FRAME));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The delay egui reports once a UI that asks every frame to wake after `left` has run,
    /// stepped as the UI tests step it, a quarter of a second a frame. `Harness::run` panics
    /// if the request reads as one to repaint at once.
    fn reported_delay(left: Duration) -> Duration {
        let mut harness = egui_kittest::Harness::new_ui(move |ui| {
            request_repaint_after_pause(ui.ctx(), left);
        });
        harness.run();
        harness.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay
    }

    #[test]
    fn the_end_of_a_pause_stays_a_delayed_repaint() {
        for left in [
            Duration::ZERO,
            Duration::from_millis(100),
            Duration::from_millis(250),
        ] {
            assert!(reported_delay(left) > Duration::ZERO, "{left:?}");
        }
    }

    #[test]
    fn a_pause_longer_than_a_frame_wakes_when_it_ends() {
        assert_eq!(
            reported_delay(Duration::from_millis(300)),
            Duration::from_millis(50)
        );
    }
}
