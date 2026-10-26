//! Writing result rows to CSV or JSON files, all at once or a page at a time.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use dbdesk_core::Row;
use serde::ser::SerializeMap;
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
    let mut writer = ExportWriter::create(path, format, columns)?;
    writer.write(rows)?;
    writer.finish()
}

/// Writes rows to a file a page at a time, so an export never holds them all.
///
/// They go to a file beside `path` (its name plus `.partial`), renamed to `path` by
/// [`ExportWriter::finish`]: an export that fails or is abandoned never leaves half a file
/// under the name chosen, nor replaces a file that was there.
pub struct ExportWriter {
    out: BufWriter<File>,
    format: ExportFormat,
    /// JSON's keys, one per column.
    keys: Vec<String>,
    /// How many rows were written so far.
    rows: usize,
    path: PathBuf,
    partial: PathBuf,
}

impl ExportWriter {
    /// Starts the file, writing CSV's header line.
    ///
    /// # Errors
    ///
    /// [`ExportError`] if the file can't be created or written.
    pub fn create(
        path: &Path,
        format: ExportFormat,
        columns: &[String],
    ) -> Result<Self, ExportError> {
        let mut partial_name = path.file_name().unwrap_or_default().to_owned();
        partial_name.push(".partial");
        let partial = path.with_file_name(partial_name);
        let failed = |source| ExportError {
            path: path.to_owned(),
            source,
        };
        let mut out = BufWriter::new(File::create(&partial).map_err(failed)?);
        if format == ExportFormat::Csv {
            let header: Vec<&str> = columns.iter().map(String::as_str).collect();
            write_csv_line(&mut out, header.into_iter()).map_err(failed)?;
        }
        Ok(Self {
            out,
            format,
            keys: unique_keys(columns),
            rows: 0,
            path: path.to_owned(),
            partial,
        })
    }

    /// Appends rows.
    ///
    /// # Errors
    ///
    /// [`ExportError`] if the file can't be written.
    pub fn write(&mut self, rows: &[Row]) -> Result<(), ExportError> {
        for row in rows {
            let written = match self.format {
                ExportFormat::Csv => write_csv_line(
                    &mut self.out,
                    row.iter().map(|cell| cell.as_deref().unwrap_or_default()),
                ),
                ExportFormat::Json => self.write_json_object(row),
            };
            written.map_err(|source| self.error(source))?;
            self.rows += 1;
        }
        Ok(())
    }

    /// How many rows have been written.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Ends the file and puts it in place.
    ///
    /// # Errors
    ///
    /// [`ExportError`] if the file can't be written or renamed.
    pub fn finish(mut self) -> Result<(), ExportError> {
        let ending: &[u8] = match (self.format, self.rows) {
            (ExportFormat::Csv, _) => b"",
            (ExportFormat::Json, 0) => b"[]\n",
            (ExportFormat::Json, _) => b"\n]\n",
        };
        let finished = self
            .out
            .write_all(ending)
            .and_then(|()| self.out.flush())
            .and_then(|()| fs::rename(&self.partial, &self.path));
        finished.map_err(|source| self.error(source))
    }

    /// Stops writing and removes what was written.
    pub fn abandon(self) {
        let Self { out, partial, .. } = self;
        drop(out);
        // Nothing more can be done about a partial file that won't go; its name says what it is.
        let _ = fs::remove_file(partial);
    }

    /// One object of the JSON array, laid out as `serde_json`'s pretty printer lays out an
    /// array of objects, so a file written page by page reads the same as one written at once.
    fn write_json_object(&mut self, row: &Row) -> io::Result<()> {
        let object = RowObject {
            keys: &self.keys,
            row,
        };
        let pretty = serde_json::to_string_pretty(&object).map_err(io::Error::from)?;
        let separator: &[u8] = if self.rows == 0 { b"[\n" } else { b",\n" };
        self.out.write_all(separator)?;
        for (line, text) in pretty.lines().enumerate() {
            if line > 0 {
                self.out.write_all(b"\n")?;
            }
            write!(self.out, "  {text}")?;
        }
        Ok(())
    }

    fn error(&self, source: io::Error) -> ExportError {
        ExportError {
            path: self.path.clone(),
            source,
        }
    }
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

/// Serializes a row as an object with keys in column order. A `serde_json::Map` would sort
/// them.
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

    /// What serde_json writes for the whole array at once, which the page-by-page writer must
    /// match: pages of one row each come out the same as one page of all of them.
    #[test]
    fn writes_the_same_file_a_page_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let (columns, rows) = sample();
        for format in [ExportFormat::Csv, ExportFormat::Json] {
            let path = dir.path().join(format!("paged.{}", format.extension()));
            let mut writer = ExportWriter::create(&path, format, &columns).unwrap();
            for row in &rows {
                writer.write(std::slice::from_ref(row)).unwrap();
            }
            assert_eq!(writer.rows(), 2);
            writer.finish().unwrap();
            let at_once = dir.path().join(format!("at-once.{}", format.extension()));
            export_rows(&at_once, format, &columns, &rows).unwrap();

            let paged = std::fs::read_to_string(&path).unwrap();
            assert_eq!(paged, std::fs::read_to_string(&at_once).unwrap());
            if format == ExportFormat::Json {
                let objects = RowObjectsForTest(&columns, &rows);
                assert_eq!(
                    paged,
                    serde_json::to_string_pretty(&objects).unwrap() + "\n"
                );
            }
        }
        let empty = dir.path().join("empty.json");
        export_rows(&empty, ExportFormat::Json, &columns, &[]).unwrap();
        assert_eq!(std::fs::read_to_string(&empty).unwrap(), "[]\n");
    }

    /// The rows as `serde_json` would write them all at once, to compare against.
    struct RowObjectsForTest<'a>(&'a [String], &'a [Row]);

    impl Serialize for RowObjectsForTest<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeSeq;
            let keys = unique_keys(self.0);
            let mut seq = serializer.serialize_seq(Some(self.1.len()))?;
            for row in self.1 {
                seq.serialize_element(&RowObject { keys: &keys, row })?;
            }
            seq.end()
        }
    }

    #[test]
    fn leaves_the_chosen_file_alone_until_finished_and_when_abandoned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rows.csv");
        std::fs::write(&path, "before").unwrap();
        let (columns, rows) = sample();

        let mut writer = ExportWriter::create(&path, ExportFormat::Csv, &columns).unwrap();
        writer.write(&rows).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "before");
        writer.abandon();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "before");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no partial file left"
        );
    }
}
