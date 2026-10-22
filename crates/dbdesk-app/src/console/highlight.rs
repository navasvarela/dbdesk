//! Syntax highlighting for the console editor, from the same lexer that splits statements, so
//! what looks like a string is exactly what the splitter treats as one.

use std::ops::Range;

use dbdesk_sql::{Dialect, TokenKind, is_keyword, tokenize};
use egui::text::LayoutJob;
use egui::{FontId, Stroke, Style, TextFormat, TextStyle};

use crate::theme::SyntaxColors;

const ERROR_UNDERLINE_WIDTH: f32 = 1.5;

/// Lays out `text` with SQL colours, underlining the tokens that overlap `error`.
pub fn layout_job(
    text: &str,
    dialect: Dialect,
    error: Option<&Range<usize>>,
    style: &Style,
) -> LayoutJob {
    let font = TextStyle::Monospace.resolve(style);
    let colors = SyntaxColors::for_visuals(&style.visuals);
    let error_stroke = Stroke::new(ERROR_UNDERLINE_WIDTH, style.visuals.error_fg_color);

    let mut job = LayoutJob::default();
    for token in tokenize(text, dialect) {
        let fragment = &text[token.range.clone()];
        let mut format = format_for(token.kind, fragment, &font, &colors);
        if error.is_some_and(|error| overlaps(error, &token.range)) {
            format.underline = error_stroke;
        }
        job.append(fragment, 0.0, format);
    }
    job
}

fn format_for(kind: TokenKind, fragment: &str, font: &FontId, colors: &SyntaxColors) -> TextFormat {
    let color = match kind {
        TokenKind::Word if is_keyword(fragment) => colors.keyword,
        TokenKind::String => colors.string,
        TokenKind::Number => colors.number,
        TokenKind::LineComment | TokenKind::BlockComment => colors.comment,
        TokenKind::QuotedIdentifier => colors.identifier,
        TokenKind::Word | TokenKind::Whitespace | TokenKind::Semicolon | TokenKind::Punctuation => {
            colors.plain
        }
    };
    let mut format = TextFormat::simple(font.clone(), color);
    format.italics = matches!(kind, TokenKind::LineComment | TokenKind::BlockComment);
    format
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// The text to underline for an error reported at byte `offset`: the token there, or one
/// character if the offset falls on whitespace or past the end.
pub fn error_marker(text: &str, dialect: Dialect, offset: usize) -> Range<usize> {
    tokenize(text, dialect)
        .into_iter()
        .find(|token| token.range.contains(&offset) && token.kind != TokenKind::Whitespace)
        .map(|token| token.range)
        .unwrap_or_else(|| {
            let start = offset.min(text.len());
            let end = text[start..]
                .chars()
                .next()
                .map_or(start, |c| start + c.len_utf8());
            start..end
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_the_token_at_the_error() {
        let text = "SELECT * FROM missing_table";

        assert_eq!(
            &text[error_marker(text, Dialect::Postgres, 14)],
            "missing_table"
        );
        assert_eq!(
            error_marker(text, Dialect::Postgres, 99),
            text.len()..text.len()
        );
    }
}
