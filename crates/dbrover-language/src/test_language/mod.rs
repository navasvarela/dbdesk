//! A language built only for tests, shaped like the key-value commands N1 brings: one command
//! per line, a verb and its arguments, `"…"` strings and `#` comments. Its verbs cover every
//! [`StatementKind`], so a test can drive each safety check without SQL.

#[cfg(test)]
mod tests;

use std::ops::Range;

use dbrover_sql::{StatementKind, Token, TokenKind, Unclassified};

/// Every verb with the kind it is classified as, sorted by verb.
const VERBS: [(&str, StatementKind); 15] = [
    ("begin", StatementKind::Transaction),
    ("commit", StatementKind::Transaction),
    ("config", StatementKind::Admin),
    ("create", StatementKind::Schema),
    ("del", StatementKind::Write),
    ("drop", StatementKind::Schema),
    ("exists", StatementKind::Read),
    ("flush", StatementKind::Admin),
    ("get", StatementKind::Read),
    ("incr", StatementKind::Write),
    ("keys", StatementKind::Read),
    ("ping", StatementKind::Read),
    ("rollback", StatementKind::Transaction),
    ("scan", StatementKind::Read),
    ("set", StatementKind::Write),
];

/// The verbs alone, sorted, for completion.
pub(crate) const KEYWORDS: &[&str] = &{
    let mut words = [""; VERBS.len()];
    let mut index = 0;
    while index < VERBS.len() {
        words[index] = VERBS[index].0;
        index += 1;
    }
    words
};

pub(crate) fn is_keyword(word: &str) -> bool {
    verb_kind(word).is_some()
}

fn verb_kind(word: &str) -> Option<StatementKind> {
    let word = word.to_ascii_lowercase();
    VERBS
        .binary_search_by(|(verb, _)| verb.cmp(&word.as_str()))
        .ok()
        .map(|index| VERBS[index].1)
}

/// Splits `text` into tokens covering every byte. A string or a comment ends at the end of its
/// line at the latest, since a command never spans lines.
pub(crate) fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut rest = text;
    while let Some(first) = rest.chars().next() {
        let length = match first {
            '#' => rest.find('\n').unwrap_or(rest.len()),
            '"' => string_length(rest),
            c if c.is_whitespace() => rest
                .find(|c: char| !c.is_whitespace())
                .unwrap_or(rest.len()),
            _ => rest
                .find(|c: char| c.is_whitespace() || c == '"' || c == '#')
                .unwrap_or(rest.len()),
        };
        let start = text.len() - rest.len();
        let fragment = &rest[..length];
        let kind = match first {
            '#' => TokenKind::LineComment,
            '"' => TokenKind::String,
            c if c.is_whitespace() => TokenKind::Whitespace,
            _ if fragment.bytes().all(|byte| byte.is_ascii_digit()) => TokenKind::Number,
            _ => TokenKind::Word,
        };
        tokens.push(Token {
            kind,
            range: start..start + length,
        });
        rest = &rest[length..];
    }
    tokens
}

/// The length of the string `text` starts with: up to its closing quote, a backslash escaping
/// the character after it, or to the end of the line when it isn't closed.
fn string_length(text: &str) -> usize {
    let mut escaped = false;
    for (index, c) in text.char_indices().skip(1) {
        match c {
            '\n' => return index,
            '"' if !escaped => return index + 1,
            '\\' => escaped = !escaped,
            _ => escaped = false,
        }
    }
    text.len()
}

/// One range per line with a command on it, from its first to its last meaningful token.
pub(crate) fn split(script: &str) -> Vec<Range<usize>> {
    let mut commands: Vec<Range<usize>> = Vec::new();
    let mut current: Option<Range<usize>> = None;
    for token in tokenize(script) {
        if token.kind == TokenKind::Whitespace && script[token.range.clone()].contains('\n') {
            commands.extend(current.take());
        } else if !token.kind.is_trivia() {
            current = Some(match current {
                Some(command) => command.start..token.range.end,
                None => token.range,
            });
        }
    }
    commands.extend(current);
    commands
}

/// The verbs that touch every key, as Redis's `FLUSHDB` and `KEYS` do, with why.
const HAZARDS: [(&str, &str); 2] = [
    ("flush", "FLUSH removes every key."),
    (
        "keys",
        "KEYS reads every key, and the server does nothing else until it has.",
    ),
];

pub(crate) fn hazard(command: &str) -> Option<&'static str> {
    let verb = verb(command)?;
    HAZARDS
        .iter()
        .find(|(hazardous, _)| verb.eq_ignore_ascii_case(hazardous))
        .map(|(_, description)| *description)
}

/// The command's first word, when it starts with one.
fn verb(command: &str) -> Option<&str> {
    tokenize(command)
        .into_iter()
        .find(|token| !token.kind.is_trivia())
        .filter(|token| token.kind == TokenKind::Word)
        .map(|token| &command[token.range])
}

/// Classifies one command by its verb. An unknown verb, or a command that doesn't start with
/// one, is unclassified.
pub(crate) fn classify(command: &str) -> Result<StatementKind, Unclassified> {
    let range = match split(command).as_slice() {
        [] => return Err(Unclassified::Empty),
        [range] => range.clone(),
        several => return Err(Unclassified::Several(several.len())),
    };
    let Some(verb) = verb(&command[range]) else {
        return Err(Unclassified::Unparseable(
            "a command starts with its verb".to_owned(),
        ));
    };
    verb_kind(verb).ok_or_else(|| Unclassified::Unparseable(format!("unknown command `{verb}`")))
}
