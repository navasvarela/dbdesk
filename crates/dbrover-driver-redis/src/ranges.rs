//! Commands that read a range of one large value, read a page at a time rather than in one
//! reply (`docs/nosql.md`, N1): `LRANGE` and `ZRANGE … WITHSCORES` by index, `XRANGE` and
//! `XREVRANGE` without `COUNT`. Each page is one short command over the next part of the range,
//! so a list of a million items neither stalls the server nor fills memory.
//!
//! A list's items come with their index, which an edit names the item by (`LSET`). If the value
//! changes between pages, the pages see it as it is when each is read, as `SCAN` does.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use dbrover_core::{ColumnInfo, DriverError, ResultCursor, ResultShape, Row};
use redis::aio::MultiplexedConnection;
use redis::{Cmd, Value};

use crate::execution::{Shared, Stopping};
use crate::reply::{self, Table};
use crate::{Session, errors};

/// How many items the first page reads, before the grid asks for a page of its own size.
const FIRST_PAGE: i64 = 500;

/// A range command the driver pages through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Range {
    /// `LRANGE key start stop`.
    List { start: i64, stop: i64 },
    /// `ZRANGE key start stop WITHSCORES`, highest scores first with `REV`.
    SortedSet {
        start: i64,
        stop: i64,
        reverse: bool,
    },
    /// `XRANGE key start end`, or `XREVRANGE key end start` when `reverse`: the bounds as
    /// written, the first one where the next page starts.
    Stream {
        from: Vec<u8>,
        to: Vec<u8>,
        reverse: bool,
    },
}

impl Range {
    /// The range `arguments` read, the command's name first, if the driver pages it.
    pub(crate) fn of(arguments: &[Vec<u8>]) -> Option<(Vec<u8>, Self)> {
        let (name, rest) = arguments.split_first()?;
        let name = std::str::from_utf8(name).ok()?.to_ascii_lowercase();
        let number = |index: usize| std::str::from_utf8(rest.get(index)?).ok()?.parse().ok();
        let options: Vec<String> = rest
            .iter()
            .skip(3)
            .map(|option| String::from_utf8_lossy(option).to_ascii_lowercase())
            .collect();
        let range = match name.as_str() {
            "lrange" if rest.len() == 3 => Self::List {
                start: number(1)?,
                stop: number(2)?,
            },
            "zrange"
                if options.iter().any(|option| option == "withscores")
                    && options
                        .iter()
                        .all(|option| matches!(option.as_str(), "withscores" | "rev")) =>
            {
                Self::SortedSet {
                    start: number(1)?,
                    stop: number(2)?,
                    reverse: options.iter().any(|option| option == "rev"),
                }
            }
            "xrange" | "xrevrange" if rest.len() == 3 => Self::Stream {
                from: rest.get(1)?.clone(),
                to: rest.get(2)?.clone(),
                reverse: name == "xrevrange",
            },
            _ => return None,
        };
        Some((rest.first()?.clone(), range))
    }

    fn columns(&self) -> &'static [&'static str] {
        match self {
            Self::List { .. } => &["index", "value"],
            Self::SortedSet { .. } => &["member", "score"],
            Self::Stream { .. } => &["document"],
        }
    }

    /// The command that counts the value's items, to resolve indexes counted from the end.
    fn length_command(&self) -> Option<&'static str> {
        match self {
            Self::List { .. } => Some("LLEN"),
            Self::SortedSet { .. } => Some("ZCARD"),
            Self::Stream { .. } => None,
        }
    }
}

/// An index of a range as Redis reads it, `-1` the last item, made absolute within `length`.
fn absolute(index: i64, length: i64) -> i64 {
    if index < 0 { length + index } else { index }
}

pub(crate) struct RangeCursor {
    key: Vec<u8>,
    range: Range,
    columns: Vec<ColumnInfo>,
    /// For a list or sorted set: the next index to read, and the last, both absolute.
    next: i64,
    last: i64,
    done: bool,
    read: VecDeque<Row>,
    connection: MultiplexedConnection,
    client: Option<i64>,
    shared: Arc<Shared>,
}

impl RangeCursor {
    /// Resolves the range and reads its first page, so a command the server refuses, such as
    /// `LRANGE` on a hash, fails here.
    ///
    /// # Errors
    ///
    /// [`DriverError::Statement`] if the server refuses a command, as for any command.
    pub(crate) async fn start(
        key: Vec<u8>,
        range: Range,
        session: &Session,
        shared: Arc<Shared>,
    ) -> Result<Self, DriverError> {
        let mut cursor = Self {
            key,
            columns: range
                .columns()
                .iter()
                .map(|name| reply::column(name))
                .collect(),
            range,
            next: 0,
            last: -1,
            done: false,
            read: VecDeque::new(),
            connection: session.connection.clone(),
            client: session.client,
            shared,
        };
        if let Some(length) = cursor.range.length_command() {
            let mut command = Cmd::new();
            command.arg(length).arg(&cursor.key[..]);
            let length = match cursor.run(&command).await? {
                Value::Int(length) => length,
                other => return Err(unexpected(length, &other)),
            };
            let (start, stop) = match cursor.range {
                Range::List { start, stop } | Range::SortedSet { start, stop, .. } => (start, stop),
                Range::Stream { .. } => (0, -1),
            };
            cursor.next = absolute(start, length).max(0);
            cursor.last = absolute(stop, length).min(length - 1);
        }
        cursor.read_page(FIRST_PAGE).await?;
        Ok(cursor)
    }

