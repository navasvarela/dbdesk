//! A SQL formatter built on the lexer: it only ever changes the whitespace between tokens and
//! the case of keywords, so it can't change what a statement does.
//!
//! Tokens are copied as they are, so strings, dollar-quoted bodies, quoted names and comments
//! come out byte for byte. What it decides is where lines break and how far they are indented
//! (see [`layout`]). Before handing anything back, [`format_ranges`] lexes its own output and
//! checks it against the input with [`same_meaning`]; a difference is a bug in the layout, and
//! the text is left alone rather than risk running something else.
//!
//! Written for dbdesk rather than taken from the `sqlformat` crate, which (in 0.5) changes
//! what some statements mean: it reformats dollar-quoted function bodies, puts spaces inside
//! string literals (`ESCAPE '\'` becomes `ESCAPE ' \ '`), splits prefixed literals
//! (`B'1010'`, `U&'…'`) and SQLite's `[bracketed names]`.

mod layout;
mod meaning;
#[cfg(test)]
mod tests;

use std::fmt;
use std::ops::Range;

use crate::dialect::Dialect;
use crate::keywords::is_keyword;
use crate::lexer::{Token, TokenKind, tokenize};

pub use meaning::same_meaning;

/// How keywords are written in formatted SQL. Names and everything else keep their case.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KeywordCase {
    #[default]
    Upper,
    Lower,
    /// As typed.
    Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatOptions {
    pub keyword_case: KeywordCase,
    /// Spaces per indentation level.
    pub indent_width: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            keyword_case: KeywordCase::Upper,
            indent_width: 4,
        }
    }
}

/// Formatting was refused because the output would not lex to the same tokens as the input.
/// Never expected; it means the layout has a bug, so the text is left as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError;

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("formatting would have changed the statement, so it was left as it was")
    }
}

impl std::error::Error for FormatError {}

/// Formatted text, and where each part of the original text went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formatted {
    text: String,
    /// Stretches copied unchanged (tokens, and the text outside the formatted ranges), in
    /// order. Offsets inside one map linearly; that holds for tokens whose keyword case
    /// changed too, since ASCII case changes keep the length.
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    source: Range<usize>,
    target: usize,
}

impl Formatted {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn into_text(self) -> String {
        self.text
    }

    /// Where byte `offset` of the original text is in the formatted text: on the same token,
    /// as far into it; in whitespace that was changed, just after the token before it.
    pub fn map_offset(&self, offset: usize) -> usize {
        let index = self
            .segments
            .partition_point(|segment| segment.source.start <= offset);
        let Some(segment) = index.checked_sub(1).and_then(|i| self.segments.get(i)) else {
            return 0;
        };
        let into = offset.min(segment.source.end) - segment.source.start;
        segment.target + into
    }
}

/// Formats each of `ranges` of `text` (sorted, not overlapping, starting and ending on token
/// boundaries, such as the ranges [`split_statements`](crate::split_statements) returns) and
/// copies the rest unchanged.
///
/// # Errors
///
/// [`FormatError`] if the result would lex differently from `text`; see [`same_meaning`].
pub fn format_ranges(
    text: &str,
    ranges: &[Range<usize>],
    dialect: Dialect,
    options: &FormatOptions,
) -> Result<Formatted, FormatError> {
    let mut out = Writer::new(options.indent_width);
    let mut copied_up_to = 0;
    for range in ranges {
        let Some(fragment) = text.get(range.clone()) else {
            continue;
        };
        if range.start < copied_up_to {
            continue;
        }
        out.copy(text, copied_up_to..range.start);
        let start = out.text.len();
        out.base_indent = line_indent(text, range.start).to_owned();
        out.line_level = 0;
        let items = items(text, range.clone(), dialect, options.keyword_case);
        layout::render(&items, &mut out);
        if !same_meaning(fragment, &out.text[start..], dialect) {
            return Err(FormatError);
        }
        copied_up_to = range.end;
    }
    out.copy(text, copied_up_to..text.len());
    Ok(Formatted {
        text: out.text,
        segments: out.segments,
    })
}

/// The spaces and tabs that start the line `offset` is on: continuation lines of a statement
/// that starts there are indented by that much too.
fn line_indent(text: &str, offset: usize) -> &str {
    let line_start = text[..offset].rfind('\n').map_or(0, |newline| newline + 1);
    let line = &text[line_start..offset];
    let end = line
        .find(|c: char| c != ' ' && c != '\t')
        .unwrap_or(line.len());
    &line[..end]
}

