//! An HTML table: a fragment to paste into an email or a page, or a page of its own.

use std::io::{self, Write};

use dbrover_core::Cell;

use super::TextFormat;
use crate::ExportColumn;

/// The page's style: readable on screen, and the header repeated on each printed page.
const PAGE_STYLE: &str = "body { font-family: system-ui, sans-serif; margin: 2em; }
table { border-collapse: collapse; }
th, td { border: 1px solid #999; padding: 0.25em 0.5em; text-align: left; vertical-align: top; }
thead th { background: #eee; }
td.null { color: #777; font-style: italic; }
thead { display: table-header-group; }
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlOptions {
    pub document: HtmlDocument,
    /// What a `NULL` is shown as: by default `NULL`, as the grid shows it.
    pub null_text: String,
}

impl Default for HtmlOptions {
    fn default() -> Self {
        Self {
            document: HtmlDocument::default(),
            null_text: "NULL".to_owned(),
        }
    }
}

/// How much of a document is written around the table.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HtmlDocument {
    /// A whole UTF-8 page, with a little CSS that prints well.
    #[default]
    Page,
    /// The `<table>` alone, to paste into an email or a page.
    Fragment,
}

pub(crate) struct Html {
    options: HtmlOptions,
}

impl Html {
    pub(crate) fn new(options: HtmlOptions) -> Self {
        Self { options }
    }
}

impl TextFormat for Html {
    fn header(&mut self, out: &mut dyn Write, columns: &[ExportColumn]) -> io::Result<()> {
        if self.options.document == HtmlDocument::Page {
            write!(
                out,
                "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n\
                 <title>Exported rows</title>\n<style>\n{PAGE_STYLE}</style>\n</head>\n<body>\n"
            )?;
        }
        out.write_all(b"<table>\n<thead>\n<tr>")?;
        for column in columns {
            write!(out, "<th>{}</th>", escape(&column.name))?;
        }
        out.write_all(b"</tr>\n</thead>\n<tbody>\n")
    }

    fn row(&mut self, out: &mut dyn Write, row: &[Cell]) -> io::Result<()> {
        out.write_all(b"<tr>")?;
        for cell in row {
            match cell {
                Some(value) => write!(out, "<td>{}</td>", escape(value))?,
                None => write!(
                    out,
                    "<td class=\"null\">{}</td>",
                    escape(&self.options.null_text)
                )?,
            }
        }
        out.write_all(b"</tr>\n")
    }

    fn footer(&mut self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(b"</tbody>\n</table>\n")?;
        if self.options.document == HtmlDocument::Page {
            out.write_all(b"</body>\n</html>\n")?;
        }
        Ok(())
    }
}

/// Text as HTML: the characters markup gives meaning to as entities, and line breaks as
/// `<br>`, which a table cell would otherwise run together.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            '\r' => {
                chars.next_if_eq(&'\n');
                escaped.push_str("<br>");
            }
            '\n' => escaped.push_str("<br>"),
            c => escaped.push(c),
        }
    }
    escaped
}
