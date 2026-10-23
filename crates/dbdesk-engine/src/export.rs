//! Writing result rows to a file the user picked, on the blocking pool.

use std::path::PathBuf;

use dbdesk_core::{Row, describe_error};
use dbdesk_store::{ExportFormat, export_rows};

use crate::events::EventSink;
use crate::{ConsoleId, Event};

/// Rows to write, and where.
#[derive(Debug)]
pub struct Export {
    pub path: PathBuf,
    pub format: ExportFormat,
    pub columns: Vec<String>,
    pub rows: Vec<Row>,
}

/// Writes the export and reports [`Event::ExportFinished`].
pub(crate) async fn run(console: ConsoleId, export: Export, events: EventSink) {
    let Export {
        path,
        format,
        columns,
        rows,
    } = export;
    let count = rows.len();
    let task_path = path.clone();
    let written =
        tokio::task::spawn_blocking(move || export_rows(&task_path, format, &columns, &rows)).await;
    let result = match written {
        Ok(Ok(())) => {
            tracing::info!("exported {count} rows to {}", path.display());
            Ok(count)
        }
        Ok(Err(error)) => Err(describe_error(&error)),
        Err(error) => Err(describe_error(&error)),
    };
    if let Err(error) = &result {
        tracing::error!("export failed: {error}");
    }
    events.send(Event::ExportFinished {
        console,
        path,
        result,
    });
}
