//! Documents flattened into columns, as the grid shows a result of documents (ADR 0006).
//!
//! The columns are the top-level fields of the documents read so far: `_id` first, the rest in
//! the order they first appear. A page of documents only ever adds columns at the right, so a
//! column keeps its place, and its sort and selection, while pages arrive. Expanding a column
//! turns an object's fields into columns of their own in its place (`address.city`), and
//! collapsing puts it back; those two rearrange the columns, and the grid starts over.

use std::collections::HashSet;

use crate::{Path, Step, Syntax, Value};

/// How many columns the grid gets. Each column is drawn and laid out every frame, and a
/// document with thousands of fields would make the grid both slow and unreadable; the rest
/// are in the tree, which shows a document whole. Chosen, not measured: a few screens wide.
pub const MAX_COLUMNS: usize = 200;

/// The name of the column holding a row that isn't a document, when one came.
pub const UNREADABLE_COLUMN: &str = "(not a document)";

/// The documents of one result, read, and the columns they flatten into.
#[derive(Debug, Default)]
pub struct Flattened {
    documents: Vec<Document>,
    /// Every column, in order, the ones past [`MAX_COLUMNS`] included.
    columns: Vec<Column>,
    /// The fields expanded into columns of their own.
    expanded: HashSet<Path>,
}

