//! SQL text utilities for the query console: a forgiving lexer, statement splitting,
//! statement-under-cursor, a check for destructive statements, a formatter, and a syntax
//! checker and linter.
//!
//! Everything works on text as typed, which is usually half-finished: nothing here fails on
//! invalid input. Unterminated strings and comments run to the end of the text. All
//! positions are byte offsets into the input. Only the syntax checker parses, with
//! `sqlparser`, one statement at a time.
//!
//! A hand-written lexer rather than `sqlparser`'s tokenizer: that one rejects unterminated
//! input and reports line/column positions, while the editor needs byte ranges for any text,
//! and the same tokens also drive syntax highlighting.

mod check;
mod explain;
mod format;
mod functions;
mod keywords;
mod lexer;
mod projection;
mod safety;
mod split;
mod transaction;

pub use check::{LintRule, Problem, ProblemSource, Severity, check_statement, is_checkable};
pub use explain::explain_prefix;
pub use format::{FormatError, FormatOptions, Formatted, KeywordCase, format_ranges, same_meaning};
pub use functions::builtin_functions;
pub use keywords::{is_keyword, keywords};
pub use lexer::{Dialect, Token, TokenKind, first_word, tokenize};
pub use projection::{ProjectionItem, projection};
pub use safety::{Hazard, hazard};
pub use split::{split_statements, statement_at};
pub use transaction::controls_transactions;
