//! The commands the grid's edits of a collection's documents become (`docs/nosql.md`, N2·6):
//! a changed field is `updateOne` by `_id` with `$set` on its path, a deleted document
//! `deleteOne` by `_id`, an added one `insertOne`. Each value is written in the shell's syntax
//! with the type it had, read from its canonical Extended JSON, so `NumberLong(7)` stays a
//! 64-bit integer when 8 is typed over it.

use dbrover_document::{Path, Step, Syntax, Value as Document, ValueType};

use super::parser::parse_value;
use super::syntax::Value;
use super::{Command, is_plain_name, parse, quote};

/// A `find` on one collection: the documents it returns are that collection's, by `_id`, so
/// the grid can change them a command at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionView {
    /// The database `getSiblingDB` named; `None` for the console's own.
    database: Option<String>,
    collection: String,
}

impl CollectionView {
    /// The collection `command` reads, when it is a `find` the grid can edit.
    pub fn of(command: &str) -> Option<Self> {
        let Ok(Command::Call(call)) = parse(command) else {
            return None;
        };
        if call.method.name != "find" || call.chained("explain").is_some() {
            return None;
        }
        Some(Self {
            database: call.database,
            collection: call.collection?,
        })
    }

    pub fn collection(&self) -> &str {
        &self.collection
    }

    /// `db.getCollection("…")`, through `getSiblingDB` for another database.
    fn target(&self) -> String {
        let database = match &self.database {
            Some(database) => format!("db.getSiblingDB({})", quote(database)),
            None => "db".to_owned(),
        };
        format!("{database}.getCollection({})", quote(&self.collection))
    }

    /// Sets the field at `path` of the document whose `_id` is `id` to `value`, written in the
    /// shell's syntax. A missing value is a removed field: `$unset`.
    pub fn update(&self, id: &str, path: &str, value: Option<&str>) -> String {
        let change = match value {
            Some(value) => format!("{{$set: {{{}: {value}}}}}", quote(path)),
            None => format!("{{$unset: {{{}: \"\"}}}}", quote(path)),
        };
        format!("{}.updateOne({{_id: {id}}}, {change})", self.target())
    }

    /// Deletes the document whose `_id` is `id`.
    pub fn delete(&self, id: &str) -> String {
        format!("{}.deleteOne({{_id: {id}}})", self.target())
    }

    /// Inserts `document`, written in the shell's syntax.
    pub fn insert(&self, document: &str) -> String {
        format!("{}.insertOne({document})", self.target())
    }
}

/// The `_id` of `document`, in the shell's syntax.
///
/// # Errors
///
/// The document has no `_id`, or one the shell can't write.
pub fn document_id(document: &Document) -> Result<String, String> {
    let Document::Object(fields) = document else {
        return Err("this row isn't a document".to_owned());
    };
    let id = fields
        .iter()
        .find(|(name, _)| name == "_id")
        .map(|(_, id)| id)
        .ok_or("this document has no _id to find it by")?;
    shell_value(id)
}

/// The path `$set` takes for a field the grid shows at `path`: its steps joined by dots,
/// `items.2.sku`.
///
/// # Errors
///
/// A field whose name holds a dot, which a dotted path can't reach.
pub fn update_path(path: &Path) -> Result<String, String> {
    let steps: Vec<String> = path
        .steps()
        .iter()
        .map(|step| match step {
            Step::Field(name) if name.contains('.') => Err(format!(
                "the field \u{201c}{name}\u{201d} has a dot in its name, which $set can't reach"
            )),
            Step::Field(name) => Ok(name.clone()),
            Step::Index(index) => Ok(index.to_string()),
        })
        .collect::<Result<_, _>>()?;
    Ok(steps.join("."))
}

