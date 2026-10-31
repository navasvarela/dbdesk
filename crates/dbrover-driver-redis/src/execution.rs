//! Running commands: a reply read whole, a `SCAN` followed to its end a page at a time, and
//! cancelling, within what Redis allows.
//!
//! Redis runs one command at a time and can't stop most of them once started (`docs/nosql.md`,
//! N1). So a cancel unblocks a blocking command with `CLIENT UNBLOCK`, kills a script that
//! hasn't written with `SCRIPT KILL` or `FUNCTION KILL`, and otherwise stops waiting for the
//! reply: the session is then left with a reply still to come, so the next command opens a new
//! one, and the server may still be finishing the command.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use dbrover_core::{CancelHandle, ColumnInfo, DriverError, ResultCursor, ResultShape, Row};
use dbrover_language::redis::{CommandSpec, Flag};
use redis::aio::{ConnectionLike, MultiplexedConnection};
use redis::{Cmd, Value};
use tokio::sync::Notify;

use crate::reply::{self, Table};
use crate::{Login, Session, errors};

/// How a running command can be stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stopping {
    /// It may wait for something to happen (`BLPOP`, `XREAD BLOCK`): `CLIENT UNBLOCK` ends the
    /// wait with an error, and the session stays usable.
    Unblock,
    /// A script: `SCRIPT KILL` stops it, unless it has written.
    KillScript,
    /// A function: `FUNCTION KILL`, likewise.
    KillFunction,
    /// Nothing stops it once the server runs it.
    StopWaiting,
}

impl Stopping {
    pub(crate) fn of(name: &str, spec: Option<&CommandSpec>) -> Self {
        match name {
            "eval" | "evalsha" | "eval_ro" | "evalsha_ro" => Self::KillScript,
            "fcall" | "fcall_ro" => Self::KillFunction,
            _ if spec.is_some_and(|spec| spec.has(Flag::Blocking)) => Self::Unblock,
            _ => Self::StopWaiting,
        }
    }
}

/// The command a session is running, for a cancel to stop.
struct Running {
    stopping: Stopping,
    /// The session's `CLIENT ID`, for `CLIENT UNBLOCK`; `None` when the login may not ask.
    client: Option<i64>,
    /// Woken by a cancel that couldn't stop the command, to stop waiting for its reply.
    stop: Arc<Notify>,
}

/// What a connection, its cursors and its cancel handle share.
pub(crate) struct Shared {
    login: Login,
    running: Mutex<Option<Running>>,
    /// Set when a cancel stopped waiting for a reply. The session would hand that reply to the
    /// next command, so the next command opens a new session first.
    stale: AtomicBool,
}

impl Shared {
    pub(crate) fn new(login: Login) -> Self {
        Self {
            login,
            running: Mutex::new(None),
            stale: AtomicBool::new(false),
        }
    }

    pub(crate) fn login(&self) -> &Login {
        &self.login
    }

    /// Whether the session must be replaced before its next command; clears the mark.
    pub(crate) fn take_stale(&self) -> bool {
        self.stale.swap(false, Ordering::SeqCst)
    }

    fn set_running(&self, running: Option<Running>) {
        *self.running.lock().unwrap_or_else(PoisonError::into_inner) = running;
    }

    fn running(&self) -> Option<(Stopping, Option<i64>, Arc<Notify>)> {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|running| (running.stopping, running.client, Arc::clone(&running.stop)))
    }

    /// Runs `command` on `connection`, as the session `client`, until it answers or a cancel
    /// stops waiting for it. An error the server answers with fails the command.
    pub(crate) async fn run(
        &self,
        connection: &mut MultiplexedConnection,
        client: Option<i64>,
        stopping: Stopping,
        command: &Cmd,
    ) -> Result<Value, DriverError> {
        let stop = Arc::new(Notify::new());
        self.set_running(Some(Running {
            stopping,
            client,
            stop: Arc::clone(&stop),
        }));
        let reply = tokio::select! {
            reply = connection.req_packed_command(command) => Some(reply),
            () = stop.notified() => None,
        };
        self.set_running(None);
        match reply {
            Some(Ok(Value::ServerError(error))) => Err(errors::server(&error)),
            Some(Ok(value)) => Ok(value),
            Some(Err(error)) => Err(errors::command(error)),
            None => {
                self.stale.store(true, Ordering::SeqCst);
                Err(errors::statement(STOPPED_WAITING))
            }
        }
    }

    /// Sends `arguments` on a session of its own, as a cancel does while the connection's
    /// session waits for its reply.
    async fn on_second_session(&self, arguments: &[&str]) -> Option<Value> {
        let mut session = Session::open(&self.login, 0).await.ok()?;
        let mut command = Cmd::new();
        for argument in arguments {
            command.arg(*argument);
        }
        session.connection.req_packed_command(&command).await.ok()
    }
}

