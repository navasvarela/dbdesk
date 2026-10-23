//! Syntax highlighting for the console editor, from the same lexer that splits statements, so
//! what looks like a string is exactly what the splitter treats as one.

use std::ops::Range;

use dbdesk_sql::{Dialect, TokenKind, is_keyword, tokenize};
use egui::text::LayoutJob;
use egui::{FontId, Stroke, Style, TextFormat, TextStyle};

use crate::theme::SyntaxColors;

const ERROR_UNDERLINE_WIDTH: f32 = 1.5;
/// How far the current statement's background moves from the editor's toward the text
/// colour: enough to see which statement ⌘↵ would run, faint enough to read through.
const CURRENT_STATEMENT_TINT: f32 = 0.07;

/// Byte ranges of the text drawn differently from their syntax colours.
#[derive(Debug, Default, Clone)]
pub struct Marks {
    /// Underlined in red: where the last failure points.
    pub error: Option<Range<usize>>,
    /// Shaded: the statement the cursor is in.
    pub current_statement: Option<Range<usize>>,
}

/// Lays out `text` with SQL colours and the `marks`.
pub fn layout_job(text: &str, dialect: Dialect, marks: &Marks, style: &Style) -> LayoutJob {
    let font = TextStyle::Monospace.resolve(style);
    let colors = SyntaxColors::for_visuals(&style.visuals);
    let error_stroke = Stroke::new(ERROR_UNDERLINE_WIDTH, style.visuals.error_fg_color);
    let current_background = style
        .visuals
        .extreme_bg_color
        .lerp_to_gamma(style.visuals.text_color(), CURRENT_STATEMENT_TINT);

    let mut job = LayoutJob::default();
    for token in tokenize(text, dialect) {
        let fragment = &text[token.range.clone()];
        let mut format = format_for(token.kind, fragment, &font, &colors);
        if marks
            .error
            .as_ref()
            .is_some_and(|error| overlaps(error, &token.range))
        {
            format.underline = error_stroke;
        }
        if marks
            .current_statement
            .as_ref()
            .is_some_and(|current| overlaps(current, &token.range))
        {
            format.background = current_background;
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
    fn shades_the_current_statement_and_underlines_the_error() {
        let text = "select 1; select bad";
        let marks = Marks {
            error: Some(17..20),
            current_statement: Some(10..20),
        };

        let job = layout_job(text, Dialect::Postgres, &marks, &Style::default());

        let shaded: String = job
            .sections
            .iter()
            .filter(|section| section.format.background != egui::Color32::TRANSPARENT)
            .map(|section| &text[section.byte_range.start.0..section.byte_range.end.0])
            .collect();
        let underlined: String = job
            .sections
            .iter()
            .filter(|section| section.format.underline != Stroke::NONE)
            .map(|section| &text[section.byte_range.start.0..section.byte_range.end.0])
            .collect();
        assert_eq!(shaded, "select bad");
        assert_eq!(underlined, "bad");
    }

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