    async fn run(&mut self, command: &Cmd) -> Result<Value, DriverError> {
        self.shared
            .run(
                &mut self.connection,
                self.client,
                Stopping::StopWaiting,
                command,
            )
            .await
    }

    /// Reads up to `count` more items into `read`.
    async fn read_page(&mut self, count: i64) -> Result<(), DriverError> {
        if self.done {
            return Ok(());
        }
        let mut command = Cmd::new();
        match self.range.clone() {
            Range::List { .. } | Range::SortedSet { .. } => {
                if self.next > self.last {
                    self.done = true;
                    return Ok(());
                }
                let end = self.last.min(self.next + count - 1);
                let name = match self.range {
                    Range::List { .. } => "LRANGE",
                    _ => "ZRANGE",
                };
                command.arg(name).arg(&self.key[..]).arg(self.next).arg(end);
                if let Range::SortedSet { reverse, .. } = self.range {
                    command.arg("WITHSCORES");
                    if reverse {
                        command.arg("REV");
                    }
                }
                let reply = self.run(&command).await?;
                let first = self.next;
                self.next = end + 1;
                self.done = self.next > self.last;
                self.add_items(first, reply);
            }
            Range::Stream { from, to, reverse } => {
                let name = if reverse { "XREVRANGE" } else { "XRANGE" };
                command
                    .arg(name)
                    .arg(&self.key[..])
                    .arg(&from[..])
                    .arg(&to[..])
                    .arg("COUNT")
                    .arg(count);
                let reply = self.run(&command).await?;
                let entries = match &reply {
                    Value::Array(entries) => entries.len(),
                    _ => 0,
                };
                let last_id = last_entry_id(&reply);
                let Table { rows, .. } = reply::table(&name.to_ascii_lowercase(), reply);
                self.read.extend(rows);
                match last_id {
                    // `(` makes the bound exclusive (Redis 6.2 and later), so the next page starts
                    // after the entry this one ended with.
                    Some(id) if i64::try_from(entries).is_ok_and(|entries| entries == count) => {
                        let mut from = b"(".to_vec();
                        from.extend_from_slice(&id);
                        self.range = Range::Stream { from, to, reverse };
                    }
                    _ => self.done = true,
                }
            }
        }
        Ok(())
    }

    /// Adds a list's items, each with its index from `first` on, or a sorted set's members with
    /// their scores.
    fn add_items(&mut self, first: i64, reply: Value) {
        let Value::Array(items) = reply else {
            return;
        };
        match self.range {
            Range::List { .. } => {
                for (index, item) in (first..).zip(items) {
                    self.read
                        .push_back(vec![Some(index.to_string()), reply::cell(item)]);
                }
            }
            Range::SortedSet { .. } => {
                let Table { rows, .. } = reply::table("zrange", Value::Array(items));
                self.read.extend(rows);
            }
            Range::Stream { .. } => {}
        }
    }
}

/// The id of the last entry of an `XRANGE` reply.
fn last_entry_id(reply: &Value) -> Option<Vec<u8>> {
    let Value::Array(entries) = reply else {
        return None;
    };
    let Value::Array(entry) = entries.last()? else {
        return None;
    };
    match entry.first()? {
        Value::BulkString(id) => Some(id.clone()),
        Value::SimpleString(id) => Some(id.clone().into_bytes()),
        _ => None,
    }
}

fn unexpected(command: &str, reply: &Value) -> DriverError {
    errors::statement(format!("the server answered {command} with {reply:?}"))
}

#[async_trait]
impl ResultCursor for RangeCursor {
    fn columns(&self) -> &[ColumnInfo] {
        &self.columns
    }

    fn shape(&self) -> ResultShape {
        match self.range {
            Range::Stream { .. } => ResultShape::Documents,
            Range::List { .. } | Range::SortedSet { .. } => ResultShape::Rows,
        }
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>, DriverError> {
        let wanted = i64::try_from(max_rows).unwrap_or(i64::MAX);
        let missing = wanted - i64::try_from(self.read.len()).unwrap_or(i64::MAX);
        if missing > 0 {
            self.read_page(missing).await?;
        }
        let count = max_rows.min(self.read.len());
        Ok(self.read.drain(..count).collect())
    }

    fn is_exhausted(&self) -> bool {
        self.read.is_empty() && self.done
    }

    async fn finish(self: Box<Self>) -> Result<Option<u64>, DriverError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(command: &str) -> Vec<Vec<u8>> {
        dbrover_language::redis::arguments(command).unwrap()
    }

    #[test]
    fn recognises_the_ranges_it_pages() {
        let cases = [
            ("LRANGE q 0 -1", Some(Range::List { start: 0, stop: -1 })),
            (
                "zrange z 0 -1 WITHSCORES REV",
                Some(Range::SortedSet {
                    start: 0,
                    stop: -1,
                    reverse: true,
                }),
            ),
            ("ZRANGE z 0 -1", None),
            ("ZRANGE z 0 10 BYSCORE WITHSCORES", None),
            (
                "XREVRANGE s + -",
                Some(Range::Stream {
                    from: b"+".to_vec(),
                    to: b"-".to_vec(),
                    reverse: true,
                }),
            ),
            ("XRANGE s - + COUNT 10", None),
            ("LRANGE q a b", None),
        ];
        for (command, expected) in cases {
            assert_eq!(
                Range::of(&arguments(command)).map(|(_, range)| range),
                expected,
                "{command}"
            );
        }
    }

    #[test]
    fn indexes_count_from_the_end_when_negative() {
        assert_eq!(absolute(-1, 10), 9);
        assert_eq!(absolute(3, 10), 3);
        assert_eq!(absolute(-20, 10), -10);
    }
}