/// Whitespace in the original text before a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gap {
    None,
    Space,
    Newline,
}

/// A token to format: anything but whitespace, with the whitespace before it summed up.
#[derive(Debug, Clone)]
struct Item {
    kind: TokenKind,
    /// Byte range in the whole text.
    source: Range<usize>,
    /// The token as it is written out: as typed, or a keyword in the chosen case.
    text: String,
    /// Lowercase, for words: what the layout matches keywords against.
    word: Option<String>,
    gap: Gap,
    /// The whitespace before the token, for layouts that keep line breaks as typed.
    gap_text: String,
    /// Whether the layout spaces this token from the one before: set on both sides of a
    /// comparison operator written without spaces (`a=1`).
    spaced: bool,
}

impl Item {
    fn is_punctuation(&self, text: &str) -> bool {
        self.kind == TokenKind::Punctuation && self.text == text
    }

    fn is_comment(&self) -> bool {
        matches!(self.kind, TokenKind::LineComment | TokenKind::BlockComment)
    }
}

fn items(text: &str, range: Range<usize>, dialect: Dialect, case: KeywordCase) -> Vec<Item> {
    let fragment = &text[range.clone()];
    let mut items: Vec<Item> = Vec::new();
    let mut gap_text = "";
    for Token { kind, range: local } in tokenize(fragment, dialect) {
        let written = &fragment[local.clone()];
        if kind == TokenKind::Whitespace {
            gap_text = written;
            continue;
        }
        let gap = match gap_text {
            "" => Gap::None,
            gap if gap.contains('\n') => Gap::Newline,
            _ => Gap::Space,
        };
        let word = (kind == TokenKind::Word).then(|| written.to_ascii_lowercase());
        items.push(Item {
            kind,
            source: range.start + local.start..range.start + local.end,
            text: keyword_cased(kind, written, case),
            word,
            gap,
            gap_text: gap_text.to_owned(),
            spaced: false,
        });
        gap_text = "";
    }
    space_comparisons(&mut items);
    items
}

fn keyword_cased(kind: TokenKind, written: &str, case: KeywordCase) -> String {
    if kind != TokenKind::Word || !is_keyword(written) {
        return written.to_owned();
    }
    match case {
        KeywordCase::Upper => written.to_ascii_uppercase(),
        KeywordCase::Lower => written.to_ascii_lowercase(),
        KeywordCase::Preserve => written.to_owned(),
    }
}

/// Comparison operators written without spaces between two operands (`a=1`, `b<>c`) are
/// spaced out. Anything else that touches an operator (`a=-1`, `x::int`, `*`) stays as typed.
const SPACED_OPERATORS: &[&str] = &["=", "<>", "!=", "<", ">", "<=", ">=", "||"];

fn space_comparisons(items: &mut [Item]) {
    let mut start = 1;
    while start < items.len() {
        if !is_operator(&items[start]) {
            start += 1;
            continue;
        }
        // The operator characters written together from `start` on: one operator.
        let end = (start + 1..items.len())
            .find(|&next| !(is_operator(&items[next]) && items[next].gap == Gap::None))
            .unwrap_or(items.len());
        let operator: String = items[start..end]
            .iter()
            .map(|item| item.text.as_str())
            .collect();
        let between_operands =
            is_operand(&items[start - 1]) && items.get(end).is_some_and(is_operand);
        if between_operands && SPACED_OPERATORS.contains(&operator.as_str()) {
            items[start].spaced = true;
            items[end].spaced = true;
        }
        start = end;
    }
}

fn is_operand(item: &Item) -> bool {
    matches!(
        item.kind,
        TokenKind::Word | TokenKind::Number | TokenKind::String | TokenKind::QuotedIdentifier
    ) || item.is_punctuation(")")
        || item.is_punctuation("]")
}

/// Characters that make up operators, and parameter markers (`$1`, `:name`, `?1`, `@p`).
/// Two of them written together may be one operator (`::`, `<=`, `->>`) or one marker, so
/// whitespace is never put between them.
fn is_operator(item: &Item) -> bool {
    item.kind == TokenKind::Punctuation
        && item.text.chars().all(|c| "+-*/<>=~!@#%^&|`?:$".contains(c))
}

