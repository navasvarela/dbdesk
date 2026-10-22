use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use crate::{Event, Repaint};

/// Sends events to the UI and wakes it up after each one. Cheap to clone into tasks.
#[derive(Clone)]
pub(crate) struct EventSink {
    events: UnboundedSender<Event>,
    repaint: Arc<dyn Repaint>,
}

impl EventSink {
    pub(crate) fn new(events: UnboundedSender<Event>, repaint: impl Repaint) -> Self {
        Self {
            events,
            repaint: Arc::new(repaint),
        }
    }

    pub(crate) fn send(&self, event: Event) {
        // Sending only fails once the UI has dropped its receiver, which happens while the
        // app shuts down; there is nobody left to tell.
        if self.events.send(event).is_ok() {
            self.repaint.request_repaint();
        }
    }
}
