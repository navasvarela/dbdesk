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
mod lexer;
mod methods;
mod parser;
mod syntax;
#[cfg(test)]
mod tests;
mod words;

use std::ops::Range;

use dbrover_sql::{StatementKind, Token, TokenKind, Unclassified};

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
