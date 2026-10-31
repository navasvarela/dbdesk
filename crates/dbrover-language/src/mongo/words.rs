//! The words completion offers in a MongoDB console, each with what it shows beside it, and the
//! ones the editor highlights.

use std::sync::LazyLock;

use super::methods::{CURSOR_METHODS, METHODS};
use super::syntax::Constructor;

const QUERY_OPERATORS: &[&str] = &[
    "$all",
    "$and",
    "$elemMatch",
    "$eq",
    "$exists",
    "$expr",
    "$gt",
    "$gte",
    "$in",
    "$jsonSchema",
    "$lt",
    "$lte",
    "$mod",
    "$ne",
    "$nin",
    "$nor",
    "$not",
    "$or",
    "$regex",
    "$size",
    "$text",
    "$type",
];

const UPDATE_OPERATORS: &[&str] = &[
    "$addToSet",
    "$currentDate",
    "$inc",
    "$max",
    "$min",
    "$mul",
    "$pop",
    "$pull",
    "$pullAll",
    "$push",
    "$rename",
    "$set",
    "$setOnInsert",
    "$unset",
];

const STAGES: &[&str] = &[
    "$addFields",
    "$bucket",
    "$count",
    "$densify",
    "$facet",
    "$fill",
    "$geoNear",
    "$graphLookup",
    "$group",
    "$limit",
    "$lookup",
    "$match",
    "$merge",
    "$out",
    "$project",
    "$redact",
    "$replaceRoot",
    "$replaceWith",
    "$sample",
    "$search",
    "$setWindowFields",
    "$skip",
    "$sort",
    "$sortByCount",
    "$unionWith",
    "$unset",
    "$unwind",
];

const EXPRESSION_OPERATORS: &[&str] = &[
    "$avg",
    "$concat",
    "$cond",
    "$dateToString",
    "$first",
    "$ifNull",
    "$last",
    "$sum",
    "$toString",
];

/// The shell's own words: what starts a command, and its literals.
const SHELL_WORDS: &[&str] = &[
    "collections",
    "databases",
    "db",
    "dbs",
    "false",
    "getCollection",
    "getSiblingDB",
    "new",
    "null",
    "show",
    "true",
    "use",
];

/// Every word with what completion shows beside it, sorted and without repeats: a method's
/// arguments, or what kind of word it is. A word in two lists keeps the first one's detail.
static WORDS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    let mut words: Vec<(&str, &str)> = Vec::new();
    words.extend(METHODS.iter().map(|spec| (spec.name, spec.syntax)));
    words.extend(CURSOR_METHODS.iter().map(|spec| (spec.name, spec.syntax)));
    words.extend(Constructor::NAMES.iter().map(|name| (*name, "BSON type")));
    words.extend(QUERY_OPERATORS.iter().map(|word| (*word, "query operator")));
    words.extend(
        UPDATE_OPERATORS
            .iter()
            .map(|word| (*word, "update operator")),
    );
    words.extend(STAGES.iter().map(|word| (*word, "pipeline stage")));
    words.extend(
        EXPRESSION_OPERATORS
            .iter()
            .map(|word| (*word, "expression")),
    );
    words.extend(SHELL_WORDS.iter().map(|word| (*word, "")));
    let mut seen = std::collections::HashSet::new();
    words.retain(|(word, _)| seen.insert(*word));
    words.sort_unstable_by_key(|(word, _)| *word);
    words
});

static NAMES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| WORDS.iter().map(|(word, _)| *word).collect());

/// Every word completion offers, sorted. They are written as MongoDB writes them, since its
/// names are case-sensitive.
pub(crate) fn completion_words() -> &'static [&'static str] {
    &NAMES
}

/// What completion shows beside `word`: a method's arguments, or what kind of word it is.
pub(crate) fn detail(word: &str) -> Option<&'static str> {
    WORDS
        .binary_search_by_key(&word, |(listed, _)| listed)
        .ok()
        .map(|index| WORDS[index].1)
        .filter(|detail| !detail.is_empty())
}

/// Whether `word` is highlighted: the shell's own words, methods and types. Operators are
/// written inside strings or as keys, which highlighting doesn't single out.
pub(crate) fn is_keyword(word: &str) -> bool {
    SHELL_WORDS.contains(&word)
        || Constructor::NAMES.contains(&word)
        || METHODS.iter().any(|spec| spec.name == word)
        || CURSOR_METHODS.iter().any(|spec| spec.name == word)
}