const STOPPED_WAITING: &str = "cancelled: DbRover stopped waiting for the reply and opened a new \
    connection, since Redis can't stop this command once it runs; the server may still be \
    finishing it";

/// Cancels a connection's running command from another task.
pub(crate) struct RedisCancel(pub(crate) Arc<Shared>);

#[async_trait]
impl CancelHandle for RedisCancel {
    async fn cancel(&self) -> Result<(), DriverError> {
        let shared = &self.0;
        let Some((stopping, client, stop)) = shared.running() else {
            return Ok(());
        };
        let stopped = match (stopping, client) {
            (Stopping::Unblock, Some(client)) => {
                let client = client.to_string();
                let reply = shared
                    .on_second_session(&["CLIENT", "UNBLOCK", &client, "ERROR"])
                    .await;
                reply == Some(Value::Int(1))
            }
            (Stopping::KillScript, _) => {
                shared.on_second_session(&["SCRIPT", "KILL"]).await == Some(Value::Okay)
            }
            (Stopping::KillFunction, _) => {
                shared.on_second_session(&["FUNCTION", "KILL"]).await == Some(Value::Okay)
            }
            (Stopping::Unblock | Stopping::StopWaiting, _) => false,
        };
        if !stopped {
            tracing::info!("a Redis command couldn't be stopped; no longer waiting for it");
            stop.notify_one();
        }
        Ok(())
    }
}

/// A reply read whole, handed out a page at a time.
pub(crate) struct ReplyCursor {
    columns: Vec<ColumnInfo>,
    shape: ResultShape,
    rows: VecDeque<Row>,
}

impl ReplyCursor {
    pub(crate) fn new(table: Table) -> Self {
        Self {
            columns: table.columns,
            shape: table.shape,
            rows: table.rows.into(),
        }
    }
}

#[async_trait]
impl ResultCursor for ReplyCursor {
    fn columns(&self) -> &[ColumnInfo] {
        &self.columns
    }

    fn shape(&self) -> ResultShape {
        self.shape
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>, DriverError> {
        let count = max_rows.min(self.rows.len());
        Ok(self.rows.drain(..count).collect())
    }

    fn is_exhausted(&self) -> bool {
        self.rows.is_empty()
    }

    async fn finish(self: Box<Self>) -> Result<Option<u64>, DriverError> {
        Ok(None)
    }
}

/// One of the commands that iterate with a cursor, and where its cursor argument is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scan {
    /// `SCAN cursor …`: keys.
    Keys,
    /// `HSCAN key cursor …`: fields and their values, or fields alone with `NOVALUES`.
    Hash { values: bool },
    /// `SSCAN key cursor …`: members.
    Set,
    /// `ZSCAN key cursor …`: members and their scores.
    SortedSet,
}

impl Scan {
    /// The scan `arguments` start, the command's name first, if they start one.
    pub(crate) fn of(arguments: &[Vec<u8>]) -> Option<Self> {
        let name = arguments.first()?;
        let is = |word: &str| name.eq_ignore_ascii_case(word.as_bytes());
        if is("scan") {
            Some(Self::Keys)
        } else if is("hscan") {
            let values = !arguments
                .iter()
                .skip(3)
                .any(|argument| argument.eq_ignore_ascii_case(b"novalues"));
            Some(Self::Hash { values })
        } else if is("sscan") {
            Some(Self::Set)
        } else if is("zscan") {
            Some(Self::SortedSet)
        } else {
            None
        }
    }

    fn cursor_index(self) -> usize {
        match self {
            Self::Keys => 1,
            Self::Hash { .. } | Self::Set | Self::SortedSet => 2,
        }
    }

    fn columns(self) -> &'static [&'static str] {
        match self {
            Self::Keys => &["key"],
            Self::Hash { values: true } => &["field", "value"],
            Self::Hash { values: false } => &["field"],
            Self::Set => &["member"],
            Self::SortedSet => &["member", "score"],
        }
    }
}

/// A `SCAN` followed to its end, as `redis-cli --scan` does: each page sends the command
/// again with the cursor the server last answered, until it answers `0`. The console's paging
/// so reads a large keyspace or value a page at a time, never in one long command.
pub(crate) struct ScanCursor {
    scan: Scan,
    columns: Vec<ColumnInfo>,
    arguments: Vec<Vec<u8>>,
    /// The cursor to send next; `None` once the server has answered `0`.
    next: Option<Vec<u8>>,
    read: VecDeque<Row>,
    connection: MultiplexedConnection,
    client: Option<i64>,
    shared: Arc<Shared>,
}

