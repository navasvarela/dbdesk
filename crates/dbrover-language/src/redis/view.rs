//! A key's value as the command that reads it shows it, and the one command each change to it
//! becomes (`docs/nosql.md`, N1): what **Open data** runs on a key, what the key bar above its
//! result acts on, and what the grid's edits send.
//!
//! The result's columns are the driver's (`dbrover-driver-redis`): a hash as `field` and
//! `value`, a list as `index` and `value`, a set as `member`, a sorted set as `member` and
//! `score`, a string or a JSON document as one `value`, a stream's entries as documents.

use dbrover_core::{Cell, KeyType};

use super::{arguments, quote};

/// How a command that reads a key whole shows its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewShape {
    String,
    Hash,
    List,
    Set,
    SortedSet,
    /// A stream's entries, as documents, which the grid doesn't edit.
    Stream,
    /// A document of the JSON module, as its JSON text.
    Json,
    /// A value of any other type: only its type is shown.
    Other,
}

impl ViewShape {
    /// The columns the driver gives the result, which edits are written against.
    pub fn columns(self) -> &'static [&'static str] {
        match self {
            Self::String | Self::Json => &["value"],
            Self::Hash => &["field", "value"],
            Self::List => &["index", "value"],
            Self::Set => &["member"],
            Self::SortedSet => &["member", "score"],
            Self::Stream => &["document"],
            Self::Other => &["value"],
        }
    }

    fn of_type(key_type: KeyType) -> Self {
        match key_type {
            KeyType::String => Self::String,
            KeyType::Hash => Self::Hash,
            KeyType::List => Self::List,
            KeyType::Set => Self::Set,
            KeyType::SortedSet => Self::SortedSet,
            KeyType::Stream => Self::Stream,
            KeyType::Json => Self::Json,
            KeyType::Other => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Hash => "hash",
            Self::List => "list",
            Self::Set => "set",
            Self::SortedSet => "sorted set",
            Self::Stream => "stream",
            Self::Json => "JSON",
            Self::Other => "other",
        }
    }
}

/// A key, and how the command that read it shows its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyView {
    key: Vec<u8>,
    shape: ViewShape,
}

impl KeyView {
    /// The view of `key`, of type `key_type`, that **Open data** opens.
    pub fn new(key: Vec<u8>, key_type: KeyType) -> Self {
        Self {
            key,
            shape: ViewShape::of_type(key_type),
        }
    }

    /// The key `command` reads whole, and how its result shows it; `None` for any other command,
    /// such as `HGET`, which reads one field, or `ZRANGE` without its scores.
    pub fn of(command: &str) -> Option<Self> {
        let arguments = arguments(command).ok()?;
        let (name, rest) = arguments.split_first()?;
        let name = std::str::from_utf8(name).ok()?.to_ascii_lowercase();
        let has = |word: &str| {
            rest.iter()
                .skip(1)
                .any(|argument| argument.eq_ignore_ascii_case(word.as_bytes()))
        };
        let shape = match (name.as_str(), rest.len()) {
            ("get", 1) => ViewShape::String,
            ("json.get", 1) => ViewShape::Json,
            ("hgetall", 1) => ViewShape::Hash,
            ("hscan", 2..) if !has("novalues") => ViewShape::Hash,
            ("lrange", 3) => ViewShape::List,
            ("smembers", 1) | ("sscan", 2..) => ViewShape::Set,
            ("zrange", 3..) if has("withscores") && !has("byscore") && !has("bylex") => {
                ViewShape::SortedSet
            }
            ("zscan", 2..) => ViewShape::SortedSet,
            ("xrange" | "xrevrange", 3..) => ViewShape::Stream,
            ("type", 1) => ViewShape::Other,
            _ => return None,
        };
        Some(Self {
            key: rest.first()?.clone(),
            shape,
        })
    }

    pub fn key(&self) -> &[u8] {
        &self.key
    }

