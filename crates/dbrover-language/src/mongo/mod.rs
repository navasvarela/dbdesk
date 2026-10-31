//! A subset of the MongoDB shell's syntax, read by DbRover itself with no JavaScript engine,
//! and what each command may change: the language of MongoDB consoles (`docs/nosql.md`, N2).
//!
//! A command is `use <database>`, `show dbs`, `show collections`, or a method called through
//! `db`: `db.orders.find({status: "open"}).sort({_id: -1}).limit(50)`. Its arguments are the
//! shell's relaxed JSON, with the constructors of BSON's types (`ObjectId("…")`,
//! `ISODate("…")`, `NumberLong(7)`) and regular expressions. Variables, functions and loops
//! aren't read, and the error says so.
//!
//! The parser reads a command into a [`Command`], DbRover's own tree, which the driver turns
//! into BSON, and which classification and the warnings read too.

mod classify;
mod edit;
mod lexer;
mod methods;
mod parser;
mod syntax;
#[cfg(test)]
mod tests;
mod words;

use std::ops::Range;

use dbrover_sql::{StatementKind, Token, TokenKind, Unclassified};

pub use edit::{CollectionView, document_id, shell_value, typed_value, update_path};
pub use parser::{ParseError, parse};
pub use syntax::{Call, Command, Constructor, MethodCall, Show, Value};

pub(crate) use lexer::tokenize;
pub(crate) use words::{completion_words, detail as syntax, is_keyword};

/// One range per command, from its first to its last meaningful token.
///
/// A command ends at a `;`, or at a line break once every bracket it opened is closed, unless
/// the line ends with `.` or the next line starts with one: the shell reads a chain written over
/// several lines (`db.orders\n  .find()\n  .limit(5)`) as one command.
pub(crate) fn split(script: &str) -> Vec<Range<usize>> {
    let tokens: Vec<Token> = tokenize(script);
    let meaningful: Vec<usize> = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| !token.kind.is_trivia())
        .map(|(index, _)| index)
        .collect();
    let text = |index: usize| &script[tokens[index].range.clone()];
    let mut commands = Vec::new();
    let mut current: Option<Range<usize>> = None;
    let mut depth: usize = 0;
    for (position, &index) in meaningful.iter().enumerate() {
        let token = &tokens[index];
        if token.kind == TokenKind::Semicolon && depth == 0 {
            commands.extend(current.take());
            continue;
        }
        if let Some(command) = &current
            && depth == 0
            && script[command.end..token.range.start].contains('\n')
            && text(index) != "."
            && position
                .checked_sub(1)
                .is_none_or(|previous| text(meaningful[previous]) != ".")
        {
            commands.extend(current.take());
        }
        if token.kind == TokenKind::Punctuation {
            match text(index) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        current = Some(match current {
            Some(command) => command.start..token.range.end,
            None => token.range.clone(),
        });
    }
    commands.extend(current);
    commands
}

/// Whether `name` is written bare in the shell, as a field's name in an object or a collection's
/// after `db.`: a JavaScript name, such as `orders` or `$match`.
pub fn is_plain_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(lexer::is_name_start) && chars.all(lexer::is_name_part)
}

/// `text` as a string the shell reads back the same, in double quotes.
pub fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            c if c.is_control() => quoted.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// What **Open data** runs on a collection: a `find` of every document of `collection` in
/// `database`, for a console whose database is `own`. Names are always passed as strings, so
/// one with a dot, a dash or a space reads the same.
pub fn find_all(database: &str, collection: &str, own: &str) -> String {
    let target = if database == own {
        "db".to_owned()
    } else {
        format!("db.getSiblingDB({})", quote(database))
    };
    format!("{target}.getCollection({}).find({{}})", quote(collection))
}

/// The cursor a `find` or an `aggregate` reads, rather than its plan: what `command` must be
/// for its result to be explained, or written straight to a file.
fn reads_a_cursor(command: &Command) -> bool {
    let Command::Call(call) = command else {
        return false;
    };
    matches!(call.method.name.as_str(), "find" | "aggregate") && call.chained("explain").is_none()
}

/// `command`, a `find` or an `aggregate`, with `.explain("executionStats")` after it, which
/// runs it and shows its plan with what each stage did; `None` for any other command.
pub fn explain(command: &str) -> Option<String> {
    let parsed = parse(command).ok()?;
    reads_a_cursor(&parsed).then(|| format!("{}.explain(\"executionStats\")", command.trim_end()))
}

/// Whether `command` returns documents a cursor reads: a `find` or an `aggregate`.
pub fn returns_documents(command: &str) -> bool {
    parse(command).is_ok_and(|parsed| reads_a_cursor(&parsed))
}

/// The one command `text` holds, read.
fn single(text: &str) -> Result<Command, Unclassified> {
    match split(text).as_slice() {
        [] => Err(Unclassified::Empty),
        [range] => {
            parse(&text[range.clone()]).map_err(|error| Unclassified::Unparseable(error.message))
        }
        several => Err(Unclassified::Several(several.len())),
    }
}

/// What `command` may change.
pub(crate) fn classify(command: &str) -> Result<StatementKind, Unclassified> {
    single(command).map(|command| classify::kind(&command))
}

/// Why `command` may touch everything, for the warning before it runs.
pub(crate) fn hazard(command: &str) -> Option<&'static str> {
    single(command).ok().as_ref().and_then(classify::hazard)
}
