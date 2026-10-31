//! Writing a value back as JSON, numbers as they were read.

use std::fmt::Write;

use crate::Value;

/// Spaces per level, as `serde_json`'s pretty printer and the exports indent.
const INDENT: usize = 2;

pub(crate) fn compact(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, None);
    out
}

pub(crate) fn pretty(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, Some(0));
    out
}

/// Writes `value` to `out`, indented at `level` when pretty, on one line when `None`.
fn write_value(out: &mut String, value: &Value, level: Option<usize>) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(text) => out.push_str(text),
        Value::String(text) => write_string(out, text),
        Value::Array(items) => {
            write_container(out, ('[', ']'), items, level, |out, item, level| {
                write_value(out, item, level);
            })
        }
        Value::Object(fields) => {
            write_container(
                out,
                ('{', '}'),
                fields,
                level,
                |out, (name, value), level| {
                    write_string(out, name);
                    out.push_str(if level.is_some() { ": " } else { ":" });
                    write_value(out, value, level);
                },
            );
        }
    }
}

fn write_container<T>(
    out: &mut String,
    (open, close): (char, char),
    entries: &[T],
    level: Option<usize>,
    write_entry: impl Fn(&mut String, &T, Option<usize>),
) {
    out.push(open);
    if entries.is_empty() {
        out.push(close);
        return;
    }
    let inner = level.map(|level| level + 1);
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        new_line(out, inner);
        write_entry(out, entry, inner);
    }
    new_line(out, level);
    out.push(close);
}

fn new_line(out: &mut String, level: Option<usize>) {
    if let Some(level) = level {
        out.push('\n');
        out.push_str(&" ".repeat(level * INDENT));
    }
}

/// A string in quotes, escaped as JSON requires and no more: `"`, `\` and control characters.
pub(crate) fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 => {
                // Writing to a String can't fail.
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_back_what_it_read() {
        let text = r#"{"b":1.10,"a":[true,null,{}],"e":[],"s":"q\"\\\n\u0001é"}"#;
        let value = Value::parse(text).unwrap();
        assert_eq!(compact(&value), text);
        assert_eq!(
            pretty(&value),
            "{\n  \"b\": 1.10,\n  \"a\": [\n    true,\n    null,\n    {}\n  ],\n  \"e\": [],\n  \
             \"s\": \"q\\\"\\\\\\n\\u0001é\"\n}"
        );
    }
}
