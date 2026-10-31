//! Values between the shell's syntax, BSON and the grid's text: a command's arguments become
//! BSON as the shell would send them, and what the server answers becomes canonical Extended
//! JSON (ADR 0006), or plain text for a scalar in a column of its own.

use std::str::FromStr;

use bson::spec::BinarySubtype;
use bson::{Binary, Bson, DateTime, Decimal128, Document, Regex, Timestamp, oid::ObjectId};
use dbrover_language::mongo::{Constructor, Value};

/// Why an argument can't be turned into BSON, though it parsed: a date or a number out of range,
/// or text that isn't what its constructor takes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub(crate) struct ValueError(pub(crate) String);

/// `value` as the shell sends it. A number that is an integer fitting 32 bits is an `int`, any
/// other a `double`, as the shell's JavaScript numbers are.
pub(crate) fn to_bson(value: &Value) -> Result<Bson, ValueError> {
    Ok(match value {
        Value::Null | Value::Undefined => Bson::Null,
        Value::Bool(value) => Bson::Boolean(*value),
        Value::Number(text) => number(text)?,
        Value::String(text) => Bson::String(text.clone()),
        Value::Regex { pattern, flags } => Bson::RegularExpression(Regex {
            pattern: cstring(pattern)?,
            options: cstring(&sorted(flags))?,
        }),
        Value::Array(items) => Bson::Array(items.iter().map(to_bson).collect::<Result<_, _>>()?),
        Value::Object(fields) => Bson::Document(to_document(fields)?),
        Value::Construct {
            constructor,
            arguments,
        } => construct(*constructor, arguments)?,
    })
}

/// An object's fields as a document, in the order written.
pub(crate) fn to_document(fields: &[(String, Value)]) -> Result<Document, ValueError> {
    let mut document = Document::new();
    for (key, value) in fields {
        document.insert(key.clone(), to_bson(value)?);
    }
    Ok(document)
}

/// `value`, which must be an object, as a document; `what` names it in the error.
pub(crate) fn document_argument(value: &Value, what: &str) -> Result<Document, ValueError> {
    match value.as_object() {
        Some(fields) => to_document(fields),
        None => Err(ValueError(format!("{what} must be an object, {{…}}"))),
    }
}

fn cstring(text: &str) -> Result<bson::raw::CString, ValueError> {
    bson::raw::CString::try_from(text).map_err(|error| ValueError(error.to_string()))
}

/// A regular expression's flags in the order BSON requires them.
fn sorted(flags: &str) -> String {
    let mut flags: Vec<char> = flags.chars().collect();
    flags.sort_unstable();
    flags.into_iter().collect()
}

fn number(text: &str) -> Result<Bson, ValueError> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, text),
    };
    let sign = if negative { -1.0 } else { 1.0 };
    let magnitude = match digits {
        "Infinity" => return Ok(Bson::Double(sign * f64::INFINITY)),
        "NaN" => return Ok(Bson::Double(f64::NAN)),
        hex if hex.starts_with("0x") || hex.starts_with("0X") => {
            let value = i64::from_str_radix(&hex[2..], 16)
                .map_err(|_| ValueError(format!("{text} is too large")))?;
            return Ok(integer(if negative { -value } else { value }));
        }
        decimal => decimal,
    };
    let is_integer = magnitude.bytes().all(|byte| byte.is_ascii_digit());
    if is_integer && let Ok(value) = text.parse::<i64>() {
        return Ok(integer(value));
    }
    text.parse::<f64>()
        .map(Bson::Double)
        .map_err(|_| ValueError(format!("{text} isn't a number")))
}

/// A whole number as the shell stores it: an `int` if it fits 32 bits, otherwise a `double`.
fn integer(value: i64) -> Bson {
    match i32::try_from(value) {
        Ok(value) => Bson::Int32(value),
        // The shell's numbers are doubles; past 2^53 they lose digits, as the shell's do.
        #[allow(clippy::cast_precision_loss)]
        Err(_) => Bson::Double(value as f64),
    }
}

