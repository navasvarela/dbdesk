//! SQL text utilities for the query console: a forgiving lexer, statement splitting,
//! statement-under-cursor, and a check for destructive statements.
//!
//! Everything works on text as typed, which is usually half-finished: nothing here parses
//! SQL or fails on invalid input. Unterminated strings and comments run to the end of the
//! text. All positions are byte offsets into the input.
//!
//! A hand-written lexer rather than `sqlparser`'s tokenizer: that one rejects unterminated
//! input and reports line/column positions, while the editor needs byte ranges for any text,
//! and the same tokens also drive syntax highlighting.

mod explain;
mod keywords;
mod lexer;
mod safety;
mod split;
mod transaction;

pub use explain::explain_prefix;
pub use keywords::is_keyword;
pub use lexer::{Dialect, Token, TokenKind, first_word, tokenize};
pub use safety::{Hazard, hazard};
pub use split::{split_statements, statement_at};
pub use transaction::controls_transactions;
