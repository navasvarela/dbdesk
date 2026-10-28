//! What a grid edit did to its row: the values it replaced, kept beside the statement so the
//! history can show them and the statement that would put them back.

use dbrover_core::Cell;
use serde::{Deserialize, Serialize};

/// One row's change, as the grid's Submit made it. Stored as JSON in the `changes` column.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RowChange {
    /// The table, as `schema.name`.
    pub table: String,
    #[serde(flatten)]
    pub kind: RowChangeKind,
    /// The statement that would put the row back as it was, in the data source's dialect;
    /// `None` for an inserted row, whose generated key isn't known. Nothing runs it: it is
    /// there to review, copy and run by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revert: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum RowChangeKind {
    /// Cells of a loaded row changed. `key` is the row's primary key as it was loaded.
    Updated {
        key: Vec<NamedCell>,
        cells: Vec<ReplacedCell>,
    },
    /// A loaded row deleted: its key, and every value the grid had loaded.
    Deleted {
        key: Vec<NamedCell>,
        row: Vec<NamedCell>,
    },
    /// A row added, with the values typed into it; the database chose the rest.
    Inserted { values: Vec<NamedCell> },
}

/// A column and its value, `None` for `NULL`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NamedCell {
    pub column: String,
    pub value: Cell,
}

/// A cell an edit changed: its value before, and the value written.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReplacedCell {
    pub column: String,
    pub old: Cell,
    pub new: Cell,
}

impl RowChange {
    pub(super) fn to_json(&self) -> String {
        // Strings, options and vectors only: nothing here can fail to serialise.
        serde_json::to_string(self).unwrap_or_default()
    }

    /// `None` for JSON this DbRover can't read (written by a newer one, or edited by hand):
    /// the entry still shows, without its change.
    pub(super) fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(column: &str, value: Option<&str>) -> NamedCell {
        NamedCell {
            column: column.to_owned(),
            value: value.map(str::to_owned),
        }
    }

    #[test]
    fn round_trips_through_json() {
        let changes = [
            RowChange {
                table: "public.people".to_owned(),
                kind: RowChangeKind::Updated {
                    key: vec![cell("id", Some("1"))],
                    cells: vec![ReplacedCell {
                        column: "name".to_owned(),
                        old: Some("ada".to_owned()),
                        new: None,
                    }],
                },
                revert: Some("UPDATE …".to_owned()),
            },
            RowChange {
                table: "public.people".to_owned(),
                kind: RowChangeKind::Deleted {
                    key: vec![cell("id", Some("2"))],
                    row: vec![cell("id", Some("2")), cell("name", None)],
                },
                revert: Some("INSERT …".to_owned()),
            },
            RowChange {
                table: "public.people".to_owned(),
                kind: RowChangeKind::Inserted {
                    values: vec![cell("name", Some("bob"))],
                },
                revert: None,
            },
        ];
        for change in changes {
            assert_eq!(RowChange::from_json(&change.to_json()), Some(change));
        }
        assert_eq!(
            RowChange::from_json(r#"{"table":"t","change":"merged"}"#),
            None
        );
    }

    /// The JSON is the file's format, read by later versions: its shape stays as it is.
    #[test]
    fn writes_the_documented_shape() {
        let change = RowChange {
            table: "main.people".to_owned(),
            kind: RowChangeKind::Updated {
                key: vec![cell("id", Some("1"))],
                cells: vec![ReplacedCell {
                    column: "name".to_owned(),
                    old: Some("ada".to_owned()),
                    new: Some("Ada".to_owned()),
                }],
            },
            revert: None,
        };
        assert_eq!(
            change.to_json(),
            r#"{"table":"main.people","change":"updated","key":[{"column":"id","value":"1"}],"cells":[{"column":"name","old":"ada","new":"Ada"}]}"#
        );
    }
}
