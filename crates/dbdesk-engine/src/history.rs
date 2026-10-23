//! The query history database, owned by one thread from the blocking pool. Requests queue
//! up in order, so a search sent after a run has finished sees that run's statements.

use dbdesk_core::{DataSourceId, describe_error};
use dbdesk_store::{History, HistoryEntry, HistoryFile};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use crate::Event;
use crate::events::EventSink;

/// The most entries a search returns. Plenty to scroll through; narrowing the search finds
/// anything older.
const SEARCH_LIMIT: usize = 500;

enum HistoryRequest {
    Record(HistoryEntry),
    Search {
        data_source: DataSourceId,
        text: String,
    },
}

/// A handle for queueing history work. Cheap to clone into sessions.
#[derive(Clone)]
pub(crate) struct HistoryLog {
    requests: UnboundedSender<HistoryRequest>,
}

impl HistoryLog {
    /// Starts the history thread. It runs until every handle is dropped; await the returned
    /// task to know that queued entries were written.
    pub(crate) fn start(file: HistoryFile, events: EventSink) -> (Self, JoinHandle<()>) {
        let (requests, receiver) = mpsc::unbounded_channel();
        let task = tokio::task::spawn_blocking(move || serve(&file, receiver, &events));
        (Self { requests }, task)
    }

    pub(crate) fn record(&self, entry: HistoryEntry) {
        self.send(HistoryRequest::Record(entry));
    }

    /// Answered by [`Event::HistoryFound`].
    pub(crate) fn search(&self, data_source: DataSourceId, text: String) {
        self.send(HistoryRequest::Search { data_source, text });
    }

    fn send(&self, request: HistoryRequest) {
        // Fails only once the thread has stopped, which happens during shutdown.
        if self.requests.send(request).is_err() {
            tracing::debug!("dropped a history request after the history stopped");
        }
    }
}

fn serve(file: &HistoryFile, mut requests: UnboundedReceiver<HistoryRequest>, events: &EventSink) {
    let history = file.open().map_err(|error| {
        let error = describe_error(&error);
        tracing::error!("query history is unavailable: {error}");
        error
    });
    while let Some(request) = requests.blocking_recv() {
        match request {
            HistoryRequest::Record(entry) => {
                if let Ok(history) = &history {
                    record(history, &entry);
                }
            }
            HistoryRequest::Search { data_source, text } => {
                let result = match &history {
                    Ok(history) => history
                        .search(data_source, &text, SEARCH_LIMIT)
                        .map_err(|error| describe_error(&error)),
                    Err(error) => Err(error.clone()),
                };
                events.send(Event::HistoryFound {
                    data_source,
                    text,
                    result,
                });
            }
        }
    }
}

/// Losing one history entry isn't worth interrupting anyone for, so it is only logged.
fn record(history: &History, entry: &HistoryEntry) {
    if let Err(error) = history.record(entry) {
        tracing::warn!(
            "could not record a statement in the history: {}",
            describe_error(&error)
        );
    }
}
