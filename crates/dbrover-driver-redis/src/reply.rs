//! A reply as the grid shows it, by its shape (`docs/nosql.md`, N1): an array as one column, a
//! map as field and value, a number or text as one cell, a stream's entries as documents, and a
//! value nested deeper as JSON text, which the cell viewer shows as a tree (N0b). Values are
//! text, as every engine's are (ADR 0004).

use dbrover_core::{Cell, ColumnInfo, ResultShape, Row};
use redis::Value;
use serde_json::{Map, Value as Json};

/// The columns and rows of one reply.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Table {
    pub(crate) columns: Vec<ColumnInfo>,
    pub(crate) rows: Vec<Row>,
    pub(crate) shape: ResultShape,
}

impl Table {
    fn new(columns: &[&str], rows: Vec<Row>) -> Self {
        Self {
            columns: columns.iter().map(|name| column(name)).collect(),
            rows,
            shape: ResultShape::Rows,
        }
    }
}

pub(crate) fn column(name: &str) -> ColumnInfo {
    ColumnInfo {
        name: name.to_owned(),
        type_name: None,
    }
}

/// The column for a reply of one value, or of values one per row.
const VALUE: &str = "value";

/// The reply of the command `name` (lowercase) as a table. An error the server sent as the
/// whole reply never gets here: it fails the command instead.
pub(crate) fn table(name: &str, reply: Value) -> Table {
    match reply {
        Value::Attribute { data, .. } => table(name, *data),
        Value::Map(pairs) => Table::new(
            &["field", VALUE],
            pairs
                .into_iter()
                .map(|(field, value)| vec![cell(field), cell(value)])
                .collect(),
        ),
        Value::Array(items) | Value::Set(items) | Value::Push { data: items, .. } => {
            items_table(name, items)
        }
        scalar => Table::new(&[VALUE], vec![vec![cell(scalar)]]),
    }
}

fn items_table(name: &str, items: Vec<Value>) -> Table {
    if matches!(name, "xrange" | "xrevrange")
        && let Some(documents) = stream_entries(&items)
    {
        return Table {
            columns: vec![column("document")],
            rows: documents
                .into_iter()
                .map(|document| vec![Some(document)])
                .collect(),
            shape: ResultShape::Documents,
        };
    }
    if let Some(width) = tuple_width(&items) {
        let names = tuple_columns(name, width);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let rows = items
            .into_iter()
            .map(|item| match item {
                Value::Array(values) | Value::Set(values) => values.into_iter().map(cell).collect(),
                other => vec![cell(other)],
            })
            .collect();
        return Table::new(&names, rows);
    }
    Table::new(
        &[VALUE],
        items.into_iter().map(|item| vec![cell(item)]).collect(),
    )
}

/// The width shared by `items` when every one is an array of at least two values, none of them
/// nested: `ZRANGE … WITHSCORES` answers so in RESP3, a member and its score each.
fn tuple_width(items: &[Value]) -> Option<usize> {
    let mut width = None;
    for item in items {
        let (Value::Array(values) | Value::Set(values)) = item else {
            return None;
        };
        if values.len() < 2
            || values.iter().any(is_nested)
            || width.is_some_and(|w| w != values.len())
        {
            return None;
        }
        width = Some(values.len());
    }
    width
}

/// What a tuple's values are, where the command says: a member and its score, a field and its
/// value, a position. Numbered otherwise.
fn tuple_columns(name: &str, width: usize) -> Vec<String> {
    let known: &[&str] = match name {
        "zrange" | "zrangebyscore" | "zrevrange" | "zrevrangebyscore" | "zrandmember"
        | "zpopmin" | "zpopmax" | "zunion" | "zinter" | "zdiff" | "zscan" => &["member", "score"],
        "hrandfield" | "hscan" => &["field", VALUE],
        "geopos" => &["longitude", "latitude"],
        _ => &[],
    };
    if known.len() == width {
        known.iter().map(|&name| name.to_owned()).collect()
    } else {
        (1..=width).map(|index| index.to_string()).collect()
    }
}

