//! Reading CSV files to import, a record at a time: RFC 4180 (quoted fields, doubled quotes,
//! line breaks inside quotes), the counterpart of the export's writer, with the separator
//! chosen and CRLF or LF line ends. Each record knows the line it starts on, so a failure can
//! name it.

use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

/// What a UTF-8 file may start with, written by Excel among others; it isn't part of the data.
const BYTE_ORDER_MARK: char = '\u{feff}';

/// The character between fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separator {
    Comma,
    Semicolon,
    Tab,
}

impl Separator {
    pub fn char(self) -> char {
        match self {
            Self::Comma => ',',
            Self::Semicolon => ';',
            Self::Tab => '\t',
        }
    }
}

/// One record and the line of the file it starts on, counting from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvRecord {
    pub line: usize,
    pub fields: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CsvError {
    #[error("could not read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("line {line} isn't UTF-8 text")]
    NotText { line: usize },
    #[error("the quoted field that starts on line {line} never ends")]
    UnterminatedQuote { line: usize },
}

/// Reads records from CSV text.
pub struct CsvReader<R> {
    input: R,
    separator: char,
    path: PathBuf,
    /// The last line read, counting from 1.
    line: usize,
}

impl CsvReader<BufReader<File>> {
    /// Opens a file to read.
    ///
    /// # Errors
    ///
    /// [`CsvError::Read`] if the file can't be opened.
    pub fn open(path: &Path, separator: Separator) -> Result<Self, CsvError> {
        let file = File::open(path).map_err(|source| CsvError::Read {
            path: path.to_owned(),
            source,
        })?;
        Ok(Self::new(BufReader::new(file), separator, path))
    }
}

impl<R: BufRead> CsvReader<R> {
    /// Reads `input`; `path` names it in errors.
    pub fn new(input: R, separator: Separator, path: &Path) -> Self {
        Self {
            input,
            separator: separator.char(),
            path: path.to_owned(),
            line: 0,
        }
    }

    /// The next record, or `None` at the end. Blank lines are skipped.
    ///
    /// # Errors
    ///
    /// [`CsvError`] if the file can't be read, isn't text, or ends inside a quoted field.
    pub fn next_record(&mut self) -> Result<Option<CsvRecord>, CsvError> {
        loop {
            let start = self.line + 1;
            let Some(mut text) = self.read_line()? else {
                return Ok(None);
            };
            if start == 1 && text.starts_with(BYTE_ORDER_MARK) {
                text.remove(0);
            }
            if text.is_empty() {
                continue;
            }
            let mut record = Fields::new(self.separator);
            record.read(&text);
            // A line break inside quotes is part of the field: the record goes on.
            while record.in_quotes {
                let Some(more) = self.read_line()? else {
                    return Err(CsvError::UnterminatedQuote { line: start });
                };
                record.field.push('\n');
                record.read(&more);
            }
            return Ok(Some(CsvRecord {
                line: start,
                fields: record.finish(),
            }));
        }
    }

    /// Up to `count` records; fewer only at the end of the file.
    ///
    /// # Errors
    ///
    /// As [`CsvReader::next_record`].
    pub fn next_records(&mut self, count: usize) -> Result<Vec<CsvRecord>, CsvError> {
        let mut records = Vec::new();
        while records.len() < count {
            match self.next_record()? {
                Some(record) => records.push(record),
                None => break,
            }
        }
        Ok(records)
    }

    /// The next line without its line end, or `None` at the end of the input.
    fn read_line(&mut self) -> Result<Option<String>, CsvError> {
        let mut bytes = Vec::new();
        let read = self
            .input
            .read_until(b'\n', &mut bytes)
            .map_err(|source| CsvError::Read {
                path: self.path.clone(),
                source,
            })?;
        if read == 0 {
            return Ok(None);
        }
        self.line += 1;
        let mut text =
            String::from_utf8(bytes).map_err(|_| CsvError::NotText { line: self.line })?;
        if text.ends_with('\n') {
            text.pop();
        }
        if text.ends_with('\r') {
            text.pop();
        }
        Ok(Some(text))
    }
}

