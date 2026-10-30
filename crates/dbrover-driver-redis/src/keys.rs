//! Keys in the explorer (`docs/nosql.md`, N1): grouped into a tree by the data source's
//! separator, so `user:42:cart` sits under `user` › `42`, and read with `SCAN` a page at a time,
//! never with `KEYS`.
//!
//! A level lists what the keys under its prefix hold next: a group for each prefix up to the
//! next separator, and the keys with no separator left. Keys come in `SCAN`'s order, the order
//! of the server's hash table, so a group can turn up in any batch; the groups already listed
//! are remembered per level, and listed once. A page holds exactly as many nodes as asked
//! (fewer only at the end, or when its time budget runs out). One that stops partway through a
//! batch keeps the batch's unread keys for the next page, which starts with them and goes on
//! from the cursor the server gave after the batch: sending the batch's cursor again could
//! answer with other keys while the server is rehashing, and miss some. A page asked for with a
//! token whose keys the connection no longer holds (a new connection, say) reads that batch
//! again and skips what the token says was read.
//!
//! With a filter, a level lists every key under it whose name contains the filter, ignoring
//! case, without grouping: the filter becomes `SCAN`'s `MATCH`, so the server does the search,
//! and a key three levels down shows at once.

use std::collections::{HashMap, HashSet};

use dbrover_core::{
    CatalogNode, CatalogPath, ChildrenPage, ChildrenQuery, DriverError, KeyType, NodeKind,
    PageToken,
};
use dbrover_language::redis as language;
use redis::aio::ConnectionLike;
use redis::{Cmd, Value};

use crate::{RedisConnection, errors, reply};

/// How many keys `SCAN` is asked for at a time: about a page's worth, so a page usually takes one
/// short command. Chosen, not measured; the server spends about as long on each call as on a
/// `GET` per key.
const SCAN_COUNT: &str = "1000";

/// The most `SCAN` calls one page may take, about 100,000 keys: a level whose keys mostly fall
/// into a few groups would otherwise read the whole database for one page of a million keys.
/// The page then ends early, with a token to go on from.
const SCANS_PER_PAGE: usize = 100;

/// A level of keys: those of `database` whose names start with `prefix`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Level {
    pub(crate) database: i64,
    prefix: Vec<u8>,
}

impl Level {
    /// The level `path` lists: a database, or a group of keys in one. `None` for any other
    /// path, such as a key's, which has no children.
    pub(crate) fn of(path: &CatalogPath) -> Option<Self> {
        let (first, groups) = path.segments().split_first()?;
        if first.kind != NodeKind::Database {
            return None;
        }
        let database = first.name.parse().ok()?;
        let prefix = match groups.last() {
            None => Vec::new(),
            Some(group)
                if groups
                    .iter()
                    .all(|segment| segment.kind == NodeKind::KeyPrefix) =>
            {
                decode(&group.name)?
            }
            Some(_) => return None,
        };
        Some(Self { database, prefix })
    }
}

/// The name a key or a group has in a path: as an argument is written in a command, so a name
/// that isn't text survives the trip.
fn encode(name: &[u8]) -> String {
    language::quote(name)
}

pub(crate) fn decode(name: &str) -> Option<Vec<u8>> {
    match language::arguments(name).ok()?.as_slice() {
        [argument] => Some(argument.clone()),
        _ => None,
    }
}

/// What a key is, seen from the level of `prefix`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    /// The keys sharing this longer prefix, separator included.
    Group(Vec<u8>),
    Key(Vec<u8>),
}

