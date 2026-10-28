//! Snippets from `snippets.toml` in the editor: offering them in completion, and Tab moving
//! through the fields of the one just inserted.

use std::ops::Range;

use dbrover_store::{Snippet, SnippetBody};

use super::completion::{Candidate, CandidateKind};

/// The snippets whose trigger starts with `prefix` (ignoring case), as candidates, placed in
/// `candidates` after those whose names start with it: a name typed in full wins.
pub fn offer(candidates: &mut Vec<Candidate>, snippets: &[&Snippet], prefix: &str) {
    if prefix.is_empty() {
        return;
    }
    let typed = prefix.to_lowercase();
    let starts = |label: &str| label.to_lowercase().starts_with(&typed);
    let offered = snippets
        .iter()
        .filter(|snippet| starts(&snippet.trigger))
        .map(|snippet| Candidate {
            kind: CandidateKind::Snippet,
            label: snippet.trigger.clone(),
            detail: snippet.description.clone(),
            insert: snippet.body.text().to_owned(),
            cursor_back: 0,
        });
    let at = candidates
        .iter()
        .position(|candidate| !starts(&candidate.label))
        .unwrap_or(candidates.len());
    candidates.splice(at..at, offered);
}

/// The fields of an inserted snippet, as byte ranges of the editor's text, while Tab moves
/// through them. The last is where the cursor ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetFields {
    stops: Vec<Range<usize>>,
    current: usize,
    /// The text's length when last seen, to tell how much an edit added or removed.
    text_len: usize,
}

impl SnippetFields {
    /// The fields of `body`, inserted at byte `at` of a text now `text_len` long. `None` when
    /// the body has no fields, and only places the cursor.
    pub fn start(body: &SnippetBody, at: usize, text_len: usize) -> Option<Self> {
        let stops: Vec<Range<usize>> = body
            .stops()
            .iter()
            .map(|stop| stop.start + at..stop.end + at)
            .collect();
        (stops.len() > 1).then_some(Self {
            stops,
            current: 0,
            text_len,
        })
    }

    /// Where the cursor ends when a body has no fields to visit.
    pub fn end_of(body: &SnippetBody, at: usize) -> usize {
        body.stops().last().map_or(at, |stop| stop.end + at)
    }

    /// What is selected now: the current field.
    pub fn current(&self) -> Range<usize> {
        self.stops[self.current].clone()
    }

    /// Takes in an edit that left the text `text_len` long with the cursor at `cursor`.
    /// Typing in the current field grows or shrinks it and moves the fields after it; an edit
    /// anywhere else ends the fields, which says so by returning `false`.
    pub fn follow_edit(&mut self, text_len: usize, cursor: usize) -> bool {
        let field = self.current();
        let grown = text_len >= self.text_len;
        let change = text_len.abs_diff(self.text_len);
        let end = if grown {
            field.end + change
        } else if field.len() >= change {
            field.end - change
        } else {
            return false;
        };
        if !(field.start..=end).contains(&cursor) {
            return false;
        }
        self.text_len = text_len;
        self.stops[self.current] = field.start..end;
        for stop in &mut self.stops[self.current + 1..] {
            *stop = if grown {
                stop.start + change..stop.end + change
            } else {
                stop.start.saturating_sub(change)..stop.end.saturating_sub(change)
            };
        }
        true
    }

    /// Whether the cursor or selection `selection` is still in the current field, as after a
    /// click inside it.
    pub fn holds(&self, selection: &Range<usize>) -> bool {
        let field = self.current();
        field.start <= selection.start && selection.end <= field.end
    }

    /// Moves to the next field, or the previous one. `None` once past the last, the end,
    /// where the fields are done.
    pub fn advance(&mut self, forward: bool) -> Option<Range<usize>> {
        if forward {
            self.current += 1;
        } else {
            self.current = self.current.saturating_sub(1);
        }
        self.stops.get(self.current).cloned()
    }

    /// Whether the current field is the last, where the cursor ends.
    pub fn at_end(&self) -> bool {
        self.current + 1 == self.stops.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet(trigger: &str, body: &str) -> Snippet {
        Snippet {
            trigger: trigger.to_owned(),
            description: Some(format!("the {trigger} snippet")),
            body: SnippetBody::parse(body).unwrap(),
            drivers: None,
        }
    }

    fn candidate(label: &str) -> Candidate {
        Candidate {
            kind: CandidateKind::Keyword,
            label: label.to_owned(),
            detail: None,
            insert: label.to_owned(),
            cursor_back: 0,
        }
    }

    #[test]
    fn offers_snippets_after_names_that_start_with_the_word() {
        let sel = snippet("sel", "SELECT $1");
        let selc = snippet("selcount", "SELECT count(*) FROM $1");
        let upd = snippet("upd", "UPDATE $1");
        let mut candidates = vec![candidate("SELECT"), candidate("ANALYSE")];

        offer(&mut candidates, &[&sel, &selc, &upd], "Sel");

        let labels: Vec<&str> = candidates.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["SELECT", "sel", "selcount", "ANALYSE"]);
        assert_eq!(candidates[1].kind, CandidateKind::Snippet);
        assert_eq!(candidates[1].detail.as_deref(), Some("the sel snippet"));
        let mut none = Vec::new();
        offer(&mut none, &[&sel], "");
        assert!(none.is_empty());
    }

    #[test]
    fn tab_visits_each_field_then_the_end() {
        let body = SnippetBody::parse("SELECT ${1:*} FROM ${2:t};").unwrap();
        // Inserted after "-- x\n", five bytes.
        let mut fields = SnippetFields::start(&body, 5, 5 + body.text().len()).unwrap();

        assert_eq!(fields.current(), 12..13);
        assert_eq!(fields.advance(true), Some(19..20));
        assert_eq!(fields.advance(false), Some(12..13));
        assert_eq!(fields.advance(true), Some(19..20));
        assert_eq!(fields.advance(true), Some(21..21));
        assert!(fields.at_end());
        assert_eq!(fields.advance(true), None);
    }

    #[test]
    fn typing_in_a_field_moves_the_fields_after_it() {
        let body = SnippetBody::parse("SELECT ${1:*} FROM ${2:t};").unwrap();
        let len = body.text().len();
        let mut fields = SnippetFields::start(&body, 0, len).unwrap();

        // "*" replaced by "id, name": 8 bytes instead of 1, cursor after them.
        assert!(fields.follow_edit(len + 7, 15));
        assert_eq!(fields.current(), 7..15);
        assert_eq!(fields.advance(true), Some(21..22));
        // "t" deleted.
        assert!(fields.follow_edit(len + 6, 21));
        assert_eq!(fields.current(), 21..21);
        assert_eq!(fields.advance(true), Some(22..22));
    }

    #[test]
    fn an_edit_outside_the_field_ends_them() {
        let body = SnippetBody::parse("SELECT ${1:*} FROM ${2:t};").unwrap();
        let len = body.text().len();
        let mut fields = SnippetFields::start(&body, 10, len + 10).unwrap();

        assert!(!fields.follow_edit(len + 11, 1));
        assert!(fields.holds(&(17..18)));
        assert!(!fields.holds(&(0..0)));
    }

    #[test]
    fn a_body_without_fields_only_places_the_cursor() {
        let body = SnippetBody::parse("SELECT 1;$0 -- done").unwrap();
        assert_eq!(SnippetFields::start(&body, 3, 30), None);
        assert_eq!(SnippetFields::end_of(&body, 3), 12);
    }
}
