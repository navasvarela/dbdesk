//! Writing result rows to a file the user picked, on the blocking pool: the rows a console
//! has loaded, or (in `session::export`) every row of a statement run again.

use std::path::PathBuf;

use dbrover_core::{Row, describe_error};
use dbrover_export::{ExportColumn, ExportError, ExportSpec};
use dbrover_sql::ParameterValues;

use crate::events::EventSink;
use crate::{ConsoleId, Event, Exporter, RunId};

/// Rows to write, and where.
#[derive(Debug)]
pub struct Export {
    pub path: PathBuf,
    pub spec: ExportSpec,
    pub columns: Vec<ExportColumn>,
    pub rows: Vec<Row>,
}

/// A statement whose every row goes to a file, and where.
#[derive(Debug)]
pub struct ExportAll {
    /// Identifies the export for [`crate::Command::Cancel`]; like a run's, chosen by the UI.
    pub run: RunId,
    pub sql: String,
    /// The values of the statement's `:name` parameters, as for [`crate::Command::Execute`].
    pub parameters: ParameterValues,
    pub path: PathBuf,
    pub spec: ExportSpec,
}

fn write_rows(
    path: &std::path::Path,
    spec: &ExportSpec,
    columns: &[ExportColumn],
    rows: &[Row],
) -> Result<(), ExportError> {
    let mut writer = crate::export_file::create_writer(path, spec, columns)?;
    writer.write(rows)?;
    writer.finish()
}

/// Writes the export and reports [`Event::ExportFinished`].
pub(crate) async fn run(console: ConsoleId, export: Export, events: EventSink) {
    let Export {
        path,
        spec,
        columns,
        rows,
    } = export;
    let count = rows.len();
    let task_path = path.clone();
    let written =
        tokio::task::spawn_blocking(move || write_rows(&task_path, &spec, &columns, &rows)).await;
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
        exporter: Exporter::Console(console),
        path,
        result,
    });
}