fn entry(key: &[u8], prefix: &[u8], separator: &[u8]) -> Entry {
    let rest = key.get(prefix.len()..).unwrap_or_default();
    match find(rest, separator) {
        Some(at) if !separator.is_empty() => {
            Entry::Group(key[..prefix.len() + at + separator.len()].to_vec())
        }
        _ => Entry::Key(key.to_vec()),
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The `MATCH` pattern for the keys under `prefix`, and, with a filter, only those whose name
/// after it contains the filter, ignoring case: each letter becomes `[aA]`, and the characters
/// a pattern gives a meaning are escaped.
fn match_pattern(prefix: &[u8], filter: Option<&str>) -> Vec<u8> {
    let mut pattern = Vec::with_capacity(prefix.len() + 2);
    for &byte in prefix {
        escape(&mut pattern, byte);
    }
    pattern.push(b'*');
    if let Some(filter) = filter.filter(|filter| !filter.is_empty()) {
        for byte in filter.bytes() {
            if byte.is_ascii_alphabetic() {
                pattern.extend_from_slice(&[
                    b'[',
                    byte.to_ascii_lowercase(),
                    byte.to_ascii_uppercase(),
                    b']',
                ]);
            } else {
                escape(&mut pattern, byte);
            }
        }
        pattern.push(b'*');
    }
    pattern
}

fn escape(pattern: &mut Vec<u8>, byte: u8) {
    if matches!(byte, b'*' | b'?' | b'[' | b']' | b'\\') {
        pattern.push(b'\\');
    }
    pattern.push(byte);
}

/// Where a page goes on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Position {
    /// With a `SCAN` at this cursor.
    At(String),
    /// With the keys of the batch the server answered `batch` with, from `read` on, then a
    /// `SCAN` at `next`, the cursor it gave after that batch.
    Within {
        batch: String,
        read: usize,
        next: String,
    },
}

impl Position {
    const START: &str = "0";

    fn from_token(token: Option<&PageToken>) -> Option<Self> {
        let Some(token) = token else {
            return Some(Self::At(Self::START.to_owned()));
        };
        let mut words = token.as_str().split(' ');
        let position = match words.next()? {
            "at" => Self::At(words.next()?.to_owned()),
            "in" => Self::Within {
                batch: words.next()?.to_owned(),
                read: words.next()?.parse().ok()?,
                next: words.next()?.to_owned(),
            },
            _ => return None,
        };
        words.next().is_none().then_some(position)
    }

    fn token(&self) -> PageToken {
        PageToken::new(match self {
            Self::At(cursor) => format!("at {cursor}"),
            Self::Within { batch, read, next } => format!("in {batch} {read} {next}"),
        })
    }
}

/// What the driver remembers of a level listed a page at a time.
#[derive(Default)]
struct Listing {
    /// The groups already listed, so each is listed once however many batches it turns up in.
    groups: HashSet<Vec<u8>>,
    /// The unread keys of the batch the last page stopped in, and that page's token.
    unread: Option<(PageToken, Vec<Vec<u8>>)>,
}

/// Each level being listed a page at a time, for each filter. Forgotten when the level's first
/// page is asked for again, and once its last page is listed.
#[derive(Default)]
pub(crate) struct Listings(HashMap<(CatalogPath, Option<String>), Listing>);

impl RedisConnection {
    /// A page of the keys at `path`, as `query` asks.
    pub(crate) async fn key_page(
        &mut self,
        path: &CatalogPath,
        level: &Level,
        query: &ChildrenQuery,
    ) -> Result<ChildrenPage, DriverError> {
        let position = Position::from_token(query.after.as_ref())
            .ok_or_else(|| DriverError::InvalidPath(path.clone()))?;
        let filter = query.filter.as_deref().filter(|filter| !filter.is_empty());
        // A filter lists matching keys wherever they are, so it groups nothing.
        let separator = match filter {
            Some(_) => Vec::new(),
            None => self.shared.login().separator.as_bytes().to_vec(),
        };
        let id = (path.clone(), filter.map(str::to_owned));
        let mut listing = match query.after {
            None => Listing::default(),
            Some(_) => self.listings.0.remove(&id).unwrap_or_default(),
        };
        let pattern = match_pattern(&level.prefix, filter);
        let read = self
            .read_entries(
                level,
                &pattern,
                &separator,
                query.limit,
                position,
                &mut listing,
            )
            .await;
        // Kept for the next page, and for asking this one again if it failed.
        if !matches!(read, Ok((_, None))) {
            self.listings.0.insert(id, listing);
        }
        let (entries, next) = read?;
        let nodes = self
            .nodes(path, &level.prefix, &separator, entries)
            .await?
            .into_iter()
            .filter(|node| query.matches(&node.label))
            .collect();
        Ok(ChildrenPage {
            nodes,
            next: next.as_ref().map(Position::token),
        })
    }

    /// Reads keys from `position` on, until `limit` entries are found, the scan ends, or the
    /// page's time budget runs out; returns them, and where the next page goes on from.
    async fn read_entries(
        &mut self,
        level: &Level,
        pattern: &[u8],
        separator: &[u8],
        limit: Option<usize>,
        position: Position,
        listing: &mut Listing,
    ) -> Result<(Vec<Entry>, Option<Position>), DriverError> {
        self.select(level.database).await?;
        let mut scans = 0;
        // The batch being read: the cursor that asked for it, how many of its keys came before
        // `keys`, the keys left to read, and the cursor after it.
        let (mut batch, mut read, mut keys, mut next) = match position {
            Position::At(cursor) => {
                let (next, keys) = self.scan(&cursor, pattern).await?;
                scans += 1;
                (cursor, 0, keys, next)
            }
            Position::Within { batch, read, next } => {
                let token = Position::Within {
                    batch: batch.clone(),
                    read,
                    next: next.clone(),
                }
                .token();
                let kept = listing
                    .unread
                    .take()
                    .filter(|(kept, _)| *kept == token)
                    .map(|(_, keys)| keys);
                let keys = match kept {
                    Some(keys) => keys,
                    None => {
                        let (_, keys) = self.scan(&batch, pattern).await?;
                        scans += 1;
                        keys.into_iter().skip(read).collect()
                    }
                };
                (batch, read, keys, next)
            }
        };
        let mut entries = Vec::new();
        loop {
            for (index, key) in keys.iter().enumerate() {
                let entry = entry(key, &level.prefix, separator);
                if let Entry::Group(group) = &entry
                    && !listing.groups.insert(group.clone())
                {
                    continue;
                }
                entries.push(entry);
                let full = limit.is_some_and(|limit| entries.len() >= limit);
                if full && index + 1 < keys.len() {
                    let stopped = Position::Within {
                        batch,
                        read: read + index + 1,
                        next,
                    };
                    listing.unread = Some((stopped.token(), keys[index + 1..].to_vec()));
                    return Ok((entries, Some(stopped)));
                }
            }
            if next == Position::START {
                return Ok((entries, None));
            }
            let full = limit.is_some_and(|limit| entries.len() >= limit);
            let out_of_time = limit.is_some() && scans >= SCANS_PER_PAGE;
            if full || out_of_time {
                return Ok((entries, Some(Position::At(next))));
            }
            let (after, batch_keys) = self.scan(&next, pattern).await?;
            scans += 1;
            batch = std::mem::replace(&mut next, after);
            read = 0;
            keys = batch_keys;
        }
    }

    /// One `SCAN` call: the cursor to go on from, and the batch of keys.
    async fn scan(
        &mut self,
        cursor: &str,
        pattern: &[u8],
    ) -> Result<(String, Vec<Vec<u8>>), DriverError> {
        let mut command = Cmd::new();
        command
            .arg("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(SCAN_COUNT);
        let reply = self.catalog_query(&command).await?;
        let Value::Array(parts) = reply else {
            return Err(unexpected("SCAN", &reply));
        };
        let [cursor, Value::Array(keys)] = parts.as_slice() else {
            return Err(unexpected("SCAN", &Value::Array(parts)));
        };
        let keys = keys
            .iter()
            .filter_map(|key| match key {
                Value::BulkString(bytes) => Some(bytes.clone()),
                _ => None,
            })
            .collect();
        Ok((reply::text(cursor).unwrap_or_default(), keys))
    }

    /// The nodes for `entries`, each key with its type and time to live, asked for together in
    /// one round trip. A key gone since the scan is left out.
    async fn nodes(
        &mut self,
        level: &CatalogPath,
        prefix: &[u8],
        separator: &[u8],
        entries: Vec<Entry>,
    ) -> Result<Vec<CatalogNode>, DriverError> {
        let keys: Vec<&[u8]> = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Key(key) => Some(key.as_slice()),
                Entry::Group(_) => None,
            })
            .collect();
        let mut pipeline = redis::pipe();
        for key in &keys {
            pipeline.cmd("TYPE").arg(*key).cmd("PTTL").arg(*key);
        }
        let mut answers = if keys.is_empty() {
            Vec::new()
        } else {
            let session = self.session().await?;
            session
                .connection
                .req_packed_commands(&pipeline, 0, keys.len() * 2)
                .await
                .map_err(errors::command)?
        }
        .into_iter();
        let mut nodes = Vec::with_capacity(entries.len());
        for entry in entries {
            match entry {
                Entry::Group(group) => nodes.push(group_node(level, prefix, &group, separator)),
                Entry::Key(key) => {
                    let (Some(key_type), Some(ttl)) = (answers.next(), answers.next()) else {
                        break;
                    };
                    let Some(key_type) = reply::text(&key_type)
                        .as_deref()
                        .and_then(KeyType::from_type_name)
                    else {
                        continue;
                    };
                    let ttl = match ttl {
                        Value::Int(-2) => continue,
                        Value::Int(milliseconds) if milliseconds >= 0 => {
                            Some(milliseconds.unsigned_abs())
                        }
                        _ => None,
                    };
                    nodes.push(key_node(level, prefix, &key, key_type, ttl));
                }
            }
        }
        Ok(nodes)
    }

    /// Makes `database` the session's own, as `SELECT` does, for the explorer's connection to
    /// list another database's keys.
    async fn select(&mut self, database: i64) -> Result<(), DriverError> {
        if database != self.database {
            let mut command = Cmd::new();
            command.arg("SELECT").arg(database);
            self.catalog_query(&command).await?;
            self.database = database;
        }
        Ok(())
    }

    /// Runs one of the explorer's own commands. The server refusing it is the connection
    /// failing, not a statement of the user's.
    pub(crate) async fn catalog_query(&mut self, command: &Cmd) -> Result<Value, DriverError> {
        let session = self.session().await?;
        match session.connection.req_packed_command(command).await {
            Ok(Value::ServerError(error)) => Err(DriverError::Connection(
                errors::ServerFailure(reply::error_text(&error)).into(),
            )),
            Ok(value) => Ok(value),
            Err(error) => Err(match errors::command(error) {
                DriverError::Statement { source, .. } => DriverError::Connection(source),
                other => other,
            }),
        }
    }
}

