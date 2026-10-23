use std::path::Path;

use anyhow::Context as _;
use tracing::Level;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{Layer as _, fmt};

use crate::log_console::{self, LogFeed};

/// Daily log files kept before the oldest is deleted.
const MAX_LOG_FILES: usize = 7;

pub struct Telemetry {
    pub log_feed: LogFeed,
    /// Flushes the file log when dropped, so it must live until the app exits.
    /// `None` when file logging is unavailable; the reason is in the log console.
    pub file_guard: Option<WorkerGuard>,
}

/// Installs the global `tracing` subscriber: everything goes to the log console and, when the
/// logs directory is usable, to a daily rolling file. Records from the `log` crate (used by
/// eframe and winit) are bridged in too.
///
/// Must be called on the thread that will run the UI (see `log_console::pipeline`).
pub fn init(logs_dir: &Path) -> anyhow::Result<Telemetry> {
    let (console_layer, log_feed) = log_console::pipeline();
    let (file_writer, file_guard) = match open_log_files(logs_dir) {
        Ok((writer, guard)) => (Some(writer), Ok(guard)),
        Err(error) => (None, Err(error)),
    };
    let file_layer = file_writer.map(|writer| {
        fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .with_filter(targets())
    });

    tracing_subscriber::registry()
        .with(console_layer.with_filter(targets()))
        .with(file_layer)
        .try_init()
        .context("could not install the tracing subscriber")?;

    let file_guard = file_guard
        .inspect_err(|error| tracing::warn!("file logging disabled: {error:#}"))
        .ok();
    Ok(Telemetry {
        log_feed,
        file_guard,
    })
}

/// dbdesk's own crates log from debug up. Dependencies only from warn up: at info they are
/// chatty about things a database tool's user doesn't care about (GL setup, window events).
pub fn targets() -> Targets {
    Targets::new()
        .with_target("dbdesk", Level::DEBUG)
        .with_default(Level::WARN)
}

fn open_log_files(logs_dir: &Path) -> anyhow::Result<(NonBlocking, WorkerGuard)> {
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("dbdesk")
        .filename_suffix("log")
        .max_log_files(MAX_LOG_FILES)
        .build(logs_dir)
        .with_context(|| format!("could not open log files in {}", logs_dir.display()))?;
    Ok(tracing_appender::non_blocking(appender))
}
