//! What the import dialog shows before anything is imported: the file's first records, which
//! of its columns goes into which of the table's, and the values that don't look like their
//! column's type. Pure; the engine reads the whole file when importing.

use std::path::Path;

use dbdesk_core::{ColumnDescription, TableDescription};
use dbdesk_store::{CsvReader, CsvRecord, Separator};

/// How many records the preview reads and checks.
pub const PREVIEW_RECORDS: usize = 20;

/// The start of a file: its header, if it has one, and the records after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub header: Option<Vec<String>>,
    pub records: Vec<CsvRecord>,
}

impl Preview {
    /// Reads the start of the file at `path`.
    ///
    /// Reads on the calling thread, which the UI can afford: it stops after a few records,
    /// whatever the size of the file.
    pub fn read(path: &Path, separator: Separator, header: bool) -> Result<Self, String> {
        let describe = |error: dbdesk_store::CsvError| dbdesk_core::describe_error(&error);
        let mut reader = CsvReader::open(path, separator).map_err(describe)?;
        let header = if header {
            reader
                .next_record()
                .map_err(describe)?
                .map(|record| record.fields)
        } else {
            None
        };
        let records = reader.next_records(PREVIEW_RECORDS).map_err(describe)?;
        Ok(Self { header, records })
    }

    /// The file's columns, by their header or, without one, by number: as many as the widest
    /// record has.
    pub fn columns(&self) -> Vec<String> {
        let widest = self
            .records
            .iter()
            .map(|record| record.fields.len())
            .chain(self.header.iter().map(Vec::len))
            .max()
            .unwrap_or(0);
        (0..widest)
            .map(|index| {
                self.header
                    .as_ref()
                    .and_then(|header| header.get(index))
                    .filter(|name| !name.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| format!("Column {}", index + 1))
            })
            .collect()
    }

    /// Where each of the table's columns comes from, as a first guess: the file's column with
    /// the same name, ignoring case, if the file has a header; otherwise the file's column in
    /// the same place.
    pub fn match_columns(&self, table: &TableDescription) -> Vec<Option<usize>> {
        let width = self.columns().len();
        table
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| match &self.header {
                Some(header) => header
                    .iter()
                    .position(|name| name.trim().eq_ignore_ascii_case(&column.name)),
                None => (index < width).then_some(index),
            })
            .collect()
    }
}

/// A preview value that the database will probably refuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub line: usize,
    pub column: String,
    pub message: String,
}

/// Checks each previewed value against the column it goes into. `mapping` gives, for each
/// of the table's columns, the file's column it comes from, if any.
pub fn check(
    table: &TableDescription,
    mapping: &[Option<usize>],
    preview: &Preview,
) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for record in &preview.records {
        for (column, field) in table.columns.iter().zip(mapping) {
            let Some(field) = field else {
                continue;
            };
            let value = record.fields.get(*field).map_or("", String::as_str);
            if let Some(message) = problem(column, value) {
                warnings.push(Warning {
                    line: record.line,
                    column: column.name.clone(),
                    message,
                });
            }
        }
    }
    warnings
}

/// What looks wrong with `value` for `column`. An empty field goes in as `NULL`.
fn problem(column: &ColumnDescription, value: &str) -> Option<String> {
    if value.is_empty() {
        let refused = !column.nullable && column.default.is_none();
        return refused.then(|| format!("empty, so NULL, but {} can't be NULL", column.name));
    }
    let value = value.trim();
    let kind = Kind::of(&column.data_type);
    let fits = match kind {
        Kind::Integer => value.parse::<i128>().is_ok(),
        Kind::Decimal => value.parse::<f64>().is_ok(),
        Kind::Boolean => BOOLEANS.contains(&value.to_ascii_lowercase().as_str()),
        Kind::Date => is_date(value),
        Kind::Other => true,
    };
    (!fits).then(|| format!("\u{201c}{value}\u{201d} isn't {}", kind.a()))
}

/// What the engines accept as a boolean, in any case.
const BOOLEANS: &[&str] = &[
    "true", "false", "t", "f", "yes", "no", "y", "n", "on", "off", "1", "0",
];

/// The kinds of type the preview can check a value against. Any other type is left to the
/// database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Integer,
    Decimal,
    Boolean,
    Date,
    Other,
}

impl Kind {
    /// From a type as the database spells it: `integer`, `int(11) unsigned`,
    /// `character varying(20)`, `double precision`.
    fn of(data_type: &str) -> Self {
        let lower = data_type.to_ascii_lowercase();
        let base = lower
            .split(|c: char| c == '(' || c.is_whitespace())
            .next()
            .unwrap_or_default();
        match base {
            "int" | "integer" | "bigint" | "smallint" | "tinyint" | "mediumint" | "int2"
            | "int4" | "int8" | "serial" | "bigserial" | "smallserial" | "hugeint" | "ubigint"
            | "uinteger" | "usmallint" | "utinyint" => Self::Integer,
            "numeric" | "decimal" | "real" | "double" | "float" | "float4" | "float8" | "money"
            | "smallmoney" => Self::Decimal,
            "boolean" | "bool" | "bit" => Self::Boolean,
            "date" => Self::Date,
            _ => Self::Other,
        }
    }

