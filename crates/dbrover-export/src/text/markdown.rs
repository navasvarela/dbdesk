//! A GitHub-flavoured Markdown table, for pull requests, issues and chat.

use std::io::{self, Write};

use dbrover_core::Cell;

use super::{TextFormat, or_null};
use crate::ExportColumn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownOptions {
    /// What a `NULL` is shown as: by default `NULL`, as the grid shows it.
    pub null_text: String,
}

impl Default for MarkdownOptions {
    fn default() -> Self {
        Self {
            null_text: "NULL".to_owned(),
        }
    }
}

pub(crate) struct Markdown {
    options: MarkdownOptions,
    /// A table needs a column; without one there is nothing to write, rows included.
    has_columns: bool,
}

impl Markdown {
    pub(crate) fn new(options: MarkdownOptions) -> Self {
        Self {
            options,
            has_columns: false,
        }
    }
}

impl TextFormat for Markdown {
    fn header(&mut self, out: &mut dyn Write, columns: &[ExportColumn]) -> io::Result<()> {
        self.has_columns = !columns.is_empty();
        if !self.has_columns {
            return Ok(());
        }
        line(out, columns.iter().map(|column| column.name.as_str()))?;
        line(out, columns.iter().map(|_| "---"))
    }

    fn row(&mut self, out: &mut dyn Write, row: &[Cell]) -> io::Result<()> {
        if !self.has_columns {
            return Ok(());
        }
        let null_text = &self.options.null_text;
        line(out, row.iter().map(|cell| or_null(cell, null_text)))
    }

    fn footer(&mut self, _out: &mut dyn Write) -> io::Result<()> {
        Ok(())
    }
}

fn line<'a>(out: &mut dyn Write, cells: impl Iterator<Item = &'a str>) -> io::Result<()> {
    out.write_all(b"|")?;
    for cell in cells {
        write!(out, " {} |", escape(cell))?;
    }
    out.write_all(b"\n")
}

/// A value as a table cell: a `|` would end the cell and a line break the row, so the one is
/// escaped and the other written as `<br>`, which GitHub renders as a break.
fn escape(value: &str) -> String {
    value
        .replace('|', "\\|")
        .replace("\r\n", "<br>")
        .replace(['\r', '\n'], "<br>")
}