fn unexpected(command: &str, reply: &Value) -> DriverError {
    DriverError::Connection(format!("the server answered {command} with {reply:?}").into())
}

/// The text a name's bytes show as.
fn label(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A group's node, labelled by what its prefix adds to the level's, without the separator:
/// `42` for `user:42:` under `user:`.
fn group_node(level: &CatalogPath, prefix: &[u8], group: &[u8], separator: &[u8]) -> CatalogNode {
    let end = group
        .len()
        .saturating_sub(separator.len())
        .max(prefix.len());
    CatalogNode {
        path: level.child(NodeKind::KeyPrefix, encode(group)),
        label: label(group.get(prefix.len()..end).unwrap_or_default()),
        detail: None,
        has_children: true,
        has_definition: false,
        system: false,
    }
}

fn key_node(
    level: &CatalogPath,
    prefix: &[u8],
    key: &[u8],
    key_type: KeyType,
    ttl: Option<u64>,
) -> CatalogNode {
    let rest = key.get(prefix.len()..).unwrap_or_default();
    let detail = match ttl {
        Some(milliseconds) => format!("{} · TTL {}", key_type.label(), ttl_text(milliseconds)),
        None => key_type.label().to_owned(),
    };
    CatalogNode {
        path: level.child(NodeKind::Key(key_type), encode(key)),
        // A key named as its group's prefix, such as `user:` among `user:…`, has nothing left.
        label: label(if rest.is_empty() { key } else { rest }),
        detail: Some(detail),
        has_children: false,
        has_definition: false,
        system: false,
    }
}

/// A time to live as a person reads it: the largest whole unit, `42 s`, `3 min`, `5 h`, `2 d`.
pub(crate) fn ttl_text(milliseconds: u64) -> String {
    let seconds = milliseconds.div_ceil(1000);
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3_600 => format!("{} min", seconds / 60),
        3_600..86_400 => format!("{} h", seconds / 3_600),
        _ => format!("{} d", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_keys_by_the_separator_below_the_prefix() {
        let cases: [(&str, &str, &str, Entry); 6] = [
            ("user:42:cart", "", ":", Entry::Group(b"user:".to_vec())),
            (
                "user:42:cart",
                "user:",
                ":",
                Entry::Group(b"user:42:".to_vec()),
            ),
            (
                "user:42:cart",
                "user:42:",
                ":",
                Entry::Key(b"user:42:cart".to_vec()),
            ),
            ("plain", "", ":", Entry::Key(b"plain".to_vec())),
            ("a::b", "", "::", Entry::Group(b"a::".to_vec())),
            ("user:42", "", "", Entry::Key(b"user:42".to_vec())),
        ];
        for (key, prefix, separator, expected) in cases {
            assert_eq!(
                entry(key.as_bytes(), prefix.as_bytes(), separator.as_bytes()),
                expected,
                "{key} under {prefix:?}"
            );
        }
    }

    #[test]
    fn patterns_escape_the_prefix_and_ignore_the_filters_case() {
        assert_eq!(match_pattern(b"user:", None), b"user:*");
        assert_eq!(match_pattern(b"a*b[", None), b"a\\*b\\[*");
        assert_eq!(match_pattern(b"", Some("Ca1?")), b"*[cC][aA]1\\?*");
        assert_eq!(match_pattern(b"", Some("")), b"*");
    }

    #[test]
    fn levels_come_from_a_database_and_its_groups() {
        let database = CatalogPath::root().child(NodeKind::Database, "3");
        assert_eq!(
            Level::of(&database),
            Some(Level {
                database: 3,
                prefix: Vec::new()
            })
        );
        let group = database
            .child(NodeKind::KeyPrefix, "user:")
            .child(NodeKind::KeyPrefix, "\"user:a b:\"");
        assert_eq!(
            Level::of(&group).map(|level| level.prefix),
            Some(b"user:a b:".to_vec())
        );
        assert_eq!(
            Level::of(&database.child(NodeKind::Key(KeyType::Hash), "k")),
            None
        );
        assert_eq!(Level::of(&CatalogPath::root()), None);
    }

    #[test]
    fn names_survive_the_path_whatever_their_bytes() {
        for name in [&b"user:42"[..], b"with space", b"\xff\x00", b"\"quoted\""] {
            assert_eq!(decode(&encode(name)).as_deref(), Some(name));
        }
    }

    #[test]
    fn tokens_say_where_to_go_on() {
        let within = Position::Within {
            batch: "1234".to_owned(),
            read: 17,
            next: "5678".to_owned(),
        };
        assert_eq!(Position::from_token(Some(&within.token())), Some(within));
        let at = Position::At("42".to_owned());
        assert_eq!(Position::from_token(Some(&at.token())), Some(at));
        assert_eq!(
            Position::from_token(None),
            Some(Position::At("0".to_owned()))
        );
        for nonsense in ["nonsense", "at", "in 1 x 2", "at 1 2"] {
            assert_eq!(
                Position::from_token(Some(&PageToken::new(nonsense))),
                None,
                "{nonsense}"
            );
        }
    }

    #[test]
    fn labels_a_key_by_what_follows_its_groups() {
        let level = CatalogPath::root()
            .child(NodeKind::Database, "0")
            .child(NodeKind::KeyPrefix, "user:");
        let node = key_node(&level, b"user:", b"user:42", KeyType::Hash, Some(90_000));
        assert_eq!(node.label, "42");
        assert_eq!(node.detail.as_deref(), Some("hash · TTL 1 min"));
        assert_eq!(node.kind(), Some(NodeKind::Key(KeyType::Hash)));
        assert_eq!(
            key_node(&level, b"user:", b"user:", KeyType::String, None).label,
            "user:"
        );
        let group = group_node(&level, b"user:", b"user:42:", b":");
        assert_eq!(group.label, "42");
        assert_eq!(
            group.path.last().map(|segment| segment.name.as_str()),
            Some("user:42:")
        );
    }

    #[test]
    fn times_to_live_read_in_their_largest_unit() {
        let cases = [
            (0, "0 s"),
            (1, "1 s"),
            (59_000, "59 s"),
            (60_000, "1 min"),
            (7_200_000, "2 h"),
            (172_800_000, "2 d"),
        ];
        for (milliseconds, expected) in cases {
            assert_eq!(ttl_text(milliseconds), expected);
        }
    }
}
