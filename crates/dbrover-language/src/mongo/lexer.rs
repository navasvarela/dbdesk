//! The shell's tokens: names, strings, numbers, regular expressions, punctuation and comments,
//! as JavaScript writes them. Every byte of the text is in exactly one token, so the editor can
//! highlight from them and the splitter and parser read the same ones.

use dbrover_sql::{Token, TokenKind};

/// Splits `text` into tokens covering every byte. A regular expression is a
/// [`TokenKind::String`] that starts with `/`; a string or regular expression that isn't closed
/// ends at the end of its line, and a block comment that isn't closed at the end of the text.
pub(crate) fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut start = 0;
    while let Some(first) = text[start..].chars().next() {
        let rest = &text[start..];
        let (kind, length) = match first {
            c if c.is_whitespace() => (
                TokenKind::Whitespace,
                rest.find(|c: char| !c.is_whitespace())
                    .unwrap_or(rest.len()),
            ),
            '/' if rest.starts_with("//") => (
                TokenKind::LineComment,
                rest.find('\n').unwrap_or(rest.len()),
            ),
            '/' if rest.starts_with("/*") => (
                TokenKind::BlockComment,
                rest[2..].find("*/").map_or(rest.len(), |end| end + 4),
            ),
            '/' if regex_may_start(text, &tokens) => (TokenKind::String, regex_length(rest)),
            '"' | '\'' => (TokenKind::String, quoted_length(rest, first)),
            '`' => (TokenKind::String, template_length(rest)),
            ';' => (TokenKind::Semicolon, 1),
            c if c.is_ascii_digit() => (TokenKind::Number, number_length(rest)),
            '.' if rest[1..].starts_with(|c: char| c.is_ascii_digit()) => {
                (TokenKind::Number, number_length(rest))
            }
            c if is_name_start(c) => (
                TokenKind::Word,
                rest.find(|c: char| !is_name_part(c)).unwrap_or(rest.len()),
            ),
            c => (TokenKind::Punctuation, c.len_utf8()),
        };
        tokens.push(Token {
            kind,
            range: start..start + length,
        });
        start += length;
    }
    tokens
}

pub(crate) fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

pub(crate) fn is_name_part(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Whether a `/` here starts a regular expression rather than dividing: JavaScript decides by
/// what comes before it. Nothing the shell's subset writes divides, so a `/` reads as a
/// regular expression after an opening bracket, a comma, a colon or nothing at all.
fn regex_may_start(text: &str, tokens: &[Token]) -> bool {
    let previous = tokens.iter().rev().find(|token| !token.kind.is_trivia());
    match previous {
        None => true,
        Some(token) => {
            token.kind == TokenKind::Punctuation
                && matches!(
                    &text[token.range.clone()],
                    "(" | "[" | "{" | "," | ":" | "="
                )
        }
    }
}

/// The length of the regular expression `text` starts with, its flags included: up to the
/// closing `/` outside a character class, or the end of the line.
fn regex_length(text: &str) -> usize {
    let mut escaped = false;
    let mut in_class = false;
    for (index, c) in text.char_indices().skip(1) {
        match c {
            '\n' => return index,
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => {
                let flags = text[index + 1..]
                    .find(|c: char| !c.is_ascii_alphabetic())
                    .unwrap_or(text.len() - index - 1);
                return index + 1 + flags;
            }
            _ => {}
        }
    }
    text.len()
}

/// The length of the string `text` starts with: up to its closing `quote`, or the end of the
/// line. A backslash escapes the next character, a line break included, as JavaScript allows.
fn quoted_length(text: &str, quote: char) -> usize {
    let mut escaped = false;
    for (index, c) in text.char_indices().skip(1) {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '\n' => return index,
            c if c == quote => return index + 1,
            _ => {}
        }
    }
    text.len()
}

/// A template string spans lines; it ends at its closing backtick or the end of the text.
fn template_length(text: &str) -> usize {
    let mut escaped = false;
    for (index, c) in text.char_indices().skip(1) {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '`' => return index + 1,
            _ => {}
        }
    }
    text.len()
}

/// The length of the number `text` starts with: hexadecimal (`0x1f`), or decimal with an
/// optional fraction and exponent (`1`, `.5`, `2.5e-3`). Letters and digits run on after it
/// are taken too, so `12abc` is one token the parser can refuse whole.
fn number_length(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut index = 0;
    if text.starts_with("0x") || text.starts_with("0X") {
        index = 2;
    } else {
        while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.') {
            index += 1;
        }
        if index < bytes.len() && matches!(bytes[index], b'e' | b'E') {
            index += 1;
            if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
                index += 1;
            }
        }
    }
    while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_') {
        index += 1;
    }
    index
}