    pub fn shape(&self) -> ViewShape {
        self.shape
    }

    /// The command that reads the whole value, a page at a time where the value can be large.
    pub fn read(&self) -> String {
        let key = quote(&self.key);
        match self.shape {
            ViewShape::String => format!("GET {key}"),
            ViewShape::Hash => format!("HSCAN {key} 0 COUNT 1000"),
            ViewShape::List => format!("LRANGE {key} 0 -1"),
            ViewShape::Set => format!("SSCAN {key} 0 COUNT 1000"),
            ViewShape::SortedSet => format!("ZRANGE {key} 0 -1 WITHSCORES"),
            ViewShape::Stream => format!("XREVRANGE {key} + -"),
            ViewShape::Json => format!("JSON.GET {key}"),
            ViewShape::Other => format!("TYPE {key}"),
        }
    }

    /// The same view of another key, as after `RENAME`.
    pub fn renamed(&self, key: Vec<u8>) -> Self {
        Self {
            key,
            shape: self.shape,
        }
    }

    /// Whether `columns` are those this view's command gives, so its rows can be edited as it
    /// says.
    pub fn fits(&self, columns: &[&str]) -> bool {
        columns == self.shape.columns()
    }

    /// Whether a loaded row's `column` can be typed into: a value, a score, never what names
    /// the row (a field, an index, a member).
    pub fn edits_loaded(&self, column: usize) -> bool {
        matches!(
            (self.shape, column),
            (ViewShape::String | ViewShape::Json, 0)
                | (ViewShape::Hash | ViewShape::List | ViewShape::SortedSet, 1)
        )
    }

    /// Whether an added row's `column` can be typed into: all but a list's index, which the
    /// server gives.
    pub fn edits_new(&self, column: usize) -> bool {
        self.changes_rows() && !(self.shape == ViewShape::List && column == 0)
    }

    /// Whether rows can be added and deleted: the members of a collection can, a string's value
    /// can't.
    pub fn changes_rows(&self) -> bool {
        matches!(
            self.shape,
            ViewShape::Hash | ViewShape::List | ViewShape::Set | ViewShape::SortedSet
        )
    }

    /// The command that sets `column` of `row`, a loaded row, to `value`.
    pub fn update(&self, row: &[Cell], column: usize, value: &Cell) -> Option<String> {
        let key = quote(&self.key);
        let value = text(value);
        let named = |index: usize| {
            row.get(index)?
                .as_deref()
                .map(|name| quote(name.as_bytes()))
        };
        Some(match (self.shape, column) {
            // KEEPTTL: changing the value doesn't change when the key expires.
            (ViewShape::String, 0) => format!("SET {key} {value} KEEPTTL"),
            (ViewShape::Json, 0) => format!("JSON.SET {key} $ {value}"),
            (ViewShape::Hash, 1) => format!("HSET {key} {} {value}", named(0)?),
            (ViewShape::List, 1) => format!("LSET {key} {} {value}", named(0)?),
            // XX: only change the score of a member that is still there.
            (ViewShape::SortedSet, 1) => format!("ZADD {key} XX {value} {}", named(0)?),
            _ => return None,
        })
    }

    /// The command that adds a row of `values`, by column.
    pub fn insert(&self, values: &[Cell]) -> Option<String> {
        let key = quote(&self.key);
        let value = |index: usize| text(values.get(index).unwrap_or(&None));
        let required = |index: usize| {
            values
                .get(index)?
                .as_deref()
                .filter(|text| !text.is_empty())
                .map(|text| quote(text.as_bytes()))
        };
        Some(match self.shape {
            ViewShape::Hash => format!("HSET {key} {} {}", required(0)?, value(1)),
            ViewShape::List => format!("RPUSH {key} {}", value(1)),
            ViewShape::Set => format!("SADD {key} {}", required(0)?),
            ViewShape::SortedSet => format!("ZADD {key} {} {}", required(1)?, required(0)?),
            ViewShape::String | ViewShape::Json | ViewShape::Stream | ViewShape::Other => {
                return None;
            }
        })
    }

