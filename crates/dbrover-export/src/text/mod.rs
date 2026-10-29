//! The formats that write text: a header, then a line or block per row, then a footer.

mod csv;
mod html;
mod json;
mod markdown;
mod tsv;

use std::io::{self, Write};

use dbrover_core::Cell;

pub(crate) use self::csv::Csv;
pub use self::csv::{CsvOptions, FieldSeparator, InvalidSeparator, LineEnding, Quoting};
pub(crate) use self::html::Html;
pub use self::html::{HtmlDocument, HtmlOptions};
pub(crate) use self::json::{Json, JsonLines};
pub(crate) use self::markdown::Markdown;
pub use self::markdown::MarkdownOptions;
pub(crate) use self::tsv::Tsv;
pub use self::tsv::TsvOptions;
use crate::{ExportColumn, ExportSpec};

/// A text format, written a row at a time so any number of rows streams through it.
///
/// [`TextFormat::header`] is called once, first, then [`TextFormat::row`] for each row, then
/// [`TextFormat::footer`] once. A format keeps whatever it needs between calls (the column
/// names, whether a separator is due) and writes what it still owes in the footer.
pub trait TextFormat: Send {
    /// Writes what comes before the rows. Each row that follows has a cell per column.
    ///
    /// # Errors
    ///
    /// What `out` reports.
    fn header(&mut self, out: &mut dyn Write, columns: &[ExportColumn]) -> io::Result<()>;

    /// Writes one row.
    ///
    /// # Errors
    ///
    /// What `out` reports.
    fn row(&mut self, out: &mut dyn Write, row: &[Cell]) -> io::Result<()>;

    /// Writes what comes after the rows.
    ///
    /// # Errors
    ///
    /// What `out` reports.
    fn footer(&mut self, out: &mut dyn Write) -> io::Result<()>;
}

/// `rows` under a header of `columns`, as text: what goes on the clipboard, on a program's
/// stdin or into an MCP answer.
pub fn render(spec: &ExportSpec, columns: &[ExportColumn], rows: &[Vec<Cell>]) -> String {
    let mut out = Vec::new();
    // Writing into a `Vec` can't fail, and nor can serializing strings and nulls as JSON, the
    // only other thing a format does that returns an error.
    let _ = write_all(spec.text_format().as_mut(), &mut out, columns, rows);
    String::from_utf8_lossy(&out).into_owned()
}

fn write_all(
    format: &mut dyn TextFormat,
    out: &mut dyn Write,
    columns: &[ExportColumn],
    rows: &[Vec<Cell>],
) -> io::Result<()> {
    format.header(out, columns)?;
    for row in rows {
        format.row(out, row)?;
    }
    format.footer(out)
}

/// `value`, or `null_text` for `NULL`.
fn or_null<'a>(cell: &'a Cell, null_text: &'a str) -> &'a str {
    cell.as_deref().unwrap_or(null_text)
}