/// A stream's entries as documents, `_id` first and then the entry's fields in order, as
/// `XRANGE` answers them: `[id, [field, value, …]]` each. `None` if they aren't so shaped.
fn stream_entries(items: &[Value]) -> Option<Vec<String>> {
    items
        .iter()
        .map(|item| {
            let Value::Array(entry) = item else {
                return None;
            };
            let [id, Value::Array(fields)] = entry.as_slice() else {
                return None;
            };
            let mut document = Map::new();
            document.insert("_id".to_owned(), json(id));
            for pair in fields.chunks(2) {
                let [field, value] = pair else {
                    return None;
                };
                document.insert(text(field)?, json(value));
            }
            Some(Json::Object(document).to_string())
        })
        .collect()
}

fn is_nested(value: &Value) -> bool {
    matches!(
        value,
        Value::Array(_)
            | Value::Set(_)
            | Value::Map(_)
            | Value::Push { .. }
            | Value::Attribute { .. }
    )
}

/// A value as one cell: text for a single value, JSON for one with others inside it, and
/// `None` for nil.
pub(crate) fn cell(value: Value) -> Cell {
    match value {
        Value::Nil => None,
        Value::Attribute { data, .. } => cell(*data),
        nested if is_nested(&nested) => Some(json(&nested).to_string()),
        scalar => text(&scalar),
    }
}

/// A single value's text, as `redis-cli` shows it: a string as it is, or quoted with `\x`
/// escapes when it isn't UTF-8; a number as the server wrote it. `None` for nil and for values
/// with others inside them.
pub(crate) fn text(value: &Value) -> Option<String> {
    Some(match value {
        Value::Int(number) => number.to_string(),
        Value::BulkString(bytes) => bytes_text(bytes),
        Value::SimpleString(text) => text.clone(),
        Value::Okay => "OK".to_owned(),
        Value::Double(number) => double_text(*number),
        Value::Boolean(boolean) => boolean.to_string(),
        Value::VerbatimString { text, .. } => text.clone(),
        Value::BigNumber(digits) => String::from_utf8_lossy(digits).into_owned(),
        Value::ServerError(error) => format!("(error) {}", error_text(error)),
        Value::Attribute { data, .. } => text(data)?,
        // Nil, values with others inside them, and kinds of value a later client may add.
        _ => return None,
    })
}

/// A string's text, or, when it isn't UTF-8, as `redis-cli` quotes it, with `\xHH` for each byte
/// that isn't printable, so binary values show without breaking the grid.
pub(crate) fn bytes_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => dbrover_language::redis::quote(bytes),
    }
}

/// A double as Redis writes one: `inf`, `-inf` and `nan` for the values that aren't numbers,
/// and an exponent for very large or very small ones rather than hundreds of digits.
fn double_text(number: f64) -> String {
    if number.is_nan() {
        "nan".to_owned()
    } else if number.is_infinite() {
        if number > 0.0 { "inf" } else { "-inf" }.to_owned()
    } else if number != 0.0 && !(1e-5..1e17).contains(&number.abs()) {
        format!("{number:e}")
    } else {
        number.to_string()
    }
}

pub(crate) fn error_text(error: &redis::ServerError) -> String {
    match error.details() {
        Some(details) => format!("{} {details}", error.code()),
        None => error.code().to_owned(),
    }
}

