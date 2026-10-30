//! The platform's save dialog, for choosing where exported rows, or the query history, go.

use std::path::PathBuf;

use dbrover_export::ExportFormat;

/// Shows the save dialog for exporting `what` ("rows", "history") and returns the chosen file,
/// or `None` if it was cancelled.
///
/// Blocks the UI thread until the dialog closes, unlike everything else in the UI (see
/// ADR-0003): the dialog is modal anyway, and macOS requires it to run on the main thread.
/// The file itself is written by the engine.
pub fn pick_export_path(format: ExportFormat, what: &str) -> Option<PathBuf> {
    let (name, extension) = (format.name(), format.extension());
    rfd::FileDialog::new()
        .set_title(format!("Export {what} as {name}"))
        .add_filter(name, &[extension])
        .set_file_name(format!("{what}.{extension}"))
        .save_file()
}