    /// The command that removes `row`, a loaded row. For a list, the first item equal to its
    /// value: Redis removes list items by value, not by index.
    pub fn delete(&self, row: &[Cell]) -> Option<String> {
        let key = quote(&self.key);
        let named = |index: usize| Some(quote(row.get(index)?.as_deref()?.as_bytes()));
        Some(match self.shape {
            ViewShape::Hash => format!("HDEL {key} {}", named(0)?),
            ViewShape::List => format!("LREM {key} 1 {}", text(row.get(1)?)),
            ViewShape::Set => format!("SREM {key} {}", named(0)?),
            ViewShape::SortedSet => format!("ZREM {key} {}", named(0)?),
            ViewShape::String | ViewShape::Json | ViewShape::Stream | ViewShape::Other => {
                return None;
            }
        })
    }

    /// The command that makes the key expire in `seconds`.
    pub fn expire(&self, seconds: u64) -> String {
        format!("EXPIRE {} {seconds}", quote(&self.key))
    }

    /// The command that keeps the key from expiring.
    pub fn persist(&self) -> String {
        format!("PERSIST {}", quote(&self.key))
    }

    /// The command that renames the key, refusing to replace another key of that name.
    pub fn rename(&self, key: &[u8]) -> String {
        format!("RENAMENX {} {}", quote(&self.key), quote(key))
    }

    /// The command that deletes the key.
    pub fn delete_key(&self) -> String {
        format!("DEL {}", quote(&self.key))
    }
}

