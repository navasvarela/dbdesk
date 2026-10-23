use super::{Gap, Item, KeywordCase, items, removable, splittable};
use crate::keywords::is_keyword;
use crate::lexer::{Dialect, TokenKind};

/// Whether `after` is `before` with only whitespace and keyword case changed, in ways that
/// can't change how the database reads it.
///
/// Both are lexed, whitespace dropped, and the tokens compared: same kinds and the same text
/// (keywords ignoring case; strings, comments, names and everything else exactly). A token
/// written against the one before it in `before` may only gain whitespace where the two can't
/// form one token (see `splittable`), and tokens apart in `before` may only be joined
/// before `,`, `;` and `)` or after `(`.
pub fn same_meaning(before: &str, after: &str, dialect: Dialect) -> bool {
    let before = items(before, 0..before.len(), dialect, KeywordCase::Preserve);
    let after = items(after, 0..after.len(), dialect, KeywordCase::Preserve);
    before.len() == after.len()
        && before
            .iter()
            .zip(&after)
            .enumerate()
            .all(|(index, (old, new))| {
                same_token(old, new)
                    && index
                        .checked_sub(1)
                        .is_none_or(|previous| same_spacing(&before[previous], old, new))
            })
}

fn same_token(old: &Item, new: &Item) -> bool {
    old.kind == new.kind
        && (old.text == new.text
            || (old.kind == TokenKind::Word
                && is_keyword(&old.text)
                && old.text.eq_ignore_ascii_case(&new.text)))
}

/// Whether the space between `previous` and `old` may have become the space before `new`.
fn same_spacing(previous: &Item, old: &Item, new: &Item) -> bool {
    match (old.gap == Gap::None, new.gap == Gap::None) {
        (true, false) => splittable(previous, old),
        (false, true) => removable(previous, old),
        _ => true,
    }
}