/// One document, or the text of a row that wasn't one.
#[derive(Debug)]
pub enum Document {
    Read(Value),
    /// What came, and why it can't be read: a row of other than one cell, or text that isn't
    /// JSON. A driver's mistake, shown rather than failing the result.
    Unreadable {
        text: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Column {
    Field(Path),
    Unreadable,
}

/// What one of a document's columns holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCell<'a> {
    /// The document has no such field, which is not the same as `null`.
    Missing,
    Value(&'a Value),
    /// The text of a row that isn't a document, in [`UNREADABLE_COLUMN`].
    Unreadable(&'a str),
}

/// How the columns changed when a page was added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnsChange {
    /// None added.
    Unchanged,
    /// Some added at the right; every column before them is where it was.
    Appended,
    /// `_id` arrived after other columns and went first, moving every other column.
    Rearranged,
}

impl Flattened {
    /// Reads a page of rows, each a document as canonical Extended JSON text in its one cell,
    /// and adds the columns they bring.
    pub fn add_page(&mut self, rows: &[Vec<Option<String>>]) -> ColumnsChange {
        let mut change = ColumnsChange::Unchanged;
        for row in rows {
            let document = read_row(row);
            let added = self.add_columns_of(&document);
            self.documents.push(document);
            change = match (change, added) {
                (ColumnsChange::Rearranged, _) | (_, ColumnsChange::Rearranged) => {
                    ColumnsChange::Rearranged
                }
                (ColumnsChange::Appended, _) | (_, ColumnsChange::Appended) => {
                    ColumnsChange::Appended
                }
                _ => ColumnsChange::Unchanged,
            };
        }
        change
    }

    pub fn documents(&self) -> &[Document] {
        &self.documents
    }

    /// The columns the grid shows, by name: at most [`MAX_COLUMNS`].
    pub fn column_names(&self) -> Vec<String> {
        self.columns
            .iter()
            .take(MAX_COLUMNS)
            .map(|column| match column {
                Column::Field(path) => path.to_string(),
                Column::Unreadable => UNREADABLE_COLUMN.to_owned(),
            })
            .collect()
    }

    /// How many columns there are past [`MAX_COLUMNS`], which only the tree shows.
    pub fn hidden_columns(&self) -> usize {
        self.columns.len().saturating_sub(MAX_COLUMNS)
    }

    /// What the document at `row` holds in the shown column `column`.
    pub fn cell(&self, row: usize, column: usize) -> FieldCell<'_> {
        let (Some(document), Some(column)) = (
            self.documents.get(row),
            self.columns[..self.shown()].get(column),
        ) else {
            return FieldCell::Missing;
        };
        match (document, column) {
            (Document::Read(value), Column::Field(path)) => match field(value, path) {
                // An expanded field's objects are in the columns it was expanded into.
                Some(found) if !(self.expanded.contains(path) && is_plain_object(found)) => {
                    FieldCell::Value(found)
                }
                _ => FieldCell::Missing,
            },
            (Document::Unreadable { text, .. }, Column::Unreadable) => FieldCell::Unreadable(text),
            (Document::Read(_), Column::Unreadable)
            | (Document::Unreadable { .. }, Column::Field(_)) => FieldCell::Missing,
        }
    }

    /// The grid's rows for the documents from `from` on: each shown column's text, as the
    /// grid sorts, filters and copies it. A scalar is as the shell writes it, an object or array
    /// is compact Extended JSON, and a missing field is `None`, like `null`; [`Self::cell`]
    /// tells the two apart for drawing.
    pub fn rows(&self, from: usize) -> Vec<Vec<Option<String>>> {
        let columns = self.shown();
        (from..self.documents.len())
            .map(|row| {
                (0..columns)
                    .map(|column| self.cell(row, column).text())
                    .collect()
            })
            .collect()
    }

    /// Whether the shown column `column` holds an object in some document, so it can be
    /// expanded into its fields.
    pub fn can_expand(&self, column: usize) -> bool {
        let Some(Column::Field(path)) = self.columns.get(column) else {
            return false;
        };
        !self.expanded.contains(path)
            && self.documents
            .iter()
            .any(|document| matches!(document, Document::Read(value) if field(value, path).is_some_and(is_plain_object)))
    }

    /// The field the shown column `column` was expanded from, which collapsing it puts back.
    pub fn expanded_from(&self, column: usize) -> Option<Path> {
        let Some(Column::Field(path)) = self.columns.get(column) else {
            return None;
        };
        (1..path.steps().len())
            .rev()
            .map(|length| Path::from(path.steps()[..length].to_vec()))
            .find(|parent| self.expanded.contains(parent))
    }

    /// Turns the shown column `column`'s objects into a column for each of their fields, in
    /// its place. Every column after it moves. If some document holds something other than an
    /// object there, the column stays too, for those values.
    pub fn expand(&mut self, column: usize) {
        if let Some(Column::Field(path)) = self.columns.get(column) {
            self.expanded.insert(path.clone());
            self.recompute_columns();
        }
    }

    /// Puts the columns expanded from `field` back into one.
    pub fn collapse(&mut self, field: &Path) {
        self.expanded
            .retain(|expanded| !starts_with(expanded, field));
        self.recompute_columns();
    }

    fn shown(&self) -> usize {
        self.columns.len().min(MAX_COLUMNS)
    }

    /// The columns again from every document, after expanding or collapsing one: the
    /// fields in the order they first appear, `_id` first, and each expanded field's columns
    /// together where it was, unlike a page's new columns, which go at the right.
    fn recompute_columns(&mut self) {
        let mut tree = FieldTree::default();
        let mut unreadable = false;
        for document in &self.documents {
            match document {
                Document::Read(Value::Object(fields)) => {
                    tree.add(&Path::default(), fields, &self.expanded)
                }
                Document::Read(_) | Document::Unreadable { .. } => unreadable = true,
            }
        }
        if let Some(id) = tree.fields.iter().position(|(name, _)| name == "_id") {
            let id = tree.fields.remove(id);
            tree.fields.insert(0, id);
        }
        let mut columns = Vec::new();
        tree.columns(&Path::default(), &mut columns);
        if unreadable {
            columns.push(Column::Unreadable);
        }
        self.columns = columns;
    }

    fn add_columns_of(&mut self, document: &Document) -> ColumnsChange {
        let mut columns = Columns {
            seen: self.columns.iter().cloned().collect(),
            order: std::mem::take(&mut self.columns),
        };
        let before = columns.order.len();
        let id_first = columns.add(document, &self.expanded);
        self.columns = columns.order;
        if id_first && before > 0 {
            ColumnsChange::Rearranged
        } else if self.columns.len() > before {
            ColumnsChange::Appended
        } else {
            ColumnsChange::Unchanged
        }
    }
}

impl FieldCell<'_> {
    fn text(self) -> Option<String> {
        match self {
            Self::Missing | Self::Value(Value::Null) => None,
            Self::Value(value) => Some(value.copied(Syntax::ExtendedJson)),
            Self::Unreadable(text) => Some(text.to_owned()),
        }
    }
}

