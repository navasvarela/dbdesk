//! Comma-separated values (RFC 4180), and the options people change: the separator, the
//! quoting, the header, how `NULL` is written, the line ending and a byte order mark.

use std::io::{self, Write};

use dbrover_core::Cell;
use serde::{Deserialize, Serialize};

use super::TextFormat;
use crate::ExportColumn;

/// Excel takes a UTF-8 file for one only when it starts with this; other programs mostly
/// read it as part of the first column's name, which is why it is off by default.
const BYTE_ORDER_MARK: &str = "\u{feff}";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CsvOptions {
    pub separator: FieldSeparator,
    pub quoting: Quoting,
    /// Whether the first line names the columns.
    pub header: bool,
    /// What a `NULL` is written as: by default an empty field, as most programs read it.
    pub null_text: String,
    pub line_ending: LineEnding,
    /// Whether the file starts with a UTF-8 byte order mark.
    pub bom: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            separator: FieldSeparator::COMMA,
            quoting: Quoting::WhereNeeded,
            header: true,
            null_text: String::new(),
            line_ending: LineEnding::Crlf,
            bom: false,
        }
    }
}

impl CsvOptions {
    /// What Excel opens with accents intact: commas, CRLF, and a byte order mark. No `sep=`
    /// line: Microsoft doesn't document it, and most other programs read it as data.
    pub fn for_excel() -> Self {
        Self {
            bom: true,
            ..Self::default()
        }
    }
}

/// The character between fields. Any but a quote or a line break, which would make a file
/// that can't be read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "char", into = "char")]
pub struct FieldSeparator(char);

impl FieldSeparator {
    pub const COMMA: Self = Self(',');
    pub const SEMICOLON: Self = Self(';');
    pub const TAB: Self = Self('\t');
    pub const PIPE: Self = Self('|');

    pub fn get(self) -> char {
        self.0
    }
}

impl From<FieldSeparator> for char {
    fn from(separator: FieldSeparator) -> Self {
        separator.0
    }
}

impl TryFrom<char> for FieldSeparator {
    type Error = InvalidSeparator;

    fn try_from(separator: char) -> Result<Self, Self::Error> {
        match separator {
            '"' | '\r' | '\n' => Err(InvalidSeparator(separator)),
            separator => Ok(Self(separator)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} can't separate fields: it is a quote or a line break")]
pub struct InvalidSeparator(char);

/// Which fields are put in quotes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quoting {
    /// Fields holding the separator, a quote or a line break, as RFC 4180 asks.
    #[default]
    WhereNeeded,
    /// Every field but a `NULL`, which stays unquoted so a reader can tell it from text.
    Always,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineEnding {
    /// As RFC 4180 asks, and Windows programs expect.
    #[default]
    Crlf,
    Lf,
}

impl LineEnding {
    fn as_str(self) -> &'static str {
        match self {
            Self::Crlf => "\r\n",
            Self::Lf => "\n",
        }
    }
}

pub(crate) struct Csv {
    options: CsvOptions,
}

impl Csv {
    pub(crate) fn new(options: CsvOptions) -> Self {
        Self { options }
    }
}

/// One record: `fields` separated, quoted as `options` say, ended with the line ending.
fn line<'a>(
    options: &CsvOptions,
    out: &mut dyn Write,
    fields: impl Iterator<Item = Field<'a>>,
) -> io::Result<()> {
    let separator = options.separator.get();
    let needs_quotes = |text: &str| text.contains([separator, '"', '\r', '\n']);
    for (index, field) in fields.enumerate() {
        if index > 0 {
            write!(out, "{separator}")?;
        }
        let quoted = match (field, options.quoting) {
            (Field::Text(_), Quoting::Always) => true,
            (Field::Text(text) | Field::Null(text), _) => needs_quotes(text),
        };
        let text = field.text();
        if quoted {
            write!(out, "\"{}\"", text.replace('"', "\"\""))?;
        } else {
            out.write_all(text.as_bytes())?;
        }
    }
    out.write_all(options.line_ending.as_str().as_bytes())
}

/// A field's text, and whether it stands for `NULL`.
#[derive(Clone, Copy)]
enum Field<'a> {
    Text(&'a str),
    Null(&'a str),
}

impl<'a> Field<'a> {
    fn of(cell: &'a Cell, null_text: &'a str) -> Self {
        match cell {
            Some(text) => Self::Text(text),
            None => Self::Null(null_text),
        }
    }

    fn text(self) -> &'a str {
        match self {
            Self::Text(text) | Self::Null(text) => text,
        }
    }
}

impl TextFormat for Csv {
    fn header(&mut self, out: &mut dyn Write, columns: &[ExportColumn]) -> io::Result<()> {
        if self.options.bom {
            out.write_all(BYTE_ORDER_MARK.as_bytes())?;
        }
        if self.options.header {
            let names = columns.iter().map(|column| Field::Text(&column.name));
            line(&self.options, out, names)?;
        }
        Ok(())
    }

    fn row(&mut self, out: &mut dyn Write, row: &[Cell]) -> io::Result<()> {
        let null_text = &self.options.null_text;
        line(
            &self.options,
            out,
            row.iter().map(|cell| Field::of(cell, null_text)),
        )
    }

    fn footer(&mut self, _out: &mut dyn Write) -> io::Result<()> {
        Ok(())
    }
}
