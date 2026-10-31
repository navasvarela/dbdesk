//! Redis's commands, as `redis-cli` reads them, and what each may change: the language of Redis,
//! Valkey and Dragonfly consoles (`docs/nosql.md`, N1).
//!
//! A command is one line: its name and arguments, separated by spaces. An argument is written
//! bare, or in `"…"` with backslash escapes (`\n`, `\"`, `\x41`), or in `'…'` where only `\'`
//! is an escape, as `redis-cli` splits its input (`sdssplitargs` in Redis's sources). A line
//! that starts with `#` is a comment, which `redis-cli` doesn't have, but no command starts
//! with one.
//!
//! What a command may change comes from the server's own flags for it, in [`commands`], a table
//! generated from Redis's and Valkey's command definitions so the check works before
//! connecting.

mod commands;
#[cfg(test)]
mod tests;
mod view;

use std::fmt;
use std::ops::Range;

use dbrover_sql::{StatementKind, Token, TokenKind, Unclassified};

pub use commands::{CommandSpec, Flag};
pub use view::{KeyView, ViewShape};

/// Why a command's arguments can't be read, as `redis-cli` would refuse them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArgumentsError {
    #[error("a quoted argument isn't closed")]
    Unclosed,
    #[error("a closing quote must be followed by a space or the end of the command")]
    TextAfterQuote,
}

/// Splits `text` into tokens covering every byte, for highlighting and splitting. A quoted
/// argument is a [`TokenKind::String`], ending at the end of its line at the latest, since a
/// command never spans lines.
pub(crate) fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut line_start = true;
    while let Some(first) = text[start..].chars().next() {
        let rest = &text[start..];
        let (kind, length) = match first {
            '#' if line_start => (
                TokenKind::LineComment,
                rest.find('\n').unwrap_or(rest.len()),
            ),
            '"' | '\'' => (TokenKind::String, quoted_length(rest, first)),
            c if c.is_whitespace() => (
                TokenKind::Whitespace,
                rest.find(|c: char| !c.is_whitespace())
                    .unwrap_or(rest.len()),
            ),
            _ => {
                let length = rest
                    .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                    .unwrap_or(rest.len());
                (word_kind(&rest[..length]), length)
            }
        };
        let fragment = &rest[..length];
        line_start = match kind {
            TokenKind::Whitespace => line_start || fragment.contains('\n'),
            _ => false,
        };
        tokens.push(Token {
            kind,
            range: start..start + length,
        });
        start += length;
    }
    tokens
}

fn word_kind(word: &str) -> TokenKind {
    let digits = word.strip_prefix('-').unwrap_or(word);
    if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
        TokenKind::Number
    } else {
        TokenKind::Word
    }
}