/// A value as an argument; nil, which Redis can't store, as the empty string.
fn text(value: &Cell) -> String {
    quote(value.as_deref().unwrap_or_default().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(command: &str) -> Option<(Vec<u8>, ViewShape)> {
        KeyView::of(command).map(|view| (view.key, view.shape))
    }

    fn cells(values: &[&str]) -> Vec<Cell> {
        values
            .iter()
            .map(|value| Some((*value).to_owned()))
            .collect()
    }

    #[test]
    fn recognises_the_commands_that_read_a_key_whole() {
        let cases = [
            (
                "GET user:42",
                Some((b"user:42".to_vec(), ViewShape::String)),
            ),
            (
                "hscan \"a b\" 0 COUNT 1000",
                Some((b"a b".to_vec(), ViewShape::Hash)),
            ),
            ("HGETALL h", Some((b"h".to_vec(), ViewShape::Hash))),
            ("LRANGE q 0 -1", Some((b"q".to_vec(), ViewShape::List))),
            ("SMEMBERS s", Some((b"s".to_vec(), ViewShape::Set))),
            (
                "ZRANGE z 0 -1 WITHSCORES",
                Some((b"z".to_vec(), ViewShape::SortedSet)),
            ),
            (
                "XREVRANGE events + -",
                Some((b"events".to_vec(), ViewShape::Stream)),
            ),
            ("JSON.GET doc", Some((b"doc".to_vec(), ViewShape::Json))),
            ("HSCAN h 0 NOVALUES", None),
            ("ZRANGE z 0 -1", None),
            ("ZRANGE z 0 10 BYSCORE WITHSCORES", None),
            ("HGET h field", None),
            ("GET a b", None),
        ];
        for (command, expected) in cases {
            assert_eq!(view(command), expected, "{command}");
        }
    }

    #[test]
    fn open_data_reads_each_type_a_page_at_a_time_and_is_recognised_again() {
        for key_type in [
            KeyType::String,
            KeyType::Hash,
            KeyType::List,
            KeyType::Set,
            KeyType::SortedSet,
            KeyType::Stream,
            KeyType::Json,
            KeyType::Other,
        ] {
            let opened = KeyView::new(b"user 42".to_vec(), key_type);
            assert_eq!(
                KeyView::of(&opened.read()),
                Some(opened.clone()),
                "{key_type:?}"
            );
        }
        assert_eq!(
            KeyView::new(b"h".to_vec(), KeyType::Hash).read(),
            "HSCAN h 0 COUNT 1000"
        );
    }

    #[test]
    fn each_change_is_one_command() {
        let hash = KeyView::new(b"user:42".to_vec(), KeyType::Hash);
        let row = cells(&["name", "Ada"]);
        assert_eq!(
            hash.update(&row, 1, &Some("Ada Lovelace".to_owned()))
                .as_deref(),
            Some("HSET user:42 name \"Ada Lovelace\"")
        );
        assert_eq!(hash.update(&row, 0, &Some("x".to_owned())), None);
        assert_eq!(hash.delete(&row).as_deref(), Some("HDEL user:42 name"));
        assert_eq!(
            hash.insert(&cells(&["city", "London"])).as_deref(),
            Some("HSET user:42 city London")
        );
        assert_eq!(hash.insert(&[None, Some("x".to_owned())]), None);

        let list = KeyView::new(b"q".to_vec(), KeyType::List);
        assert_eq!(
            list.update(&cells(&["3", "a"]), 1, &Some("b".to_owned()))
                .as_deref(),
            Some("LSET q 3 b")
        );
        assert_eq!(
            list.insert(&[None, Some("job".to_owned())]).as_deref(),
            Some("RPUSH q job")
        );
        assert_eq!(
            list.delete(&cells(&["3", "a b"])).as_deref(),
            Some("LREM q 1 \"a b\"")
        );
        assert!(!list.edits_new(0));

        let scores = KeyView::new(b"z".to_vec(), KeyType::SortedSet);
        assert_eq!(
            scores
                .update(&cells(&["ada", "10"]), 1, &Some("11".to_owned()))
                .as_deref(),
            Some("ZADD z XX 11 ada")
        );
        assert_eq!(
            scores.insert(&cells(&["bob", "7.5"])).as_deref(),
            Some("ZADD z 7.5 bob")
        );
        assert_eq!(
            scores.delete(&cells(&["bob", "7.5"])).as_deref(),
            Some("ZREM z bob")
        );

        let set = KeyView::new(b"s".to_vec(), KeyType::Set);
        assert!(!set.edits_loaded(0));
        assert_eq!(set.insert(&cells(&["red"])).as_deref(), Some("SADD s red"));

        let string = KeyView::new(b"k".to_vec(), KeyType::String);
        assert_eq!(
            string.update(&cells(&["a"]), 0, &None).as_deref(),
            Some("SET k \"\" KEEPTTL")
        );
        assert!(!string.changes_rows());
        assert_eq!(string.insert(&cells(&["a"])), None);

        let json = KeyView::new(b"doc".to_vec(), KeyType::Json);
        assert_eq!(
            json.update(&cells(&["{}"]), 0, &Some(r#"{"a":1}"#.to_owned()))
                .as_deref(),
            Some(r#"JSON.SET doc $ "{\"a\":1}""#)
        );
    }

    #[test]
    fn the_key_bar_acts_on_the_key() {
        let view = KeyView::new(b"user:42".to_vec(), KeyType::Hash);
        assert_eq!(view.expire(60), "EXPIRE user:42 60");
        assert_eq!(view.persist(), "PERSIST user:42");
        assert_eq!(view.rename(b"user 43"), "RENAMENX user:42 \"user 43\"");
        assert_eq!(view.delete_key(), "DEL user:42");
        assert_eq!(view.renamed(b"x".to_vec()).read(), "HSCAN x 0 COUNT 1000");
    }

    #[test]
    fn fits_only_the_columns_its_command_gives() {
        let hash = KeyView::new(b"h".to_vec(), KeyType::Hash);
        assert!(hash.fits(&["field", "value"]));
        assert!(!hash.fits(&["value"]));
    }
}
