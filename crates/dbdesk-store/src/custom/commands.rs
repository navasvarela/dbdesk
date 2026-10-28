//! Command actions: what `actions.toml` says about a program besides its arguments, and the
//! selected cells written as the text it reads on stdin. See `docs/hackability.md`.

use std::time::Duration;

use dbdesk_core::{CommandTemplate, Row, SelectedCells};
use serde::Deserialize;

use crate::export::{RowObject, unique_keys, write_csv_line};

/// Long enough for a program that reads a few thousand rows, short enough that one stuck on a
/// prompt nobody sees doesn't sit there all day.
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// An action that runs a program instead of SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandAction {
    pub command: CommandTemplate,
    pub stdin: StdinFormat,
    pub output: CommandOutput,
    /// How long the program may run before it is stopped.
    pub timeout: Duration,
}

/// What a program gets on stdin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StdinFormat {
    /// An array of objects keyed by column name, as Export writes it.
    Json,
    /// RFC 4180, with a header line, as Export writes it.
    Csv,
    /// Tab-separated, with a header line.
    Tsv,
    /// Nothing: stdin is empty, so a program that reads it ends rather than waits.
    #[default]
    None,
}

impl StdinFormat {
    /// As `actions.toml` writes it, and as `DBDESK_FORMAT` tells the program.
    pub fn name(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::None => "none",
        }
    }

    /// The selected columns of the selected rows, in this format. Empty for [`Self::None`].
    pub fn encode(self, cells: &SelectedCells) -> String {
        let columns: Vec<String> = cells
            .selected
            .iter()
            .filter_map(|&index| cells.columns.get(index).cloned())
            .collect();
        let rows: Vec<Row> = cells
            .rows
            .iter()
            .map(|row| {
                cells
                    .selected
                    .iter()
                    .map(|&index| row.get(index).cloned().flatten())
                    .collect()
            })
            .collect();
        match self {
            Self::Json => json(&columns, &rows),
            Self::Csv => csv(&columns, &rows),
            Self::Tsv => tsv(&columns, &rows),
            Self::None => String::new(),
        }
    }
}

/// Where a program's stdout goes when it succeeds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandOutput {
    /// A window like the cell viewer, JSON pretty-printed.
    #[default]
    Viewer,
    /// A new query console on the same data source, for a program that writes SQL.
    Console,
    /// The log console, with stderr.
    Log,
    None,
}

impl CommandOutput {
    /// As `actions.toml` writes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Console => "console",
            Self::Log => "log",
            Self::None => "none",
        }
    }
}

fn json(columns: &[String], rows: &[Row]) -> String {
    let keys = unique_keys(columns);
    let objects: Vec<RowObject<'_>> = rows
        .iter()
        .map(|row| RowObject { keys: &keys, row })
        .collect();
    // Serializing strings, nulls and maps of them can't fail.
    let mut text = serde_json::to_string_pretty(&objects).unwrap_or_default();
    text.push('\n');
    text
}

fn csv(columns: &[String], rows: &[Row]) -> String {
    let mut out = Vec::new();
    let header = columns.iter().map(String::as_str);
    // Writing to a `Vec` can't fail.
    let _ = write_csv_line(&mut out, header);
    for row in rows {
        let _ = write_csv_line(
            &mut out,
            row.iter().map(|cell| cell.as_deref().unwrap_or_default()),
        );
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn tsv(columns: &[String], rows: &[Row]) -> String {
    let line = |fields: Vec<String>| format!("{}\n", fields.join("\t"));
    let mut text = line(columns.iter().map(|name| tsv_field(Some(name))).collect());
    for row in rows {
        text.push_str(&line(
            row.iter().map(|cell| tsv_field(cell.as_deref())).collect(),
        ));
    }
    text
}

/// A field with a tab, a line break or a quote is quoted, with quotes doubled, the way
/// spreadsheets read TSV back. `NULL` is an empty field, as in CSV.
fn tsv_field(value: Option<&str>) -> String {
    let value = value.unwrap_or_default();
    if value.contains(['\t', '\n', '\r', '"']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use dbdesk_core::Cell;

    use super::*;

    fn text(value: &str) -> Cell {
        Some(value.to_owned())
    }

    fn selection() -> SelectedCells {
        SelectedCells {
            columns: vec!["id".to_owned(), "email".to_owned(), "note".to_owned()],
            selected: vec![0, 2],
            rows: vec![
                vec![text("1"), text("a@x"), text("tab\there, \"quoted\"")],
                vec![text("2"), text("b@x"), None],
            ],
        }
    }

    #[test]
    fn writes_the_selected_columns_of_the_selected_rows_in_each_format() {
        let cells = selection();

        assert_eq!(
            StdinFormat::Json.encode(&cells),
            "[\n  {\n    \"id\": \"1\",\n    \"note\": \"tab\\there, \\\"quoted\\\"\"\n  },\n  \
             {\n    \"id\": \"2\",\n    \"note\": null\n  }\n]\n"
        );
        assert_eq!(
            StdinFormat::Csv.encode(&cells),
            "id,note\r\n1,\"tab\there, \"\"quoted\"\"\"\r\n2,\r\n"
        );
        assert_eq!(
            StdinFormat::Tsv.encode(&cells),
            "id\tnote\n1\t\"tab\there, \"\"quoted\"\"\"\n2\t\n"
        );
        assert_eq!(StdinFormat::None.encode(&cells), "");
    }

    #[test]
    fn an_empty_selection_is_an_empty_array_or_just_the_header() {
        let cells = SelectedCells {
            rows: Vec::new(),
            ..selection()
        };
        assert_eq!(StdinFormat::Json.encode(&cells), "[]\n");
        assert_eq!(StdinFormat::Tsv.encode(&cells), "id\tnote\n");
    }
}