/// The length of the quoted argument `text` starts with: up to its closing `quote`, or to the
/// end of the line when it isn't closed. In `"…"` a backslash escapes the next character; in
/// `'…'` only `\'` is an escape.
fn quoted_length(text: &str, quote: char) -> usize {
    let mut escaped = false;
    for (index, c) in text.char_indices().skip(1) {
        match c {
            '\n' => return index,
            c if c == quote && !escaped => return index + 1,
            '\\' if !escaped => escaped = quote == '"' || text[index + 1..].starts_with('\''),
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

/// The arguments of `command`, one line, as `redis-cli` sends them, the command's name first.
/// `"…"` decodes `\xHH`, `\n`, `\r`, `\t`, `\b` and `\a`, and any other escaped character as
/// itself; `'…'` decodes only `\'`. A quote may start in the middle of an argument, which goes
/// on after it closes only after a space, as in `redis-cli`.
///
/// # Errors
///
/// [`ArgumentsError`] for a quote that isn't closed, or is followed by more text.
pub fn arguments(command: &str) -> Result<Vec<Vec<u8>>, ArgumentsError> {
    let mut arguments = Vec::new();
    let mut chars = command.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        if chars.peek().is_none() {
            return Ok(arguments);
        }
        let mut argument = Vec::new();
        while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
            match c {
                '"' => read_double_quoted(&mut chars, &mut argument)?,
                '\'' => read_single_quoted(&mut chars, &mut argument)?,
                c => push_char(&mut argument, c),
            }
            if matches!(c, '"' | '\'') && chars.peek().is_some_and(|next| !next.is_whitespace()) {
                return Err(ArgumentsError::TextAfterQuote);
            }
        }
        arguments.push(argument);
    }
}

fn push_char(argument: &mut Vec<u8>, c: char) {
    argument.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
}

fn read_double_quoted(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    argument: &mut Vec<u8>,
) -> Result<(), ArgumentsError> {
    while let Some(c) = chars.next() {
        match c {
            '"' => return Ok(()),
            '\\' => {
                let escaped = chars.next().ok_or(ArgumentsError::Unclosed)?;
                match escaped {
                    'x' => match hex_byte(chars) {
                        Some(byte) => argument.push(byte),
                        None => push_char(argument, 'x'),
                    },
                    'n' => argument.push(b'\n'),
                    'r' => argument.push(b'\r'),
                    't' => argument.push(b'\t'),
                    'b' => argument.push(0x08),
                    'a' => argument.push(0x07),
                    other => push_char(argument, other),
                }
            }
            other => push_char(argument, other),
        }
    }
    Err(ArgumentsError::Unclosed)
}

/// The byte two hex digits at the front of `chars` spell, consumed only when both are there.
fn hex_byte(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<u8> {
    let mut lookahead = chars.clone();
    let high = lookahead.next()?.to_digit(16)?;
    let low = lookahead.next()?.to_digit(16)?;
    chars.next();
    chars.next();
    u8::try_from(high * 16 + low).ok()
}

fn read_single_quoted(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    argument: &mut Vec<u8>,
) -> Result<(), ArgumentsError> {
    while let Some(c) = chars.next() {
        match c {
            '\'' => return Ok(()),
            '\\' if chars.peek() == Some(&'\'') => {
                chars.next();
                argument.push(b'\'');
            }
            other => push_char(argument, other),
        }
    }
    Err(ArgumentsError::Unclosed)
}

/// `argument` written so [`arguments`] reads it back: bare when it is plain printable text,
/// otherwise in `"…"` with escapes, `\xHH` for bytes that aren't printable ASCII.
pub fn quote(argument: &[u8]) -> String {
    let plain = !argument.is_empty()
        && std::str::from_utf8(argument).is_ok_and(|text| {
            !text.starts_with('#')
                && text.chars().all(|c| {
                    !c.is_whitespace() && !c.is_control() && !matches!(c, '"' | '\'' | '\\')
                })
        });
    if plain {
        return String::from_utf8_lossy(argument).into_owned();
    }
    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('"');
    for chunk in argument.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '"' => quoted.push_str("\\\""),
                '\\' => quoted.push_str("\\\\"),
                '\n' => quoted.push_str("\\n"),
                '\r' => quoted.push_str("\\r"),
                '\t' => quoted.push_str("\\t"),
                c if c.is_control() => {
                    for byte in c.encode_utf8(&mut [0; 4]).bytes() {
                        quoted.push_str(&format!("\\x{byte:02x}"));
                    }
                }
                c => quoted.push(c),
            }
        }
        for byte in chunk.invalid() {
            quoted.push_str(&format!("\\x{byte:02x}"));
        }
    }
    quoted.push('"');
    quoted
}

/// A command's name as the table keys it: lowercase, and `container|subcommand` for a
/// subcommand such as `CONFIG GET`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandName(String);

impl CommandName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommandName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.replace('|', " ").to_uppercase())
    }
}

/// The command `arguments` call, with what the table knows of it: a subcommand's own entry
/// when the command is a container such as `CONFIG`. `None` for a name the table doesn't list.
pub fn command(arguments: &[Vec<u8>]) -> Option<(CommandName, &'static CommandSpec)> {
    let name = std::str::from_utf8(arguments.first()?)
        .ok()?
        .to_ascii_lowercase();
    let spec = commands::find(&name)?;
    let subcommand = arguments
        .get(1)
        .and_then(|argument| std::str::from_utf8(argument).ok())
        .map(|sub| format!("{name}|{}", sub.to_ascii_lowercase()))
        .and_then(|full| Some((full.clone(), commands::find(&full)?)));
    Some(match subcommand {
        Some((full, sub)) => (CommandName(full), sub),
        None => (CommandName(name), spec),
    })
}

/// Commands that change nothing the flags can see, but the session: whom it signs in as,
/// which database it uses, how the server or its clients behave. A read-only data source
/// must not let them through, whatever their flags say (`docs/nosql.md`, N1).
const ADMINISTRATION: &[&str] = &[
    "auth", "client", "config", "flushall", "flushdb", "hello", "reset", "select", "shutdown",
];

/// Transaction control: `MULTI` queues commands for `EXEC`, which runs them all; there is no
/// rollback.
const TRANSACTION: &[&str] = &["discard", "exec", "multi", "unwatch", "watch"];