/// One record's fields, read a line at a time. A quoted field ends at a quote that isn't
/// doubled; a quote inside a field that didn't start with one is kept, as spreadsheet programs
/// do with `5" pipe`.
struct Fields {
    separator: char,
    fields: Vec<String>,
    field: String,
    in_quotes: bool,
}

impl Fields {
    fn new(separator: char) -> Self {
        Self {
            separator,
            fields: Vec::new(),
            field: String::new(),
            in_quotes: false,
        }
    }

    fn read(&mut self, text: &str) {
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' if self.in_quotes && chars.peek() == Some(&'"') => {
                    self.field.push('"');
                    chars.next();
                }
                '"' if self.in_quotes => self.in_quotes = false,
                '"' if self.field.is_empty() => self.in_quotes = true,
                c if c == self.separator && !self.in_quotes => {
                    self.fields.push(std::mem::take(&mut self.field));
                }
                c => self.field.push(c),
            }
        }
    }

    fn finish(mut self) -> Vec<String> {
        self.fields.push(self.field);
        self.fields
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(text: &str, separator: Separator) -> Result<Vec<CsvRecord>, CsvError> {
        let mut reader = CsvReader::new(text.as_bytes(), separator, Path::new("test.csv"));
        let mut records = Vec::new();
        while let Some(record) = reader.next_record()? {
            records.push(record);
        }
        Ok(records)
    }

    fn record(line: usize, fields: &[&str]) -> CsvRecord {
        CsvRecord {
            line,
            fields: fields.iter().map(|field| (*field).to_owned()).collect(),
        }
    }

    #[test]
    fn reads_quoted_fields_line_breaks_and_line_numbers() {
        let text = "\u{feff}id,note\r\n1,plain\r\n2,\"a, \"\"quoted\"\"\nline\",\r\n\r\n3,\n";

        assert_eq!(
            records(text, Separator::Comma).unwrap(),
            [
                record(1, &["id", "note"]),
                record(2, &["1", "plain"]),
                record(3, &["2", "a, \"quoted\"\nline", ""]),
                record(6, &["3", ""]),
            ]
        );
    }

    #[test]
    fn reads_what_the_export_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rows.csv");
        let columns = vec!["id".to_owned(), "note".to_owned()];
        let rows = vec![
            vec![Some("1".to_owned()), Some("it's, \"x\"\r\ny".to_owned())],
            vec![Some("2".to_owned()), None],
        ];
        crate::export_rows(&path, crate::ExportFormat::Csv, &columns, &rows).unwrap();

        let mut reader = CsvReader::open(&path, Separator::Comma).unwrap();
        let read = reader.next_records(10).unwrap();
        let fields: Vec<Vec<String>> = read.into_iter().map(|record| record.fields).collect();
        assert_eq!(
            fields,
            [
                vec!["id".to_owned(), "note".to_owned()],
                vec!["1".to_owned(), "it's, \"x\"\ny".to_owned()],
                vec!["2".to_owned(), String::new()],
            ]
        );
    }

    #[test]
    fn splits_on_the_separator_chosen_and_keeps_stray_quotes() {
        assert_eq!(
            records("a;\"b;c\";d,e\n", Separator::Semicolon).unwrap(),
            [record(1, &["a", "b;c", "d,e"])]
        );
        assert_eq!(
            records("a\tb,c\n", Separator::Tab).unwrap(),
            [record(1, &["a", "b,c"])]
        );
        assert_eq!(
            records("5\" pipe,2\n3,4\n", Separator::Comma).unwrap(),
            [record(1, &["5\" pipe", "2"]), record(2, &["3", "4"])]
        );
    }

    #[test]
    fn names_the_line_of_a_quote_that_never_ends_and_of_text_that_isnt_utf8() {
        assert!(matches!(
            records("a\n\"b\nc\n", Separator::Comma),
            Err(CsvError::UnterminatedQuote { line: 2 })
        ));
        let mut reader = CsvReader::new(&b"a\n\xff\n"[..], Separator::Comma, Path::new("x"));
        assert!(reader.next_record().unwrap().is_some());
        assert!(matches!(
            reader.next_record(),
            Err(CsvError::NotText { line: 2 })
        ));
    }
}
