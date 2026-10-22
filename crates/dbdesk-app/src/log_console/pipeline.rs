use std::fmt::{Debug, Display};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::thread::{self, ThreadId};

use jiff::Zoned;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use super::LogEntry;

/// Creates both ends of the log pipeline: a `tracing` layer that captures events, and the
/// feed the [`LogConsole`](super::LogConsole) drains.
///
/// Must be called on the thread that will run the UI; see [`UiWaker`] for why that matters.
///
/// The channel is unbounded on purpose. The UI drains it every frame, including while the
/// window is hidden (eframe still calls `App::logic` then), so it only grows if the UI thread
/// is stuck, and at that point the app is broken anyway.
pub fn pipeline() -> (LogConsoleLayer, LogFeed) {
    let (sender, receiver) = mpsc::channel();
    let waker = Arc::new(UiWaker {
        ui_thread: thread::current().id(),
        ctx: OnceLock::new(),
    });
    let layer = LogConsoleLayer {
        entries: sender,
        waker: Arc::clone(&waker),
    };
    let feed = LogFeed {
        entries: receiver,
        waker,
    };
    (layer, feed)
}

/// The receiving end of the log pipeline.
pub struct LogFeed {
    pub(super) entries: Receiver<LogEntry>,
    pub(super) waker: Arc<UiWaker>,
}

/// Wakes the UI when a log entry arrives from another thread.
///
/// `egui::Context::request_repaint` takes egui's context lock, which is not reentrant. If a
/// `tracing` event fired on the UI thread while that lock is held (for example from inside a
/// `ctx.input(|i| ...)` closure, or a `log` record from egui itself) and this waker called
/// `request_repaint`, the UI would deadlock. So the waker ignores events from the UI thread.
/// Those entries are picked up by the end-of-frame drain in `DbDeskApp::ui`, which requests the
/// repaint itself, outside any lock.
pub(super) struct UiWaker {
    ui_thread: ThreadId,
    /// Unset until the egui context exists; entries logged before that wait in the channel
    /// and are drained on the first frame.
    ctx: OnceLock<egui::Context>,
}

impl UiWaker {
    /// Connects the waker to the UI. Only the first call has an effect; there is one egui
    /// context per app.
    pub(super) fn install(&self, ctx: &egui::Context) {
        self.ctx.get_or_init(|| ctx.clone());
    }

    fn wake(&self) {
        if thread::current().id() == self.ui_thread {
            return;
        }
        if let Some(ctx) = self.ctx.get() {
            ctx.request_repaint();
        }
    }
}

/// A `tracing` layer that forwards every event it sees to the log console.
pub struct LogConsoleLayer {
    entries: Sender<LogEntry>,
    waker: Arc<UiWaker>,
}

impl<S: Subscriber> Layer<S> for LogConsoleLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut fields = FieldCollector::default();
        event.record(&mut fields);

        let entry = LogEntry {
            time: Zoned::now(),
            level: *metadata.level(),
            target: metadata.target().to_owned(),
            message: fields.message,
            fields: fields.others,
        };
        // Sending only fails once the UI has dropped the feed, i.e. while the app shuts down
        // and there is nobody left to show the entry to.
        if self.entries.send(entry).is_ok() {
            self.waker.wake();
        }
    }
}

#[derive(Default)]
struct FieldCollector {
    message: String,
    others: String,
}

impl FieldCollector {
    fn collect(&mut self, field: &Field, value: impl Display) {
        if field.name() == "message" {
            self.message = value.to_string();
            return;
        }
        if !self.others.is_empty() {
            self.others.push(' ');
        }
        self.others.push_str(&format!("{}={value}", field.name()));
    }
}

impl Visit for FieldCollector {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.collect(field, value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.collect(field, format_args!("{value:?}"));
    }
}

#[cfg(test)]
mod tests {
    use tracing::Level;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;

    #[test]
    fn captures_level_target_message_and_fields() {
        let (layer, feed) = pipeline();
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(rows = 3, table = "users", "query finished");
        });

        let entry = feed.entries.try_recv().unwrap();
        assert_eq!(entry.level, Level::WARN);
        assert_eq!(entry.target, module_path!());
        assert_eq!(entry.message, "query finished");
        assert_eq!(entry.fields, "rows=3 table=users");
        assert!(feed.entries.try_recv().is_err());
    }
}
