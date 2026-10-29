//! JSON: an array of objects, or one object a line (JSON Lines). Keys are the column names in
//! column order, values are strings or `null` (ADR 0004).

use std::io::{self, Write};

use dbrover_core::Cell;
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

use super::TextFormat;
use crate::ExportColumn;
use crate::column::unique_keys;

/// An array of objects, laid out as `serde_json`'s pretty printer lays out the whole array,
/// so a file written a page at a time reads the same as one written at once.
#[derive(Default)]
pub(crate) struct Json {
    keys: Vec<String>,
    rows: usize,
}

impl TextFormat for Json {
    fn header(&mut self, _out: &mut dyn Write, columns: &[ExportColumn]) -> io::Result<()> {
        self.keys = unique_keys(columns);
        Ok(())
    }

    fn row(&mut self, out: &mut dyn Write, row: &[Cell]) -> io::Result<()> {
        let object = RowObject {
            keys: &self.keys,
            row,
        };
        let pretty = serde_json::to_string_pretty(&object).map_err(io::Error::from)?;
        let separator: &[u8] = if self.rows == 0 { b"[\n" } else { b",\n" };
        out.write_all(separator)?;
        for (line, text) in pretty.lines().enumerate() {
            if line > 0 {
                out.write_all(b"\n")?;
            }
            write!(out, "  {text}")?;
        }
        self.rows += 1;
        Ok(())
    }

    fn footer(&mut self, out: &mut dyn Write) -> io::Result<()> {
        let ending: &[u8] = match self.rows {
            0 => b"[]\n",
            _ => b"\n]\n",
        };
        out.write_all(ending)
    }
}

/// One compact object a line, which `jq -c` and data pipelines read a line at a time.
#[derive(Default)]
pub(crate) struct JsonLines {
    keys: Vec<String>,
}

impl TextFormat for JsonLines {
    fn header(&mut self, _out: &mut dyn Write, columns: &[ExportColumn]) -> io::Result<()> {
        self.keys = unique_keys(columns);
        Ok(())
    }

    fn row(&mut self, out: &mut dyn Write, row: &[Cell]) -> io::Result<()> {
        let object = RowObject {
            keys: &self.keys,
            row,
        };
        serde_json::to_writer(&mut *out, &object).map_err(io::Error::from)?;
        out.write_all(b"\n")
    }

    fn footer(&mut self, _out: &mut dyn Write) -> io::Result<()> {
        Ok(())
    }
}

/// Serializes a row as an object with keys in column order. A `serde_json::Map` would sort
/// them.
struct RowObject<'a> {
    keys: &'a [String],
    row: &'a [Cell],
}

impl Serialize for RowObject<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.keys.len()))?;
        for (key, cell) in self.keys.iter().zip(self.row) {
            map.serialize_entry(key, cell)?;
        }
        map.end()
    }
}
