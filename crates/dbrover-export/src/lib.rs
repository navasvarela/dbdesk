//! Writing rows out, in every format DbRover exports: to a file a page at a time, or all at
//! once to text for the clipboard, a program's stdin or an MCP answer. The design is in
//! `docs/exports.md`.
//!
//! A format is an [`ExportSpec`]: an [`ExportFormat`] with its options. Text formats implement
//! [`TextFormat`], which writes a header, each row and a footer to any writer, so a file and
//! the clipboard get the same bytes.

mod column;
mod spec;
mod text;
mod writer;

pub use column::ExportColumn;
pub use spec::{ExportChoice, ExportFormat, ExportSpec, export_choices};
pub use text::{
    CsvOptions, FieldSeparator, HtmlDocument, HtmlOptions, InvalidSeparator, LineEnding,
    MarkdownOptions, Quoting, TextFormat, TsvOptions, render,
};
pub use writer::{ExportError, ExportWriter, export_rows};

#[cfg(test)]
mod tests;
