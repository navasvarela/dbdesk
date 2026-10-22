use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{Command, Event, Repaint};

/// Sends events to the UI and wakes it up after each one.
pub(crate) struct EventSink {
    events: UnboundedSender<Event>,
    repaint: Box<dyn Repaint>,
}

/// The UI dropped its event receiver, so nobody is listening any more.
pub(crate) struct UiGone;

impl EventSink {
    pub(crate) fn new(events: UnboundedSender<Event>, repaint: impl Repaint) -> Self {
        Self {
            events,
            repaint: Box::new(repaint),
        }
    }

    pub(crate) fn send(&self, event: Event) -> Result<(), UiGone> {
        self.events.send(event).map_err(|_| UiGone)?;
        self.repaint.request_repaint();
        Ok(())
    }
}

/// Handles commands until the UI closes the command channel.
pub(crate) async fn run(mut commands: UnboundedReceiver<Command>, events: EventSink) {
    tracing::info!("engine started");
    if events.send(Event::Started).is_err() {
        tracing::warn!("UI stopped listening before the engine started");
        return;
    }

    while let Some(command) = commands.recv().await {
        handle(command);
    }
    tracing::info!("engine stopped");
}

fn handle(command: Command) {
    match command {}
}
