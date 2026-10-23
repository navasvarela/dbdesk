//! Writing result rows to CSV or JSON files.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use dbdesk_core::Row;
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// RFC 4180: comma-separated, a header line, fields quoted when they need it. `NULL` is
    /// an empty field, as most tools read it.
    Csv,
    /// An array of objects keyed by column name, `NULL` as `null`. Values stay strings: the
    /// grid holds them as text (ADR-0004), so `1` and `'1'` can't be told apart.
    Json,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("could not write {}", path.display())]
pub struct ExportError {
    path: PathBuf,
    #[source]
    source: io::Error,
}

/// Writes `rows` under a header of `columns` to `path`, replacing it.
///
/// # Errors
///
/// [`ExportError`] if the file can't be created or written.
pub fn export_rows(
    path: &Path,
    format: ExportFormat,
    columns: &[String],
    rows: &[Row],
) -> Result<(), ExportError> {
    let write = || -> io::Result<()> {
        let mut out = BufWriter::new(File::create(path)?);
        match format {
            ExportFormat::Csv => write_csv(&mut out, columns, rows)?,
            ExportFormat::Json => write_json(&mut out, columns, rows)?,
        }
        out.flush()
    };
    write().map_err(|source| ExportError {
        path: path.to_owned(),
        source,
    })
}

fn write_csv(out: &mut impl Write, columns: &[String], rows: &[Row]) -> io::Result<()> {
    let header: Vec<&str> = columns.iter().map(String::as_str).collect();
    write_csv_line(out, header.into_iter())?;
    for row in rows {
        write_csv_line(
            out,
            row.iter().map(|cell| cell.as_deref().unwrap_or_default()),
        )?;
    }
    Ok(())
}

/// One record, ended with CRLF as RFC 4180 asks.
fn write_csv_line<'a>(
    out: &mut impl Write,
    fields: impl Iterator<Item = &'a str>,
) -> io::Result<()> {
    for (index, field) in fields.enumerate() {
        if index > 0 {
            out.write_all(b",")?;
        }
        if field.contains([',', '"', '\n', '\r']) {
            write!(out, "\"{}\"", field.replace('"', "\"\""))?;
        } else {
            out.write_all(field.as_bytes())?;
        }
    }
    out.write_all(b"\r\n")
}

fn write_json(out: &mut impl Write, columns: &[String], rows: &[Row]) -> io::Result<()> {
    let keys = unique_keys(columns);
    let objects = RowObjects { keys: &keys, rows };
    serde_json::to_writer_pretty(&mut *out, &objects).map_err(io::Error::from)?;
    out.write_all(b"\n")
}

/// Column names as JSON keys: a repeated name (`SELECT a.id, b.id`) gets `_2`, `_3`, … so no
/// value overwrites another.
fn unique_keys(columns: &[String]) -> Vec<String> {
    let mut used = HashSet::new();
    columns
        .iter()
        .map(|name| {
            let key = (1..)
                .map(|n| match n {
                    1 => name.clone(),
                    n => format!("{name}_{n}"),
                })
                .find(|key| !used.contains(key))
                .unwrap_or_else(|| name.clone());
            used.insert(key.clone());
            key
        })
        .collect()
}

/// Serializes rows as objects with keys in column order. A `serde_json::Map` would sort them.
struct RowObjects<'a> {
    keys: &'a [String],
    rows: &'a [Row],
}

impl Serialize for RowObjects<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.rows.len()))?;
        for row in self.rows {
            seq.serialize_element(&RowObject {
                keys: self.keys,
                row,
            })?;
        }
        seq.end()
    }
}

struct RowObject<'a> {
    keys: &'a [String],
    row: &'a Row,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Vec<String>, Vec<Row>) {
        let columns = vec!["id".to_owned(), "note".to_owned(), "id".to_owned()];
        let rows = vec![
            vec![
                Some("1".to_owned()),
                Some("plain".to_owned()),
                Some("7".to_owned()),
            ],
            vec![
                Some("2".to_owned()),
                Some("a, \"quoted\"\nline".to_owned()),
                None,
            ],
        ];
        (columns, rows)
    }

    #[test]
    fn writes_rfc_4180_csv() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rows.csv");
        let (columns, rows) = sample();

        export_rows(&path, ExportFormat::Csv, &columns, &rows).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "id,note,id\r\n1,plain,7\r\n2,\"a, \"\"quoted\"\"\nline\",\r\n"
        );
    }

    #[test]
    fn writes_json_objects_with_keys_in_column_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rows.json");
        let (columns, rows) = sample();

        export_rows(&path, ExportFormat::Json, &columns, &rows).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!([
                {"id": "1", "note": "plain", "id_2": "7"},
                {"id": "2", "note": "a, \"quoted\"\nline", "id_2": null},
            ])
        );
        assert!(text.find("\"note\"").unwrap() < text.find("\"id_2\"").unwrap());
    }

    #[test]
    fn reports_the_path_it_could_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/rows.csv");

        let error = export_rows(&path, ExportFormat::Csv, &[], &[]).unwrap_err();

        assert!(error.to_string().contains("missing/rows.csv"), "{error}");
    }
}
