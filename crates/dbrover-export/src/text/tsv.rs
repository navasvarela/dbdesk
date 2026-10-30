//! Tab-separated values, as ⌘C copies them: what spreadsheets read when pasted.

use std::io::{self, Write};

use dbrover_core::Cell;

use super::{TextFormat, or_null};
use crate::ExportColumn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsvOptions {
    /// Whether the first line names the columns.
    pub header: bool,
    /// What a `NULL` is written as: by default `NULL`, as the grid shows it.
    pub null_text: String,
}

impl Default for TsvOptions {
    fn default() -> Self {
        Self {
            header: true,
            null_text: "NULL".to_owned(),
        }
    }
}

pub(crate) struct Tsv {
    options: TsvOptions,
}

impl Tsv {
    pub(crate) fn new(options: TsvOptions) -> Self {
        Self { options }
    }
}

impl TextFormat for Tsv {
    fn header(&mut self, out: &mut dyn Write, columns: &[ExportColumn]) -> io::Result<()> {
        if !self.options.header {
            return Ok(());
        }
        line(out, columns.iter().map(|column| column.name.as_str()))
    }

    fn row(&mut self, out: &mut dyn Write, row: &[Cell]) -> io::Result<()> {
        let null_text = &self.options.null_text;
        line(out, row.iter().map(|cell| or_null(cell, null_text)))
    }

    fn footer(&mut self, _out: &mut dyn Write) -> io::Result<()> {
        Ok(())
    }
}

fn line<'a>(out: &mut dyn Write, fields: impl Iterator<Item = &'a str>) -> io::Result<()> {
    for (index, field) in fields.enumerate() {
        if index > 0 {
            out.write_all(b"\t")?;
        }
        write_field(out, field)?;
    }
    out.write_all(b"\n")
}

/// A field with a tab, a line break or a quote is quoted, with quotes doubled, the way
/// spreadsheets read TSV back.
fn write_field(out: &mut dyn Write, field: &str) -> io::Result<()> {
    if field.contains(['\t', '\n', '\r', '"']) {
        write!(out, "\"{}\"", field.replace('"', "\"\""))
    } else {
        out.write_all(field.as_bytes())
    }
}