/// Whether whitespace may be put between two tokens written together. Splitting operator
/// characters could split an operator; a word before a string or quoted name may be its
/// prefix (`B'1010'`, `x'ff'`, `U&"…"`); a marker before a name or number is a parameter.
fn splittable(before: &Item, after: &Item) -> bool {
    let operators = is_operator(before) && is_operator(after);
    let prefix = before.kind == TokenKind::Word
        && matches!(after.kind, TokenKind::String | TokenKind::QuotedIdentifier);
    let parameter = PARAMETER_MARKERS.contains(&before.text.as_str())
        && before.kind == TokenKind::Punctuation
        && matches!(after.kind, TokenKind::Word | TokenKind::Number);
    !(operators || prefix || parameter)
}

/// What starts a parameter: `$1` (PostgreSQL), `?1`, `:name`, `@name`, `$name` (SQLite).
const PARAMETER_MARKERS: &[&str] = &["$", "?", ":", "@"];

/// Whether the whitespace between two tokens may be removed: before `,`, `;` and `)`, and
/// after `(`, none of which joins with its neighbour into another token.
fn removable(before: &Item, after: &Item) -> bool {
    before.is_punctuation("(")
        || after.kind == TokenKind::Semicolon
        || after.is_punctuation(",")
        || after.is_punctuation(")")
}

/// What goes between the previous token and the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sep {
    Glue,
    Space,
    /// A line break, then this many levels of indentation.
    Newline(usize),
    /// The whitespace as typed, so line breaks the layout doesn't decide stay where they are.
    AsTyped,
}

/// Builds the formatted text, enforcing what [`splittable`] and [`removable`] allow whatever
/// the layout asks for, and that a line comment always ends its line.
struct Writer {
    text: String,
    segments: Vec<Segment>,
    indent_width: usize,
    /// Leading whitespace of the line the current statement starts on.
    base_indent: String,
    /// The indentation level of the line being written.
    line_level: usize,
    previous: Option<Item>,
}

impl Writer {
    fn new(indent_width: usize) -> Self {
        Self {
            text: String::new(),
            segments: Vec::new(),
            indent_width,
            base_indent: String::new(),
            line_level: 0,
            previous: None,
        }
    }

    fn copy(&mut self, text: &str, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        self.segments.push(Segment {
            source: range.clone(),
            target: self.text.len(),
        });
        self.text.push_str(&text[range]);
        self.previous = None;
    }

    fn emit(&mut self, item: &Item, sep: Sep) {
        if let Some(previous) = self.previous.take() {
            let sep = allowed(&previous, item, sep, self.line_level);
            self.separate(item, sep);
        }
        self.segments.push(Segment {
            source: item.source.clone(),
            target: self.text.len(),
        });
        self.text.push_str(&item.text);
        self.previous = Some(item.clone());
    }

    fn separate(&mut self, item: &Item, sep: Sep) {
        match sep {
            Sep::Glue => {}
            Sep::Space => self.text.push(' '),
            Sep::Newline(level) => {
                self.text.push('\n');
                self.text.push_str(&self.base_indent);
                self.text
                    .extend(std::iter::repeat_n(' ', level * self.indent_width));
                self.line_level = level;
            }
            Sep::AsTyped => {
                let mut lines = item.gap_text.split('\n');
                let last = lines.next_back().unwrap_or_default();
                for line in lines {
                    self.text.push_str(line.trim_end_matches([' ', '\t']));
                    self.text.push('\n');
                }
                self.text.push_str(last);
                if item.gap_text.contains('\n') {
                    self.line_level = last.len() / self.indent_width.max(1);
                }
            }
        }
    }
}

/// The separator to use for `wanted`, given the tokens on either side.
fn allowed(before: &Item, after: &Item, wanted: Sep, line_level: usize) -> Sep {
    if before.kind == TokenKind::LineComment {
        return match wanted {
            Sep::Newline(_) => wanted,
            Sep::AsTyped if after.gap == Gap::Newline => wanted,
            _ => Sep::Newline(line_level),
        };
    }
    match wanted {
        Sep::Glue if after.gap != Gap::None && !removable(before, after) => Sep::Space,
        Sep::Space | Sep::Newline(_) | Sep::AsTyped
            if after.gap == Gap::None && !splittable(before, after) =>
        {
            Sep::Glue
        }
        Sep::AsTyped if after.gap == Gap::None => Sep::Glue,
        _ => wanted,
    }
}