/// `value`, read from canonical Extended JSON, as the shell writes it with its type:
/// `ObjectId("…")`, `NumberLong(7)`, `ISODate("…")`, a string in quotes.
///
/// # Errors
///
/// A type the shell's syntax DbRover reads can't write: JavaScript code, a symbol, `undefined`
/// or a DBPointer.
pub fn shell_value(value: &Document) -> Result<String, String> {
    let shown = value.shown(Syntax::ExtendedJson);
    match (shown.value_type, value) {
        (ValueType::String, Document::String(text)) => Ok(quote(text)),
        (ValueType::Object, Document::Object(fields)) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|(name, value)| {
                    let name = if is_plain_name(name) {
                        name.clone()
                    } else {
                        quote(name)
                    };
                    Ok(format!("{name}: {}", shell_value(value)?))
                })
                .collect::<Result<_, String>>()?;
            Ok(format!("{{{}}}", fields.join(", ")))
        }
        (ValueType::Array, Document::Array(items)) => {
            let items: Vec<String> = items.iter().map(shell_value).collect::<Result<_, _>>()?;
            Ok(format!("[{}]", items.join(", ")))
        }
        (ValueType::Code | ValueType::Symbol | ValueType::Undefined | ValueType::DbPointer, _) => {
            Err(format!(
                "a {} value can't be written in the shell's syntax DbRover reads",
                shown.value_type.name()
            ))
        }
        _ => Ok(shown.text),
    }
}

/// What `text`, typed over a field that held `old`, becomes in the shell's syntax, keeping the
/// old value's type: `8` over `NumberLong(7)` is `NumberLong("8")`, over a string `"8"`.
/// Text that is already a value of the shell's syntax, such as `ObjectId("…")` or `{a: 1}`, is
/// taken as written. `None` for text set to `NULL`. A field that was missing or `null` takes
/// the type the text reads as: a number, `true`, `false`, a constructor, or else a string.
///
/// # Errors
///
/// Text that can't be a value of the old type, such as `abc` over a number.
pub fn typed_value(old: Option<&Document>, text: Option<&str>) -> Result<String, String> {
    let Some(text) = text else {
        return Ok("null".to_owned());
    };
    let old_type = old.map(|old| old.shown(Syntax::ExtendedJson).value_type);
    if old_type == Some(ValueType::String) {
        return Ok(quote(text));
    }
    let written = text.trim();
    let parsed = parse_value(written).ok();
    let is_literal = |value: &Value| {
        matches!(
            value,
            Value::Construct { .. }
                | Value::Object(_)
                | Value::Array(_)
                | Value::Regex { .. }
                | Value::String(_)
        )
    };
    if parsed.as_ref().is_some_and(is_literal) {
        return Ok(written.to_owned());
    }
    let integer = || {
        let digits = written.strip_prefix('-').unwrap_or(written);
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    };
    let wrong = |what: &str| Err(format!("\u{201c}{written}\u{201d} isn't {what}"));
    match old_type {
        None | Some(ValueType::Null) => Ok(match parsed {
            Some(_) => written.to_owned(),
            None => quote(text),
        }),
        Some(ValueType::Boolean) => match written {
            "true" | "false" => Ok(written.to_owned()),
            _ => wrong("true or false"),
        },
        Some(ValueType::Int32) if integer() && written.parse::<i32>().is_ok() => {
            Ok(format!("NumberInt({written})"))
        }
        Some(ValueType::Int32) => wrong("a 32-bit integer"),
        Some(ValueType::Int64) if integer() && written.parse::<i64>().is_ok() => {
            Ok(format!("NumberLong({})", quote(written)))
        }
        Some(ValueType::Int64) => wrong("a 64-bit integer"),
        Some(ValueType::Double | ValueType::Number) => match double(written) {
            Some(number) => Ok(number),
            None => wrong("a number"),
        },
        Some(ValueType::Decimal128) => Ok(format!("NumberDecimal({})", quote(written))),
        Some(ValueType::ObjectId) => Ok(format!("ObjectId({})", quote(written))),
        Some(ValueType::Date) => Ok(format!("ISODate({})", quote(written))),
        Some(ValueType::Uuid) => Ok(format!("UUID({})", quote(written))),
        Some(other) => Err(format!(
            "write a {} in the shell's syntax, as it shows",
            other.name()
        )),
    }
}

/// A double as the shell reads it back as one: a whole number keeps a fraction, since the shell
/// stores `1` as an `int`.
fn double(text: &str) -> Option<String> {
    if matches!(text, "Infinity" | "-Infinity" | "NaN") {
        return Some(text.to_owned());
    }
    text.parse::<f64>().ok()?;
    Some(if text.contains(['.', 'e', 'E']) {
        text.to_owned()
    } else {
        format!("{text}.0")
    })
}