    /// The kind, after "isn't".
    fn a(self) -> &'static str {
        match self {
            Self::Integer => "a whole number",
            Self::Decimal => "a number",
            Self::Boolean => "true or false",
            Self::Date => "a date (YYYY-MM-DD)",
            Self::Other => "a value",
        }
    }
}

/// `YYYY-MM-DD`, with a month and day that could exist.
fn is_date(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    let number = |part: &str, digits: usize| {
        (part.len() == digits && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse::<u32>().ok())
            .flatten()
    };
    matches!(
        (number(year, 4), number(month, 2), number(day, 2)),
        (Some(_), Some(1..=12), Some(1..=31))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, data_type: &str, nullable: bool) -> ColumnDescription {
        ColumnDescription {
            name: name.to_owned(),
            data_type: data_type.to_owned(),
            nullable,
            default: None,
            identity: None,
        }
    }

    fn table(columns: Vec<ColumnDescription>) -> TableDescription {
        TableDescription {
            schema: "main".to_owned(),
            name: "orders".to_owned(),
            columns,
            primary_key: vec![],
            foreign_keys: vec![],
            referenced_by: vec![],
        }
    }

    fn preview(header: Option<&[&str]>, records: &[&[&str]]) -> Preview {
        Preview {
            header: header.map(|names| names.iter().map(|name| (*name).to_owned()).collect()),
            records: records
                .iter()
                .enumerate()
                .map(|(index, fields)| CsvRecord {
                    line: index + 2,
                    fields: fields.iter().map(|field| (*field).to_owned()).collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn matches_columns_by_header_name_or_else_by_place() {
        let orders = table(vec![
            column("id", "integer", false),
            column("customer", "text", true),
            column("note", "text", true),
        ]);
        let named = preview(Some(&["Customer", "ID", "extra"]), &[]);
        assert_eq!(named.match_columns(&orders), [Some(1), Some(0), None]);
        assert_eq!(named.columns(), ["Customer", "ID", "extra"]);

        let unnamed = preview(None, &[&["1", "ada"]]);
        assert_eq!(unnamed.match_columns(&orders), [Some(0), Some(1), None]);
        assert_eq!(unnamed.columns(), ["Column 1", "Column 2"]);
    }

    #[test]
    fn flags_values_that_dont_fit_their_column() {
        let orders = table(vec![
            column("id", "INTEGER", false),
            column("price", "numeric(10, 2)", true),
            column("paid", "boolean", true),
            column("due", "date", true),
            column("note", "character varying(20)", true),
        ]);
        let rows = preview(
            None,
            &[
                &["1", "9.99", "TRUE", "2026-10-26", "anything"],
                &["", "abc", "maybe", "2026-13-01", ""],
            ],
        );
        let mapping = [Some(0), Some(1), Some(2), Some(3), Some(4)];

        let messages: Vec<(usize, String, String)> = check(&orders, &mapping, &rows)
            .into_iter()
            .map(|warning| (warning.line, warning.column, warning.message))
            .collect();
        assert_eq!(
            messages,
            [
                (
                    3,
                    "id".to_owned(),
                    "empty, so NULL, but id can't be NULL".to_owned()
                ),
                (
                    3,
                    "price".to_owned(),
                    "\u{201c}abc\u{201d} isn't a number".to_owned()
                ),
                (
                    3,
                    "paid".to_owned(),
                    "\u{201c}maybe\u{201d} isn't true or false".to_owned()
                ),
                (
                    3,
                    "due".to_owned(),
                    "\u{201c}2026-13-01\u{201d} isn't a date (YYYY-MM-DD)".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn reads_the_start_of_a_file_with_or_without_its_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("orders.csv");
        let lines: String = std::iter::once("id;name\n".to_owned())
            .chain((1..=30).map(|id| format!("{id};name {id}\n")))
            .collect();
        std::fs::write(&path, lines).unwrap();

        let with_header = Preview::read(&path, Separator::Semicolon, true).unwrap();
        assert_eq!(with_header.columns(), ["id", "name"]);
        assert_eq!(with_header.records.len(), PREVIEW_RECORDS);
        assert_eq!(with_header.records[0].line, 2);

        let without = Preview::read(&path, Separator::Semicolon, false).unwrap();
        assert_eq!(without.records[0].fields, ["id", "name"]);
    }
}
