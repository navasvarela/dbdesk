//! Ordering candidates: what matches the typed prefix best, then what was picked recently.

use std::collections::VecDeque;

use super::{Candidate, CandidateKind};

/// More would be a list nobody scrolls; typing narrows it.
const MAX_CANDIDATES: usize = 100;
/// How many accepted candidates are remembered: enough for the names of a working session.
const MAX_RECENT: usize = 50;

/// Candidates accepted lately, most recent first, shared by every console.
#[derive(Debug, Default)]
pub struct RecentCompletions {
    accepted: VecDeque<(CandidateKind, String)>,
}

impl RecentCompletions {
    pub fn record(&mut self, candidate: &Candidate) {
        let key = (candidate.kind, candidate.label.clone());
        self.accepted.retain(|accepted| *accepted != key);
        self.accepted.push_front(key);
        self.accepted.truncate(MAX_RECENT);
    }

    /// How recently the candidate was accepted: 0 for the latest, `None` if not lately.
    fn rank(&self, candidate: &Candidate) -> Option<usize> {
        self.accepted
            .iter()
            .position(|(kind, label)| *kind == candidate.kind && *label == candidate.label)
    }
}

/// Keeps the candidates matching `prefix` and orders them: prefix matches (ignoring case)
/// before fuzzy ones, the tighter fuzzy matches first; within those, recently accepted
/// candidates first, then the order they were found in, which puts the kinds the context
/// calls for first. A name typed in full isn't offered again, so Enter still breaks the line.
pub fn rank(found: Vec<Candidate>, prefix: &str, recent: &RecentCompletions) -> Vec<Candidate> {
    let mut scored: Vec<(usize, usize, usize, Candidate)> = found
        .into_iter()
        .enumerate()
        .filter(|(_, candidate)| !candidate.label.eq_ignore_ascii_case(prefix))
        .filter_map(|(order, candidate)| {
            let quality = match_quality(&candidate.label, prefix)?;
            let recency = recent.rank(&candidate).unwrap_or(usize::MAX);
            Some((quality, recency, order, candidate))
        })
        .collect();
    scored.sort_by_key(|(quality, recency, order, _)| (*quality, *recency, *order));
    scored
        .into_iter()
        .map(|(.., candidate)| candidate)
        .take(MAX_CANDIDATES)
        .collect()
}

/// How well `label` matches what was typed, lower being better: 0 when it starts with it
/// (ignoring case); otherwise 1 plus the characters skipped, when its letters appear in order
/// with each skip landing on the start of a word of the name (`cid` in `customer_id`, not
/// `cus` in `cume_dist`). `None` if it doesn't match.
fn match_quality(label: &str, prefix: &str) -> Option<usize> {
    let label: Vec<char> = label.to_lowercase().chars().collect();
    let prefix: Vec<char> = prefix.to_lowercase().chars().collect();
    if label.starts_with(&prefix) {
        return Some(0);
    }
    let starts_word = |index: usize| index == 0 || !label[index - 1].is_alphanumeric();
    let mut at = 0;
    let mut skipped = 0;
    for (index, want) in prefix.iter().enumerate() {
        if label.get(at) == Some(want) && (index > 0 || at == 0) {
            at += 1;
            continue;
        }
        let found =
            (at + 1..label.len()).find(|&next| starts_word(next) && label[next] == *want)?;
        if index == 0 {
            return None;
        }
        skipped += found - at;
        at = found + 1;
    }
    Some(1 + skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(kind: CandidateKind, label: &str) -> Candidate {
        Candidate {
            kind,
            label: label.to_owned(),
            detail: None,
            insert: label.to_owned(),
            cursor_back: 0,
        }
    }

    fn labels(candidates: &[Candidate]) -> Vec<&str> {
        candidates.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn prefix_matches_come_first_then_fuzzy_ones_then_recent_wins_ties() {
        let found = || {
            vec![
                candidate(CandidateKind::Column, "created_id"),
                candidate(CandidateKind::Column, "customer_id"),
                candidate(CandidateKind::Column, "CUSTOMER"),
                candidate(CandidateKind::Column, "id"),
            ]
        };
        let mut recent = RecentCompletions::default();
        assert_eq!(
            labels(&rank(found(), "cu", &recent)),
            ["customer_id", "CUSTOMER"]
        );
        assert_eq!(
            labels(&rank(found(), "cid", &recent)),
            ["created_id", "customer_id"],
            "fewer letters skipped first"
        );
        assert!(
            rank(found(), "cus_d", &recent).is_empty(),
            "skips land on word starts"
        );
        recent.record(&candidate(CandidateKind::Column, "CUSTOMER"));
        assert_eq!(
            labels(&rank(found(), "cu", &recent)),
            ["CUSTOMER", "customer_id"]
        );
        assert_eq!(labels(&rank(found(), "customer", &recent)), ["customer_id"]);
        assert_eq!(labels(&rank(found(), "", &recent))[0], "CUSTOMER");
    }
}