impl ScanCursor {
    /// Starts the scan and reads its first batch, so a command the server refuses fails here.
    ///
    /// # Errors
    ///
    /// [`DriverError::Statement`] if the server refuses the command, as for any command.
    pub(crate) async fn start(
        scan: Scan,
        arguments: Vec<Vec<u8>>,
        session: &Session,
        shared: Arc<Shared>,
    ) -> Result<Self, DriverError> {
        let next = arguments.get(scan.cursor_index()).cloned();
        let mut cursor = Self {
            scan,
            columns: scan
                .columns()
                .iter()
                .map(|name| reply::column(name))
                .collect(),
            arguments,
            next,
            read: VecDeque::new(),
            connection: session.connection.clone(),
            client: session.client,
            shared,
        };
        cursor.read_batch().await?;
        Ok(cursor)
    }

    async fn read_batch(&mut self) -> Result<(), DriverError> {
        let Some(position) = self.next.take() else {
            return Ok(());
        };
        let mut command = Cmd::new();
        for (index, argument) in self.arguments.iter().enumerate() {
            let argument = if index == self.scan.cursor_index() {
                &position
            } else {
                argument
            };
            command.arg(&argument[..]);
        }
        let reply = self
            .shared
            .run(
                &mut self.connection,
                self.client,
                Stopping::StopWaiting,
                &command,
            )
            .await?;
        let (cursor, items) = match reply {
            Value::Array(mut parts) if parts.len() == 2 => {
                let items = parts.pop().unwrap_or(Value::Nil);
                (parts.pop().unwrap_or(Value::Nil), items)
            }
            other => {
                return Err(errors::statement(format!(
                    "the server answered a scan with {other:?}, not a cursor and a batch"
                )));
            }
        };
        let cursor = reply::text(&cursor).unwrap_or_default();
        self.next = (cursor != "0" && !cursor.is_empty()).then(|| cursor.into_bytes());
        let items = match items {
            Value::Array(items) | Value::Set(items) => items,
            Value::Map(pairs) => pairs
                .into_iter()
                .flat_map(|(field, value)| [field, value])
                .collect(),
            other => vec![other],
        };
        let width = self.columns.len();
        let mut items = items.into_iter().map(reply::cell);
        loop {
            let row: Row = items.by_ref().take(width).collect();
            if row.is_empty() {
                break;
            }
            self.read.push_back(row);
        }
        Ok(())
    }
}

#[async_trait]
impl ResultCursor for ScanCursor {
    fn columns(&self) -> &[ColumnInfo] {
        &self.columns
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>, DriverError> {
        while self.read.len() < max_rows && self.next.is_some() {
            self.read_batch().await?;
        }
        let count = max_rows.min(self.read.len());
        Ok(self.read.drain(..count).collect())
    }

    fn is_exhausted(&self) -> bool {
        self.read.is_empty() && self.next.is_none()
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
    fn recognises_the_scans_and_their_columns() {
        let cases = [
            ("SCAN 0 MATCH user:*", Some(Scan::Keys)),
            ("hscan h 0", Some(Scan::Hash { values: true })),
            (
                "HSCAN h 0 COUNT 10 NOVALUES",
                Some(Scan::Hash { values: false }),
            ),
            ("SSCAN s 0", Some(Scan::Set)),
            ("ZSCAN z 0", Some(Scan::SortedSet)),
            ("GET scan", None),
        ];
        for (command, expected) in cases {
            assert_eq!(Scan::of(&arguments(command)), expected, "{command}");
        }
        assert_eq!(Scan::SortedSet.columns(), ["member", "score"]);
        assert_eq!(Scan::Keys.cursor_index(), 1);
        assert_eq!(Scan::Set.cursor_index(), 2);
    }

    #[test]
    fn knows_how_each_command_can_be_stopped() {
        let blpop = dbrover_language::redis::command(&arguments("BLPOP q 0")).map(|(_, spec)| spec);
        assert_eq!(Stopping::of("blpop", blpop), Stopping::Unblock);
        assert_eq!(Stopping::of("evalsha", None), Stopping::KillScript);
        assert_eq!(Stopping::of("fcall_ro", None), Stopping::KillFunction);
        let get = dbrover_language::redis::command(&arguments("GET k")).map(|(_, spec)| spec);
        assert_eq!(Stopping::of("get", get), Stopping::StopWaiting);
    }
}
