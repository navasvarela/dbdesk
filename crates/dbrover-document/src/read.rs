//! A JSON reader that keeps what `serde_json::Value` loses: numbers as written, and every field
//! in its order, a repeated name included (ADR 0006).

use crate::Value;

/// How deeply values may nest. MongoDB allows 100 levels in a document; past this, a value is
/// shown as text rather than as a tree. The tree is drawn recursively, a level a call, so the
/// bound also keeps drawing a deep value well within the UI thread's stack.
pub const MAX_DEPTH: usize = 200;

/// Why text isn't one JSON value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reason} at byte {offset}")]
pub struct ParseError {
    pub offset: usize,
    pub reason: &'static str,
}

pub(crate) fn parse(text: &str) -> Result<Value, ParseError> {
    let mut reader = Reader {
        text,
        bytes: text.as_bytes(),
        at: 0,
    };
    reader.skip_whitespace();
    let value = reader.value(0)?;
    reader.skip_whitespace();
    if reader.at < reader.bytes.len() {
        return Err(reader.error("more after the value"));
    }
    Ok(value)
}

struct Reader<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn error(&self, reason: &'static str) -> ParseError {
        ParseError {
            offset: self.at,
            reason,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        match self.peek() {
            Some(b'{') => self.object(depth + 1),
            Some(b'[') => self.array(depth + 1),
            Some(b'"') => self.string().map(Value::String),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b't') => self.word("true", Value::Bool(true)),
            Some(b'f') => self.word("false", Value::Bool(false)),
            Some(b'n') => self.word("null", Value::Null),
            Some(_) => Err(self.error("expected a value")),
            None => Err(self.error("expected a value, found the end")),
        }
    }

    fn word(&mut self, word: &str, value: Value) -> Result<Value, ParseError> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.error("expected a value"))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.error("nested too deeply"));
        }
        self.at += 1;
        let mut fields = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Value::Object(fields));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a field name"));
            }
            let name = self.string()?;
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err(self.error("expected `:`"));
            }
            self.at += 1;
            self.skip_whitespace();
            fields.push((name, self.value(depth)?));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Value::Object(fields));
                }
                _ => return Err(self.error("expected `,` or `}`")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.error("nested too deeply"));
        }
        self.at += 1;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.error("expected `,` or `]`")),
            }
        }
    }

    /// A number as JSON writes it: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(self.error("expected a digit")),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            self.required_digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            self.required_digits()?;
        }
        Ok(Value::Number(self.text[start..self.at].to_owned()))
    }

    fn digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
    }

    fn required_digits(&mut self) -> Result<(), ParseError> {
        let start = self.at;
        self.digits();
        if self.at == start {
            return Err(self.error("expected a digit"));
        }
        Ok(())
    }

    /// A string, its escapes resolved. Lenient where strict JSON gains nothing for a viewer: a
    /// control character is kept as it is, and a `\u` escape of half a surrogate pair becomes
    /// U+FFFD, as PostgreSQL's `json` type can hold one.
    fn string(&mut self) -> Result<String, ParseError> {
        self.at += 1;
        let mut text = String::new();
        loop {
            let run_start = self.at;
            while !matches!(self.peek(), Some(b'"' | b'\\') | None) {
                self.at += 1;
            }
            // The run ends at an ASCII `"` or `\`, or at the end, so it is whole characters.
            text.push_str(&self.text[run_start..self.at]);
            match self.peek() {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(text);
                }
                Some(b'\\') => {
                    self.at += 1;
                    self.escape(&mut text)?;
                }
                _ => return Err(self.error("unterminated string")),
            }
        }
    }

    fn escape(&mut self, text: &mut String) -> Result<(), ParseError> {
        let escaped = match self.peek() {
            Some(b'"') => '"',
            Some(b'\\') => '\\',
            Some(b'/') => '/',
            Some(b'b') => '\u{8}',
            Some(b'f') => '\u{c}',
            Some(b'n') => '\n',
            Some(b'r') => '\r',
            Some(b't') => '\t',
            Some(b'u') => {
                self.at += 1;
                text.push(self.unicode_escape()?);
                return Ok(());
            }
            _ => return Err(self.error("unknown escape")),
        };
        self.at += 1;
        text.push(escaped);
        Ok(())
    }

    /// The character of a `\uXXXX` escape, the `\u` already read, joining a surrogate pair
    /// written as two escapes.
    fn unicode_escape(&mut self) -> Result<char, ParseError> {
        let first = self.hex4()?;
        if !(0xD800..0xDC00).contains(&first) {
            return Ok(char::from_u32(first).unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        if !self.bytes[self.at..].starts_with(b"\\u") {
            return Ok(char::REPLACEMENT_CHARACTER);
        }
        let before_second = self.at;
        self.at += 2;
        let second = self.hex4()?;
        if !(0xDC00..0xE000).contains(&second) {
            self.at = before_second;
            return Ok(char::REPLACEMENT_CHARACTER);
        }
        let code = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
        Ok(char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER))
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let digits = self
            .text
            .get(self.at..self.at + 4)
            .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(|| self.error("expected four hex digits"))?;
        let code =
            u32::from_str_radix(digits, 16).map_err(|_| self.error("expected four hex digits"))?;
        self.at += 4;
        Ok(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(fields: &[(&str, Value)]) -> Value {
        Value::Object(
            fields
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone()))
                .collect(),
        )
    }

    fn number(text: &str) -> Value {
        Value::Number(text.to_owned())
    }

    #[test]
    fn keeps_fields_in_order_and_numbers_as_written() {
        let text =
            r#" {"b": 1.10, "a": [true, null, -0.5e+3], "b": 123456789012345678901234567890} "#;
        assert_eq!(
            parse(text),
            Ok(object(&[
                ("b", number("1.10")),
                (
                    "a",
                    Value::Array(vec![Value::Bool(true), Value::Null, number("-0.5e+3")])
                ),
                ("b", number("123456789012345678901234567890")),
            ]))
        );
    }

    #[test]
    fn resolves_escapes() {
        let cases = [
            (r#""a\"b\\c\/d""#, "a\"b\\c/d"),
            (r#""\b\f\n\r\t""#, "\u{8}\u{c}\n\r\t"),
            (r#""\u00e9t\u00E9""#, "été"),
            (r#""\ud83d\ude00""#, "😀"),
            (r#""lone \ud800 half""#, "lone \u{FFFD} half"),
            (r#""\ud800\u0041""#, "\u{FFFD}A"),
            ("\"ünïcödé\tkept\"", "ünïcödé\tkept"),
        ];
        for (text, expected) in cases {
            assert_eq!(
                parse(text),
                Ok(Value::String(expected.to_owned())),
                "{text}"
            );
        }
    }

    #[test]
    fn says_where_text_isnt_json() {
        let cases = [
            ("", 0, "expected a value, found the end"),
            ("{\"a\" 1}", 5, "expected `:`"),
            ("[1 2]", 3, "expected `,` or `]`"),
            ("{a: 1}", 1, "expected a field name"),
            ("01", 1, "more after the value"),
            ("1.", 2, "expected a digit"),
            ("-", 1, "expected a digit"),
            ("\"open", 5, "unterminated string"),
            ("\"\\x\"", 2, "unknown escape"),
            ("\"\\u12\"", 3, "expected four hex digits"),
            ("tru", 0, "expected a value"),
            ("{} {}", 3, "more after the value"),
        ];
        for (text, offset, reason) in cases {
            assert_eq!(parse(text), Err(ParseError { offset, reason }), "{text}");
        }
    }

    #[test]
    fn refuses_values_nested_past_the_limit() {
        let nested = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        assert!(parse(&nested(MAX_DEPTH)).is_ok());
        assert_eq!(
            parse(&nested(MAX_DEPTH + 1)).map_err(|error| error.reason),
            Err("nested too deeply")
        );
    }
}
