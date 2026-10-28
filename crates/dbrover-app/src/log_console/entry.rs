use jiff::Zoned;
use tracing::Level;

/// One line in the log console, captured from a `tracing` event.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub time: Zoned,
    pub level: Level,
    /// The event's target, by default the module path that emitted it.
    pub target: String,
    pub message: String,
    /// Structured fields other than the message, rendered as space-separated `key=value` pairs.
    pub fields: String,
}