fn construct(constructor: Constructor, arguments: &[Value]) -> Result<Bson, ValueError> {
    let invalid = |what: &str| ValueError(format!("{}(): {what}", constructor.name()));
    Ok(match (constructor, arguments) {
        (Constructor::ObjectId, []) => Bson::ObjectId(ObjectId::new()),
        (Constructor::ObjectId, [Value::String(hex)]) => Bson::ObjectId(
            ObjectId::parse_str(hex).map_err(|_| invalid("not 24 hexadecimal digits"))?,
        ),
        (Constructor::Date, []) => Bson::DateTime(DateTime::now()),
        (Constructor::Date, [Value::String(text)]) => {
            Bson::DateTime(date(text).ok_or_else(|| {
                invalid("write the date as ISO 8601 text, such as 2026-10-31T09:30:00Z")
            })?)
        }
        (Constructor::Date, [Value::Number(millis)]) => Bson::DateTime(DateTime::from_millis(
            millis
                .parse::<i64>()
                .map_err(|_| invalid("milliseconds must be a whole number"))?,
        )),
        (Constructor::NumberLong, [Value::String(text) | Value::Number(text)]) => {
            Bson::Int64(text.parse().map_err(|_| invalid("not a 64-bit integer"))?)
        }
        (Constructor::NumberInt, [Value::String(text) | Value::Number(text)]) => {
            Bson::Int32(text.parse().map_err(|_| invalid("not a 32-bit integer"))?)
        }
        (Constructor::NumberDecimal, [Value::String(text) | Value::Number(text)]) => {
            Bson::Decimal128(Decimal128::from_str(text).map_err(|_| invalid("not a decimal"))?)
        }
        (Constructor::Uuid, []) => Bson::Binary(Binary::from_uuid(bson::Uuid::new())),
        (Constructor::Uuid, [Value::String(text)]) => Bson::Binary(Binary::from_uuid(
            bson::Uuid::parse_str(text).map_err(|_| invalid("not a UUID"))?,
        )),
        (Constructor::BinData, [Value::Number(subtype), Value::String(base64)]) => {
            let subtype: u8 = subtype
                .parse()
                .map_err(|_| invalid("the subtype must be a number from 0 to 255"))?;
            Bson::Binary(
                Binary::from_base64(base64, BinarySubtype::from(subtype))
                    .map_err(|_| invalid("not base64"))?,
            )
        }
        (Constructor::Timestamp, []) => Bson::Timestamp(Timestamp {
            time: 0,
            increment: 0,
        }),
        (Constructor::Timestamp, [Value::Number(time), Value::Number(increment)]) => {
            timestamp(time, increment).ok_or_else(|| invalid("seconds and an increment"))?
        }
        (Constructor::Timestamp, [Value::Object(fields)]) => {
            let field = |name: &str| {
                fields.iter().find_map(|(key, value)| match value {
                    Value::Number(text) if key == name => Some(text.as_str()),
                    _ => None,
                })
            };
            field("t")
                .zip(field("i"))
                .and_then(|(time, increment)| timestamp(time, increment))
                .ok_or_else(|| invalid("{t: seconds, i: increment}"))?
        }
        (Constructor::MinKey, []) => Bson::MinKey,
        (Constructor::MaxKey, []) => Bson::MaxKey,
        _ => return Err(invalid("arguments it doesn't take")),
    })
}

fn timestamp(time: &str, increment: &str) -> Option<Bson> {
    Some(Bson::Timestamp(Timestamp {
        time: time.parse().ok()?,
        increment: increment.parse().ok()?,
    }))
}

/// A date's text as the shell reads it: ISO 8601 with a time zone, or without one as UTC, or a
/// date alone at midnight UTC.
fn date(text: &str) -> Option<DateTime> {
    let text = text.trim();
    if let Ok(date) = DateTime::parse_rfc3339_str(text) {
        return Some(date);
    }
    let candidates = [format!("{text}Z"), format!("{text}T00:00:00Z")];
    candidates
        .iter()
        .find_map(|candidate| DateTime::parse_rfc3339_str(candidate).ok())
}

/// A document as the grid receives it: canonical Extended JSON text (ADR 0006), with its fields
/// in the server's order.
pub(crate) fn document_text(document: Document) -> String {
    Bson::Document(document)
        .into_canonical_extjson()
        .to_string()
}