/// A value as JSON: arrays and sets as arrays, a map as an object when its keys are text (as an
/// array of pairs otherwise), numbers and booleans as themselves, and every string as a string.
fn json(value: &Value) -> Json {
    match value {
        Value::Nil => Json::Null,
        Value::Int(number) => Json::from(*number),
        Value::Double(number) => serde_json::Number::from_f64(*number)
            .map_or_else(|| Json::String(double_text(*number)), Json::Number),
        Value::Boolean(boolean) => Json::Bool(*boolean),
        Value::Array(items) | Value::Set(items) | Value::Push { data: items, .. } => {
            Json::Array(items.iter().map(json).collect())
        }
        Value::Map(pairs) => {
            let object: Option<Map<String, Json>> = pairs
                .iter()
                .map(|(key, value)| Some((text(key)?, json(value))))
                .collect();
            object.map_or_else(
                || {
                    Json::Array(
                        pairs
                            .iter()
                            .map(|(key, value)| Json::Array(vec![json(key), json(value)]))
                            .collect(),
                    )
                },
                Json::Object,
            )
        }
        Value::Attribute { data, .. } => json(data),
        scalar => text(scalar).map_or(Json::Null, Json::String),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bulk(text: &str) -> Value {
        Value::BulkString(text.as_bytes().to_vec())
    }

    fn names(table: &Table) -> Vec<&str> {
        table
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect()
    }

    fn texts(rows: &[Row]) -> Vec<Vec<Option<&str>>> {
        rows.iter()
            .map(|row| row.iter().map(Option::as_deref).collect())
            .collect()
    }

    #[test]
    fn a_single_value_is_one_cell() {
        let cases = [
            (bulk("Ada"), Some("Ada")),
            (Value::Int(-7), Some("-7")),
            (Value::Okay, Some("OK")),
            (Value::Nil, None),
            (Value::Double(1.5), Some("1.5")),
            (Value::Double(f64::INFINITY), Some("inf")),
            (Value::Boolean(true), Some("true")),
            (Value::BulkString(vec![0xff, b'a']), Some("\"\\xffa\"")),
        ];
        for (reply, expected) in cases {
            let table = table("get", reply);
            assert_eq!(names(&table), ["value"]);
            assert_eq!(texts(&table.rows), [vec![expected]]);
        }
    }

    #[test]
    fn an_array_is_one_column_and_a_map_two() {
        let list = table(
            "lrange",
            Value::Array(vec![bulk("a"), Value::Nil, Value::Int(3)]),
        );
        assert_eq!(names(&list), ["value"]);
        assert_eq!(
            texts(&list.rows),
            [vec![Some("a")], vec![None], vec![Some("3")]]
        );

        let hash = table(
            "hgetall",
            Value::Map(vec![
                (bulk("name"), bulk("Ada")),
                (bulk("city"), bulk("London")),
            ]),
        );
        assert_eq!(names(&hash), ["field", "value"]);
        assert_eq!(
            texts(&hash.rows),
            [
                vec![Some("name"), Some("Ada")],
                vec![Some("city"), Some("London")]
            ]
        );

        let empty = table("smembers", Value::Set(Vec::new()));
        assert_eq!(names(&empty), ["value"]);
        assert!(empty.rows.is_empty());
    }

    #[test]
    fn pairs_take_names_from_the_command() {
        let pairs = Value::Array(vec![
            Value::Array(vec![bulk("ada"), Value::Double(10.0)]),
            Value::Array(vec![bulk("bob"), Value::Double(7.5)]),
        ]);
        let scores = table("zrange", pairs.clone());
        assert_eq!(names(&scores), ["member", "score"]);
        assert_eq!(
            texts(&scores.rows),
            [
                vec![Some("ada"), Some("10")],
                vec![Some("bob"), Some("7.5")]
            ]
        );
        assert_eq!(names(&table("somethingelse", pairs)), ["1", "2"]);
    }

    #[test]
    fn nested_values_are_json() {
        let reply = Value::Array(vec![
            bulk("plain"),
            Value::Array(vec![
                bulk("a"),
                Value::Map(vec![(bulk("k"), Value::Int(1))]),
            ]),
        ]);
        let table = table("xread", reply);
        assert_eq!(names(&table), ["value"]);
        assert_eq!(
            texts(&table.rows),
            [vec![Some("plain")], vec![Some(r#"["a",{"k":1}]"#)]]
        );
    }

    #[test]
    fn a_streams_entries_are_documents_with_their_id_first() {
        let entry = |id: &str, fields: &[&str]| {
            Value::Array(vec![
                bulk(id),
                Value::Array(fields.iter().map(|field| bulk(field)).collect()),
            ])
        };
        let table = table(
            "xrevrange",
            Value::Array(vec![
                entry("2-0", &["kind", "logout"]),
                entry("1-0", &["kind", "login", "user", "ada"]),
            ]),
        );
        assert_eq!(table.shape, ResultShape::Documents);
        assert_eq!(names(&table), ["document"]);
        assert_eq!(
            texts(&table.rows),
            [
                vec![Some(r#"{"_id":"2-0","kind":"logout"}"#)],
                vec![Some(r#"{"_id":"1-0","kind":"login","user":"ada"}"#)],
            ]
        );
    }

    #[test]
    fn doubles_read_as_redis_writes_them() {
        assert_eq!(double_text(3.0), "3");
        assert_eq!(double_text(-0.25), "-0.25");
        assert_eq!(double_text(1e300), "1e300");
        assert_eq!(double_text(f64::NAN), "nan");
        assert_eq!(double_text(f64::NEG_INFINITY), "-inf");
    }
}