/// Scripts and functions, which the server flags neither way: what they run is unknown, so
/// they count as writes. Their `_RO` forms are flagged `readonly`.
const SCRIPTS: &[&str] = &["eval", "evalsha", "fcall"];

/// Commands flagged neither `readonly` nor `write` that only read, or only wait: everything
/// else without either flag (publishing, subscribing, switching a cluster connection's mode,
/// a script's or a function's administration) counts as administration.
const READS_WITHOUT_FLAG: &[&str] = &[
    "acl|cat",
    "acl|whoami",
    "cluster|countkeysinslot",
    "cluster|getkeysinslot",
    "cluster|info",
    "cluster|keyslot",
    "cluster|links",
    "cluster|myid",
    "cluster|myshardid",
    "cluster|nodes",
    "cluster|shards",
    "cluster|slots",
    "command",
    "command|count",
    "command|docs",
    "command|getkeys",
    "command|getkeysandflags",
    "command|info",
    "command|list",
    "echo",
    "function|dump",
    "function|list",
    "function|stats",
    "info",
    "lastsave",
    "memory|doctor",
    "memory|malloc-stats",
    "memory|stats",
    "ping",
    "pubsub|channels",
    "pubsub|numpat",
    "pubsub|numsub",
    "pubsub|shardchannels",
    "pubsub|shardnumsub",
    "role",
    "script|exists",
    "time",
    "wait",
    "waitaof",
];

/// What the command `arguments` name may change, from its flags and the exceptions above.
fn kind_of(name: &CommandName, spec: &CommandSpec) -> StatementKind {
    let name = name.as_str();
    let top = name.split('|').next().unwrap_or(name);
    if ADMINISTRATION.contains(&top) {
        StatementKind::Admin
    } else if TRANSACTION.contains(&top) {
        StatementKind::Transaction
    } else if SCRIPTS.contains(&top) {
        StatementKind::Write
    } else if spec.has(Flag::Admin) {
        StatementKind::Admin
    } else if spec.has(Flag::Write) {
        StatementKind::Write
    } else if spec.has(Flag::Readonly)
        || READS_WITHOUT_FLAG.contains(&name)
        || name.ends_with("|help")
    {
        StatementKind::Read
    } else {
        StatementKind::Admin
    }
}

/// Classifies one command by the flags the server gives it.
///
/// # Errors
///
/// [`Unclassified`] for text without exactly one command, a command whose quotes don't close,
/// or a name the table doesn't list, such as a module's command it wasn't generated with.
pub(crate) fn classify(command: &str) -> Result<StatementKind, Unclassified> {
    let range = match split(command).as_slice() {
        [] => return Err(Unclassified::Empty),
        [range] => range.clone(),
        several => return Err(Unclassified::Several(several.len())),
    };
    let arguments =
        arguments(&command[range]).map_err(|error| Unclassified::Unparseable(error.to_string()))?;
    let (name, spec) = self::command(&arguments).ok_or_else(|| {
        let name = arguments
            .first()
            .map(|name| String::from_utf8_lossy(name).into_owned())
            .unwrap_or_default();
        Unclassified::Unparseable(format!("unknown command `{name}`"))
    })?;
    Ok(kind_of(&name, spec))
}

/// The commands that touch every key, with why.
const HAZARDS: [(&str, &str); 3] = [
    ("flushall", "FLUSHALL removes every key of every database."),
    ("flushdb", "FLUSHDB removes every key of the database."),
    (
        "keys",
        "KEYS reads every key, and the server does nothing else until it has.",
    ),
];

pub(crate) fn hazard(command: &str) -> Option<&'static str> {
    let arguments = arguments(command).ok()?;
    let name = std::str::from_utf8(arguments.first()?).ok()?;
    HAZARDS
        .iter()
        .find(|(hazardous, _)| name.eq_ignore_ascii_case(hazardous))
        .map(|(_, description)| *description)
}

/// The command names, lowercase and sorted, for completion; subcommands are left out.
pub(crate) fn completion_words() -> &'static [&'static str] {
    commands::NAMES
}

/// Whether `word` names a command, ignoring case, for highlighting.
pub(crate) fn is_command(word: &str) -> bool {
    !word.contains('|') && commands::find(&word.to_ascii_lowercase()).is_some()
}

/// The arguments' syntax of `word`, a command name, for completion to show beside it; `None`
/// for a command without arguments.
pub(crate) fn syntax(word: &str) -> Option<&'static str> {
    commands::find(&word.to_ascii_lowercase())
        .map(|spec| spec.syntax)
        .filter(|syntax| !syntax.is_empty())
}