/// A value in a column of its own, as the shell shows it: text and numbers plainly, `null` as
/// SQL's `NULL`, and anything else as canonical Extended JSON, which the cell viewer shows with
/// its type.
pub(crate) fn cell(value: Bson) -> Option<String> {
    match value {
        Bson::Null | Bson::Undefined => None,
        Bson::String(text) => Some(text),
        Bson::Int32(number) => Some(number.to_string()),
        Bson::Int64(number) => Some(number.to_string()),
        Bson::Double(number) => Some(number.to_string()),
        Bson::Boolean(value) => Some(value.to_string()),
        other => Some(other.into_canonical_extjson().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbrover_language::mongo::{Command, parse};

    /// The first argument of `db.c.insertOne(…)` with `document` in it, as BSON.
    fn argument(document: &str) -> Bson {
        let command = format!("db.c.insertOne({document})");
        let Ok(Command::Call(call)) = parse(&command) else {
            panic!("{command} doesn't parse");
        };
        to_bson(&call.method.arguments[0]).unwrap_or_else(|error| panic!("{command}: {error}"))
    }

    fn canonical(document: &str) -> String {
        argument(document).into_canonical_extjson().to_string()
    }

    #[test]
    fn numbers_are_stored_as_the_shell_stores_them() {
        assert_eq!(
            canonical("{a: 1, b: -2147483648, c: 2147483648, d: 1.5, e: 1e3, f: 0x10}"),
            concat!(
                r#"{"a":{"$numberInt":"1"},"b":{"$numberInt":"-2147483648"},"#,
                r#""c":{"$numberDouble":"2147483648.0"},"d":{"$numberDouble":"1.5"},"#,
                r#""e":{"$numberDouble":"1000.0"},"f":{"$numberInt":"16"}}"#
            )
        );
    }

    /// Each constructor gives its BSON type, and canonical Extended JSON keeps the type, so a
    /// document read back from the grid's text is the same document.
    #[test]
    fn each_constructor_gives_its_type_and_survives_extended_json() {
        let document = argument(
            "{id: ObjectId('507f1f77bcf86cd799439011'), at: ISODate('2026-10-31T09:30:00.123Z'), \
             day: new Date('2026-10-31'), ms: Date(0), long: NumberLong('9007199254740993'), \
             int: NumberInt(7), dec: NumberDecimal('1.10'), uuid: UUID('0e3e1d4b-6c5d-4f6e-8a1b-2c3d4e5f6a7b'), \
             bin: BinData(0, 'AQID'), ts: Timestamp(1, 2), min: MinKey, max: MaxKey(), \
             re: /^a.b/mi, nothing: null, list: [true, 'x']}",
        );
        let text = document.clone().into_canonical_extjson();
        assert_eq!(Bson::try_from(text.clone()).ok(), Some(document));
        let shown = text.to_string();
        for expected in [
            r#""id":{"$oid":"507f1f77bcf86cd799439011"}"#,
            r#""at":{"$date":{"$numberLong":"1793439000123"}}"#,
            r#""day":{"$date":{"$numberLong":"1793404800000"}}"#,
            r#""ms":{"$date":{"$numberLong":"0"}}"#,
            r#""long":{"$numberLong":"9007199254740993"}"#,
            r#""int":{"$numberInt":"7"}"#,
            r#""dec":{"$numberDecimal":"1.10"}"#,
            r#""uuid":{"$binary":{"base64":"Dj4dS2xdT26KGyw9Tl9qew==","subType":"04"}}"#,
            r#""bin":{"$binary":{"base64":"AQID","subType":"00"}}"#,
            r#""ts":{"$timestamp":{"t":1,"i":2}}"#,
            r#""min":{"$minKey":1}"#,
            r#""max":{"$maxKey":1}"#,
            r#""re":{"$regularExpression":{"pattern":"^a.b","options":"im"}}"#,
        ] {
            assert!(shown.contains(expected), "{expected} not in {shown}");
        }
    }

    #[test]
    fn refuses_values_out_of_range() {
        let command = "db.c.insertOne({n: NumberInt(3000000000)})";
        let Ok(Command::Call(call)) = parse(command) else {
            panic!("{command} doesn't parse");
        };
        assert!(to_bson(&call.method.arguments[0]).is_err());
    }

    #[test]
    fn a_scalar_in_a_column_shows_plainly_and_anything_else_with_its_type() {
        assert_eq!(
            cell(Bson::String("open".to_owned())),
            Some("open".to_owned())
        );
        assert_eq!(cell(Bson::Int64(7)), Some("7".to_owned()));
        assert_eq!(cell(Bson::Null), None);
        assert_eq!(
            cell(Bson::ObjectId(
                ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap_or_default()
            )),
            Some(r#"{"$oid":"507f1f77bcf86cd799439011"}"#.to_owned())
        );
    }
}
