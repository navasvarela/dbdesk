//! BSON written back as the shell's syntax, for the commands DbRover writes itself: a
//! collection's definition, and the edits the grid makes. Each value keeps its type, written
//! with the constructor the parser reads (`NumberLong(7)`, `ISODate("…")`), so what DbRover
//! writes runs as it reads.

use std::fmt::Write as _;

use bson::spec::BinarySubtype;
use bson::{Bson, Document};
use dbrover_language::mongo::{is_plain_name, quote};

/// `value` as the shell writes it.
pub(crate) fn value(value: &Bson) -> String {
    let mut text = String::new();
    write_value(&mut text, value);
    text
}

/// `document` as the shell writes an object: `{status: "open", "a.b": 1}`.
pub(crate) fn document(document: &Document) -> String {
    let mut text = String::new();
    write_document(&mut text, document);
    text
}

fn write_document(text: &mut String, document: &Document) {
    if document.is_empty() {
        text.push_str("{}");
        return;
    }
    text.push('{');
    for (index, (key, value)) in document.iter().enumerate() {
        if index > 0 {
            text.push_str(", ");
        }
        if is_plain_name(key) {
            text.push_str(key);
        } else {
            text.push_str(&quote(key));
        }
        text.push_str(": ");
        write_value(text, value);
    }
    text.push('}');
}

fn write_value(text: &mut String, value: &Bson) {
    // Writing to a String can't fail.
    let _ = match value {
        Bson::Document(document) => {
            write_document(text, document);
            Ok(())
        }
        Bson::Array(items) => {
            text.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    text.push_str(", ");
                }
                write_value(text, item);
            }
            text.push(']');
            Ok(())
        }
        Bson::String(value) => write!(text, "{}", quote(value)),
        Bson::Boolean(value) => write!(text, "{value}"),
        Bson::Null | Bson::Undefined => write!(text, "null"),
        Bson::Int32(number) => write!(text, "{number}"),
        Bson::Int64(number) => write!(text, "NumberLong({})", quote(&number.to_string())),
        Bson::Double(number) => write!(text, "{}", double(*number)),
        Bson::Decimal128(number) => write!(text, "NumberDecimal({})", quote(&number.to_string())),
        Bson::ObjectId(id) => write!(text, "ObjectId({})", quote(&id.to_hex())),
        Bson::DateTime(date) => match date.try_to_rfc3339_string() {
            Ok(iso) => write!(text, "ISODate({})", quote(&iso)),
            Err(_) => write!(text, "new Date({})", date.timestamp_millis()),
        },
        Bson::Binary(binary) => match binary.to_uuid() {
            Ok(uuid) if binary.subtype == BinarySubtype::Uuid => {
                write!(text, "UUID({})", quote(&uuid.to_string()))
            }
            _ => write!(
                text,
                "BinData({}, {})",
                u8::from(binary.subtype),
                quote(&data_encoding::BASE64.encode(&binary.bytes))
            ),
        },
        Bson::Timestamp(timestamp) => {
            write!(
                text,
                "Timestamp({}, {})",
                timestamp.time, timestamp.increment
            )
        }
        Bson::RegularExpression(regex) => write!(text, "/{}/{}", regex.pattern, regex.options),
        Bson::MinKey => write!(text, "MinKey"),
        Bson::MaxKey => write!(text, "MaxKey"),
        other => write!(text, "{}", other.clone().into_canonical_extjson()),
    };
}

/// A double the shell reads back as one: a whole number keeps a fraction, since the shell would
/// store `1` as an `int`.
fn double(number: f64) -> String {
    if number.is_nan() {
        "NaN".to_owned()
    } else if number.is_infinite() {
        if number > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        }
        .to_owned()
    } else if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{number:.1}")
    } else {
        number.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;
    use dbrover_language::mongo::{Command, parse};

    use crate::convert::to_bson;

    /// What DbRover writes reads back as the same value, each type kept.
    #[test]
    fn writes_what_the_parser_reads_back_the_same() {
        let original = doc! {
            "plain": "it's \"quoted\"",
            "a.b": 1,
            "$set": { "n": 1.0, "big": 9_007_199_254_740_993_i64, "half": 0.5 },
            "id": bson::oid::ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap(),
            "at": bson::DateTime::from_millis(1_793_439_000_123),
            "dec": "1.10".parse::<bson::Decimal128>().unwrap(),
            "uuid": bson::Binary::from_uuid(bson::Uuid::parse_str("0e3e1d4b-6c5d-4f6e-8a1b-2c3d4e5f6a7b").unwrap()),
            "bin": bson::Binary { subtype: BinarySubtype::Generic, bytes: vec![1, 2, 3] },
            "ts": bson::Timestamp { time: 1, increment: 2 },
            "re": bson::Regex { pattern: "^a".try_into().unwrap(), options: "i".try_into().unwrap() },
            "list": [Bson::Null, true, Bson::MinKey, Bson::MaxKey],
        };
        let written = document(&original);
        let command = format!("db.c.insertOne({written})");
        let Ok(Command::Call(call)) = parse(&command) else {
            panic!("{command} doesn't parse");
        };
        assert_eq!(
            to_bson(&call.method.arguments[0]).unwrap(),
            Bson::Document(original),
            "{written}"
        );
    }
}