/// Columns being gathered, in order, with a set to find them by.
#[derive(Default)]
struct Columns {
    seen: HashSet<Column>,
    order: Vec<Column>,
}

impl Columns {
    /// Adds the columns `document` has that aren't here yet. Returns whether `_id` was among
    /// them and went first.
    fn add(&mut self, document: &Document, expanded: &HashSet<Path>) -> bool {
        match document {
            Document::Read(Value::Object(fields)) => {
                self.add_fields(&Path::default(), fields, expanded)
            }
            // A document is an object; anything else read from a row isn't one.
            Document::Read(_) | Document::Unreadable { .. } => {
                self.push(Column::Unreadable);
                false
            }
        }
    }

    fn add_fields(
        &mut self,
        parent: &Path,
        fields: &[(String, Value)],
        expanded: &HashSet<Path>,
    ) -> bool {
        let mut id_first = false;
        for (name, value) in fields {
            let path = parent.child(Step::Field(name.clone()));
            if expanded.contains(&path)
                && let Value::Object(inner) = value
                && is_plain_object(value)
            {
                self.add_fields(&path, inner, expanded);
                continue;
            }
            let column = Column::Field(path);
            if self.seen.contains(&column) {
                continue;
            }
            if parent.steps().is_empty() && name == "_id" {
                self.seen.insert(column.clone());
                self.order.insert(0, column);
                id_first = true;
            } else {
                self.push(column);
            }
        }
        id_first
    }

    fn push(&mut self, column: Column) {
        if self.seen.insert(column.clone()) {
            self.order.push(column);
        }
    }
}

/// The fields of every document, in the order they first appear, nested where expanded.
#[derive(Default)]
struct FieldTree {
    fields: Vec<(String, FieldTree)>,
    /// Some document has a value here that is a column of its own: the field isn't expanded,
    /// or it is but holds something other than an object.
    own_column: bool,
}

impl FieldTree {
    fn add(&mut self, parent: &Path, fields: &[(String, Value)], expanded: &HashSet<Path>) {
        for (name, value) in fields {
            let path = parent.child(Step::Field(name.clone()));
            let index = match self.fields.iter().position(|(field, _)| field == name) {
                Some(index) => index,
                None => {
                    self.fields.push((name.clone(), Self::default()));
                    self.fields.len() - 1
                }
            };
            let Some((_, node)) = self.fields.get_mut(index) else {
                continue;
            };
            match value {
                Value::Object(inner) if expanded.contains(&path) && is_plain_object(value) => {
                    node.add(&path, inner, expanded);
                }
                _ => node.own_column = true,
            }
        }
    }

    /// The columns, depth first: a field's own column, then those it was expanded into.
    fn columns(&self, parent: &Path, out: &mut Vec<Column>) {
        for (name, node) in &self.fields {
            let path = parent.child(Step::Field(name.clone()));
            if node.own_column {
                out.push(Column::Field(path.clone()));
            }
            node.columns(&path, out);
        }
    }
}

/// A row as a document: its one cell read as Extended JSON.
fn read_row(row: &[Option<String>]) -> Document {
    let unreadable = |reason: String| Document::Unreadable {
        text: row
            .iter()
            .map(|cell| cell.as_deref().unwrap_or("null"))
            .collect::<Vec<_>>()
            .join("\t"),
        reason,
    };
    match row {
        [Some(text)] => match Value::parse(text) {
            Ok(value @ Value::Object(_)) => Document::Read(value),
            Ok(_) => unreadable("not an object".to_owned()),
            Err(error) => unreadable(format!("not JSON: {error}")),
        },
        [None] => unreadable("null".to_owned()),
        cells => unreadable(format!("a row of {} cells, not one", cells.len())),
    }
}

