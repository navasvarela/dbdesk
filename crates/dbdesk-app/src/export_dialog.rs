//! The platform's save dialog, for choosing where exported rows go.

use std::path::PathBuf;

use dbdesk_store::ExportFormat;

/// Shows the save dialog and returns the chosen file, or `None` if it was cancelled.
///
/// Blocks the UI thread until the dialog closes, unlike everything else in the UI (see
/// ADR-0003): the dialog is modal anyway, and macOS requires it to run on the main thread.
/// The file itself is written by the engine.
pub fn pick_export_path(format: ExportFormat) -> Option<PathBuf> {
    let (name, extension) = match format {
        ExportFormat::Csv => ("CSV", format.extension()),
        ExportFormat::Json => ("JSON", format.extension()),
    };
    rfd::FileDialog::new()
        .set_title(format!("Export rows as {name}"))
        .add_filter(name, &[extension])
        .set_file_name(format!("rows.{extension}"))
        .save_file()
}
