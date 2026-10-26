//! SQL text utilities for the query console: a forgiving lexer, statement splitting,
//! statement-under-cursor, a check for destructive statements, a formatter, a syntax
//! checker and linter, a classifier of what a statement may change, the DDL the object
//! helpers write, and the statements that change a table's rows from the grid.
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
mod classify;
mod ddl;
mod dialect;
mod dml;
mod format;
mod keywords;
mod lexer;
mod projection;
mod safety;
mod split;
mod transaction;

pub use check::{LintRule, Problem, ProblemSource, Severity, check_statement, is_checkable};
pub use classify::{
    Analysis, NamePart, NamedSchemas, StatementKind, Unclassified, analyze, classify,
};
pub use ddl::{
    Check, ColumnSpec, DdlError, DdlOperation, ForeignKey, IndexMethod, IndexSpec, ObjectName,
    ReferentialAction, SchemaSpec, TableSpec, UniqueKey, ViewKind, ViewSpec, ddl_statements,
    quote_name, view_query,
};
pub use dialect::{
    Capabilities, Dialect, ExplainSetting, Savepoint, SchemaCreation, TableChange, ViewReplacement,
};
pub use dml::{ColumnValue, RowSql, Table};
pub use format::{FormatError, FormatOptions, Formatted, KeywordCase, format_ranges, same_meaning};
pub use keywords::{is_keyword, keywords};
pub use lexer::{Token, TokenKind, first_word, tokenize};
pub use projection::{ProjectionItem, projection};
pub use safety::{Hazard, hazard};
pub use split::{split_statements, statement_at};
pub use transaction::{commits_implicitly, controls_transactions};