/// The value at `path` in `document`, through objects only.
fn field<'a>(document: &'a Value, path: &Path) -> Option<&'a Value> {
    path.steps()
        .iter()
        .try_fold(document, |value, step| match (value, step) {
            (Value::Object(fields), Step::Field(name)) => fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, value)| value),
            _ => None,
        })
}

/// An object with fields of its own, rather than one of Extended JSON's special values.
fn is_plain_object(value: &Value) -> bool {
    matches!(value, Value::Object(fields) if !fields.is_empty())
        && !value.children(Syntax::ExtendedJson).is_empty()
}

fn starts_with(path: &Path, prefix: &Path) -> bool {
    path.steps().starts_with(prefix.steps())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(documents: &[&str]) -> Vec<Vec<Option<String>>> {
        documents
            .iter()
            .map(|document| vec![Some((*document).to_owned())])
            .collect()
    }

    fn texts(flattened: &Flattened) -> Vec<Vec<Option<String>>> {
        flattened.rows(0)
    }

    fn cells(texts: &[Option<&str>]) -> Vec<Option<String>> {
        texts.iter().map(|text| text.map(str::to_owned)).collect()
    }

    #[test]
    fn puts_id_first_and_adds_later_fields_at_the_right() {
        let mut flattened = Flattened::default();
        let change = flattened.add_page(&page(&[
            r#"{"name": "Ada", "_id": {"$oid": "5f1d7f0e2b5c3a0001a1b2c3"}, "total": {"$numberLong": "7"}}"#,
            r#"{"_id": 2, "name": null}"#,
        ]));
        assert_eq!(change, ColumnsChange::Appended);
        assert_eq!(flattened.column_names(), ["_id", "name", "total"]);

        let change = flattened.add_page(&page(&[r#"{"_id": 3, "city": "Leeds"}"#]));
        assert_eq!(change, ColumnsChange::Appended);
        assert_eq!(flattened.column_names(), ["_id", "name", "total", "city"]);
        assert_eq!(
            texts(&flattened),
            [
                cells(&[
                    Some(r#"ObjectId("5f1d7f0e2b5c3a0001a1b2c3")"#),
                    Some("Ada"),
                    Some("NumberLong(7)"),
                    None
                ]),
                cells(&[Some("2"), None, None, None]),
                cells(&[Some("3"), None, None, Some("Leeds")]),
            ]
        );
        assert_eq!(
            flattened.add_page(&page(&[r#"{"_id": 4}"#])),
            ColumnsChange::Unchanged
        );
    }

    #[test]
    fn tells_a_missing_field_from_null() {
        let mut flattened = Flattened::default();
        flattened.add_page(&page(&[r#"{"_id": 1, "note": null}"#, r#"{"_id": 2}"#]));
        assert_eq!(flattened.cell(0, 1), FieldCell::Value(&Value::Null));
        assert_eq!(flattened.cell(1, 1), FieldCell::Missing);
        assert_eq!(flattened.rows(1), [cells(&[Some("2"), None])]);
    }

    #[test]
    fn id_arriving_late_goes_first_and_says_so() {
        let mut flattened = Flattened::default();
        flattened.add_page(&page(&[r#"{"name": "Ada"}"#]));
        let change = flattened.add_page(&page(&[r#"{"_id": 1, "name": "Bo"}"#]));
        assert_eq!(change, ColumnsChange::Rearranged);
        assert_eq!(flattened.column_names(), ["_id", "name"]);
    }

    #[test]
    fn copies_objects_and_arrays_as_json_and_expands_objects_in_place() {
        let mut flattened = Flattened::default();
        flattened.add_page(&page(&[
            r#"{"_id": 1, "address": {"city": "Leeds", "zip": "LS1"}, "tags": ["a", "b"], "at": {"$date": {"$numberLong": "0"}}}"#,
            r#"{"_id": 2, "address": {"city": "York", "country": "UK"}}"#,
            r#"{"_id": 3, "address": "unknown"}"#,
        ]));
        assert_eq!(flattened.column_names(), ["_id", "address", "tags", "at"]);
        assert_eq!(
            flattened.rows(0)[0],
            cells(&[
                Some("1"),
                Some(r#"{"city":"Leeds","zip":"LS1"}"#),
                Some(r#"["a","b"]"#),
                Some(r#"ISODate("1970-01-01T00:00:00.000Z")"#),
            ])
        );
        assert!(flattened.can_expand(1));
        assert!(!flattened.can_expand(2), "an array isn't expanded");
        assert!(!flattened.can_expand(3), "a date is one value");

        flattened.expand(1);
        // The third document's address is text, so `address` stays, for it.
        assert_eq!(
            flattened.column_names(),
            [
                "_id",
                "address",
                "address.city",
                "address.zip",
                "address.country",
                "tags",
                "at"
            ]
        );
        assert_eq!(
            flattened.cell(2, 1),
            FieldCell::Value(&Value::String("unknown".to_owned()))
        );
        assert_eq!(flattened.cell(0, 1), FieldCell::Missing);
        assert_eq!(flattened.cell(2, 2), FieldCell::Missing);
        assert_eq!(
            flattened.rows(1)[0],
            cells(&[Some("2"), None, Some("York"), None, Some("UK"), None, None])
        );
        let address = Path::from(vec![Step::Field("address".to_owned())]);
        assert_eq!(flattened.expanded_from(2), Some(address.clone()));
        assert_eq!(flattened.expanded_from(1), None);
        assert!(!flattened.can_expand(1), "already expanded");
        assert!(!flattened.can_expand(2));

        flattened.add_page(&page(&[
            r#"{"_id": 4, "address": {"city": "Hull", "floor": 2}}"#,
        ]));
        assert_eq!(
            flattened.column_names().last().map(String::as_str),
            Some("address.floor")
        );

        flattened.collapse(&address);
        assert_eq!(flattened.column_names(), ["_id", "address", "tags", "at"]);
    }

    #[test]
    fn keeps_at_most_the_column_limit_and_counts_the_rest() {
        let fields: Vec<String> = (0..MAX_COLUMNS + 5)
            .map(|n| format!("\"f{n}\": {n}"))
            .collect();
        let document = format!("{{{}}}", fields.join(", "));
        let mut flattened = Flattened::default();
        flattened.add_page(&page(&[&document]));
        assert_eq!(flattened.column_names().len(), MAX_COLUMNS);
        assert_eq!(flattened.hidden_columns(), 5);
        assert_eq!(flattened.rows(0)[0].len(), MAX_COLUMNS);
    }

    #[test]
    fn shows_a_row_that_isnt_a_document_in_a_column_of_its_own() {
        let mut flattened = Flattened::default();
        let rows = vec![
            vec![Some(r#"{"_id": 1}"#.to_owned())],
            vec![Some("[1, 2]".to_owned())],
            vec![Some("{oops".to_owned())],
            vec![Some("a".to_owned()), Some("b".to_owned())],
        ];
        flattened.add_page(&rows);
        assert_eq!(flattened.column_names(), ["_id", UNREADABLE_COLUMN]);
        assert_eq!(
            flattened.rows(0),
            [
                cells(&[Some("1"), None]),
                cells(&[None, Some("[1, 2]")]),
                cells(&[None, Some("{oops")]),
                cells(&[None, Some("a\tb")]),
            ]
        );
        let reasons: Vec<&str> = flattened
            .documents()
            .iter()
            .filter_map(|document| match document {
                Document::Unreadable { reason, .. } => Some(reason.as_str()),
                Document::Read(_) => None,
            })
            .collect();
        assert_eq!(
            reasons,
            [
                "not an object",
                "not JSON: expected a field name at byte 1",
                "a row of 2 cells, not one"
            ]
        );
    }
}
