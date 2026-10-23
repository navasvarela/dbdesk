//! Problems in a statement as typed: syntax errors from a real parser, and lint warnings for
//! statements that are valid but rarely what was meant.
//!
//! Everything here works on one statement at a time (as [`split_statements`] cuts them), with
//! ranges relative to it, so the caller can cache results per statement and only check the
//! ones that changed.
//!
//! [`split_statements`]: crate::split_statements

mod lint;
mod syntax;
#[cfg(test)]
mod tests;

use std::ops::Range;

use crate::lexer::{Dialect, TokenKind, first_word, tokenize};

pub use lint::LintRule;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
}

/// What found a problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProblemSource {
    /// The parser could not read the statement.
    Syntax,
    Lint(LintRule),
    /// The database rejected the statement when asked to prepare it.
    Database,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Byte range in the text the problem was found in.
    pub range: Range<usize>,
    pub severity: Severity,
    pub message: String,
    pub source: ProblemSource,
}

impl Problem {
    /// The same problem in a text where its statement starts at `offset`.
    pub fn shifted(mut self, offset: usize) -> Self {
        self.range = self.range.start + offset..self.range.end + offset;
        self
    }
}

/// First words of the statements that are parsed and can be prepared by the database: data
/// queries and changes. Other statements (DDL, PL/pgSQL `DO` blocks, `COPY`, vendor
/// statements) are where parsers fall short of the database, and a false error is worse than
/// a missed one, so they aren't checked.
const CHECKED_STATEMENTS: &[&str] = &["select", "with", "values", "insert", "update", "delete"];

/// Whether `statement` is a data query or change: the statements [`check_statement`] parses,
/// and the ones a database may be asked to prepare (preparing them runs nothing, unlike some
/// SQLite pragmas, which take effect while being prepared).
pub fn is_checkable(statement: &str, dialect: Dialect) -> bool {
    first_word(statement, dialect).is_some_and(|word| {
        CHECKED_STATEMENTS.contains(&word.as_str())
            || (dialect == Dialect::Sqlite && word == "replace")
    })
}

/// The syntax error, if any, and every lint warning in one statement, with ranges relative to
/// it. Lint rules aren't filtered here; see [`LintRule::id`] for switching them off.
pub fn check_statement(statement: &str, dialect: Dialect) -> Vec<Problem> {
    let mut problems: Vec<Problem> = syntax::problem(statement, dialect).into_iter().collect();
    problems.extend(lint::problems(statement, dialect));
    problems
}

/// The range to mark for a problem at byte `offset`: the token there (or the last one, for an
/// offset past the end), never whitespace.
fn token_at(statement: &str, dialect: Dialect, offset: usize) -> Range<usize> {
    let tokens: Vec<_> = tokenize(statement, dialect)
        .into_iter()
        .filter(|token| token.kind != TokenKind::Whitespace)
        .collect();
    tokens
        .iter()
        .find(|token| token.range.contains(&offset))
        .or_else(|| {
            tokens
                .iter()
                .rev()
                .find(|token| token.range.start <= offset)
        })
        .or(tokens.first())
        .map_or(0..statement.len(), |token| token.range.clone())
}
