//! Query history: every statement a console or an MCP client runs, in `history.sqlite`. Only
//! the SQL and how it went are kept, never the rows.
//!
//! The app and `dbdesk mcp` may have the file open at the same time, each writing. The
//! database is in WAL mode, where one writer at a time appends while readers carry on, and
//! each connection waits up to [`BUSY_TIMEOUT`] for the other's write to finish rather than
//! failing at once.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dbdesk_core::DataSourceId;
use jiff::Timestamp;
use rusqlite::{TransactionBehavior, params};

/// Lexically sortable and RFC 3339: fixed millisecond precision, always UTC. A variable
/// number of fraction digits would sort `…:00.5Z` before `…:00Z`.
const TIMESTAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

/// How long a write waits for the other process's write to finish. Each write is one small
/// row, so a wait anywhere near this long means something is wrong, not busy.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS history (
    id            INTEGER PRIMARY KEY,
    data_source   TEXT NOT NULL,
    executed_at   TEXT NOT NULL,
    sql           TEXT NOT NULL,
    duration_ms   INTEGER,
    rows          INTEGER,
    outcome       TEXT NOT NULL,
    source        TEXT NOT NULL DEFAULT 'app'
);
CREATE INDEX IF NOT EXISTS history_by_source ON history (data_source, executed_at DESC);
";

/// Files written before MCP (0.2.0 and earlier) lack the `source` column. Every row in them
/// came from the app, which is what the default says.
const ADD_SOURCE: &str = "ALTER TABLE history ADD COLUMN source TEXT NOT NULL DEFAULT 'app'";

/// How a statement ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryOutcome {
    Ok,
    Error,
    Cancelled,
}

impl HistoryOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [Self::Ok, Self::Error, Self::Cancelled]
            .into_iter()
            .find(|outcome| outcome.as_str() == text)
    }
}

/// Who ran a statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistorySource {
    /// A query console in the app.
    App,
    /// An MCP client, through `dbdesk mcp`.
    Mcp,
}

impl HistorySource {
    fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Mcp => "mcp",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [Self::App, Self::Mcp]
            .into_iter()
            .find(|source| source.as_str() == text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub data_source: DataSourceId,
    pub executed_at: Timestamp,
    pub sql: String,
    pub duration: Option<Duration>,
    /// Rows changed or returned, when the database reported it.
    pub rows: Option<u64>,
    pub outcome: HistoryOutcome,
    pub source: HistorySource,
}

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("could not open the query history at {}", path.display())]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("could not create the folder for {}", path.display())]
    CreateFolder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the query history database failed")]
    Database(#[from] rusqlite::Error),
}

/// Where the history database lives; [`HistoryFile::open`] opens it.
#[derive(Debug, Clone)]
pub struct HistoryFile {
    path: PathBuf,
}

impl HistoryFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Opens the database, creating it and its folder if needed.
    ///
    /// # Errors
    ///
    /// [`HistoryError::CreateFolder`] or [`HistoryError::Open`] if it can't be created or
    /// isn't a usable database.
    pub fn open(&self) -> Result<History, HistoryError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| HistoryError::CreateFolder {
                path: self.path.clone(),
                source,
            })?;
        }
        let open_error = |source| HistoryError::Open {
            path: self.path.clone(),
            source,
        };
        let mut connection = rusqlite::Connection::open(&self.path).map_err(open_error)?;
        connection.busy_timeout(BUSY_TIMEOUT).map_err(open_error)?;
        // Persistent: once one process switches the file, every later connection, an older
        // dbdesk's included, opens it in WAL mode.
        connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| {
                row.get::<_, String>(0)
            })
            .map_err(open_error)?;
        migrate(&mut connection).map_err(open_error)?;
        Ok(History { connection })
    }
}

/// Brings the schema up to date. In an immediate transaction, so when the app and the MCP
/// server open an old file at the same moment, the second waits, then finds the column added.
fn migrate(connection: &mut rusqlite::Connection) -> rusqlite::Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(SCHEMA)?;
    let has_source = transaction
        .prepare("SELECT 1 FROM pragma_table_info('history') WHERE name = 'source'")?
        .exists([])?;
    if !has_source {
        transaction.execute(ADD_SOURCE, [])?;
    }
    transaction.commit()
}

/// An open history database.
pub struct History {
    connection: rusqlite::Connection,
}

impl History {
    /// # Errors
    ///
    /// [`HistoryError::Database`] if the row can't be written.
    pub fn record(&self, entry: &HistoryEntry) -> Result<(), HistoryError> {
        let duration_ms = entry
            .duration
            .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX));
        let rows = entry
            .rows
            .map(|rows| i64::try_from(rows).unwrap_or(i64::MAX));
        self.connection.execute(
            "INSERT INTO history (data_source, executed_at, sql, duration_ms, rows, outcome, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.data_source.to_string(),
                format_timestamp(entry.executed_at),
                entry.sql,
                duration_ms,
                rows,
                entry.outcome.as_str(),
                entry.source.as_str(),
            ],
        )?;
        Ok(())
    }

    /// The data source's statements containing `text` (ignoring ASCII case), newest first,
    /// at most `limit` of them. Rows that don't parse (edited by hand, say) are skipped.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Database`] if the query fails.
    pub fn search(
        &self,
        data_source: DataSourceId,
        text: &str,
        limit: usize,
    ) -> Result<Vec<HistoryEntry>, HistoryError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT executed_at, sql, duration_ms, rows, outcome, source FROM history
             WHERE data_source = ?1 AND sql LIKE '%' || ?2 || '%' ESCAPE '\\'
             ORDER BY executed_at DESC, id DESC
             LIMIT ?3",
        )?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = statement.query_map(
            params![data_source.to_string(), escape_like(text), limit],
            |row| {
                Ok(StoredRow {
                    executed_at: row.get(0)?,
                    sql: row.get(1)?,
                    duration_ms: row.get(2)?,
                    rows: row.get(3)?,
                    outcome: row.get(4)?,
                    source: row.get(5)?,
                })
            },
        )?;
        let mut entries = Vec::new();
        for row in rows {
            entries.extend(row?.into_entry(data_source));
        }
        Ok(entries)
    }
}

/// A history row as stored, before parsing.
struct StoredRow {
    executed_at: String,
    sql: String,
    duration_ms: Option<i64>,
    rows: Option<i64>,
    outcome: String,
    source: String,
}

impl StoredRow {
    fn into_entry(self, data_source: DataSourceId) -> Option<HistoryEntry> {
        Some(HistoryEntry {
            data_source,
            executed_at: self.executed_at.parse().ok()?,
            sql: self.sql,
            duration: self
                .duration_ms
                .and_then(|ms| u64::try_from(ms).ok())
                .map(Duration::from_millis),
            rows: self.rows.and_then(|rows| u64::try_from(rows).ok()),
            outcome: HistoryOutcome::parse(&self.outcome)?,
            source: HistorySource::parse(&self.source)?,
        })
    }
}

fn format_timestamp(timestamp: Timestamp) -> String {
    timestamp.strftime(TIMESTAMP_FORMAT).to_string()
}

/// Makes `%`, `_` and the escape character itself match literally in a `LIKE` pattern.
fn escape_like(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(data_source: DataSourceId, at_ms: i64, sql: &str) -> HistoryEntry {
        HistoryEntry {
            data_source,
            executed_at: Timestamp::from_millisecond(at_ms).unwrap(),
            sql: sql.to_owned(),
            duration: Some(Duration::from_millis(12)),
            rows: Some(3),
            outcome: HistoryOutcome::Ok,
            source: HistorySource::App,
        }
    }

    #[test]
    fn finds_a_data_sources_statements_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let history = HistoryFile::new(dir.path().join("nested/history.sqlite"))
            .open()
            .unwrap();
        let (shop, crm) = (DataSourceId::generate(), DataSourceId::generate());
        // 1_000 ms formats as `…:01.000Z`; a variable-width fraction would sort it after
        // `…:01.500Z`.
        let older = entry(shop, 1_000, "SELECT * FROM orders");
        let newer = HistoryEntry {
            outcome: HistoryOutcome::Cancelled,
            duration: None,
            rows: None,
            ..entry(shop, 1_500, "select count(*) from ORDERS")
        };
        for recorded in [&older, &newer, &entry(crm, 2_000, "SELECT * FROM orders")] {
            history.record(recorded).unwrap();
        }

        assert_eq!(
            history.search(shop, "", 10).unwrap(),
            [newer.clone(), older]
        );
        assert_eq!(
            history.search(shop, "count", 10).unwrap(),
            std::slice::from_ref(&newer)
        );
        assert_eq!(history.search(shop, "orders", 1).unwrap(), [newer]);
    }

    #[test]
    fn matches_like_wildcards_literally() {
        let dir = tempfile::tempdir().unwrap();
        let history = HistoryFile::new(dir.path().join("history.sqlite"))
            .open()
            .unwrap();
        let shop = DataSourceId::generate();
        for sql in ["select '100%'", "select 1000", "select a_b", "select axb"] {
            history.record(&entry(shop, 0, sql)).unwrap();
        }

        let found = |text| -> Vec<String> {
            history
                .search(shop, text, 10)
                .unwrap()
                .into_iter()
                .map(|entry| entry.sql)
                .collect()
        };
        assert_eq!(found("0%"), ["select '100%'"]);
        assert_eq!(found("a_b"), ["select a_b"]);
    }

    #[test]
    fn adds_the_source_column_to_a_file_from_0_2_0() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let shop = DataSourceId::generate();
        let old = rusqlite::Connection::open(&path).unwrap();
        old.execute_batch(
            "CREATE TABLE history (
                id INTEGER PRIMARY KEY, data_source TEXT NOT NULL, executed_at TEXT NOT NULL,
                sql TEXT NOT NULL, duration_ms INTEGER, rows INTEGER, outcome TEXT NOT NULL
            );",
        )
        .unwrap();
        old.execute(
            "INSERT INTO history (data_source, executed_at, sql, outcome)
             VALUES (?1, '2026-10-01T10:00:00.000Z', 'SELECT 1', 'ok')",
            [shop.to_string()],
        )
        .unwrap();

        // An older dbdesk still has the file open, as when the app runs during an upgrade.
        let history = HistoryFile::new(&path).open().unwrap();
        history
            .record(&HistoryEntry {
                source: HistorySource::Mcp,
                ..entry(shop, 1_800_000_000_000, "SELECT 2")
            })
            .unwrap();
        old.execute(
            "INSERT INTO history (data_source, executed_at, sql, outcome)
             VALUES (?1, '2026-10-02T10:00:00.000Z', 'SELECT 3', 'ok')",
            [shop.to_string()],
        )
        .unwrap();

        let sources: Vec<(String, HistorySource)> = history
            .search(shop, "", 10)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.sql, entry.source))
            .collect();
        assert_eq!(
            sources,
            [
                ("SELECT 2".to_owned(), HistorySource::Mcp),
                ("SELECT 3".to_owned(), HistorySource::App),
                ("SELECT 1".to_owned(), HistorySource::App),
            ]
        );
        let mode: String = old
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    /// Set to a history file's path in the child process of
    /// [`two_processes_write_at_once`]; [`write_as_second_process`] writes there.
    const CHILD_PATH: &str = "DBDESK_HISTORY_TEST_CHILD_PATH";
    const WRITES_PER_PROCESS: i64 = 200;

    /// Runs only as the child of [`two_processes_write_at_once`].
    #[test]
    #[ignore = "a helper process for two_processes_write_at_once"]
    fn write_as_second_process() {
        let Ok(path) = std::env::var(CHILD_PATH) else {
            return;
        };
        write_many(&HistoryFile::new(path), HistorySource::Mcp);
    }

    fn write_many(file: &HistoryFile, source: HistorySource) {
        let history = file.open().unwrap();
        let shop = DataSourceId::generate();
        for at in 0..WRITES_PER_PROCESS {
            history
                .record(&HistoryEntry {
                    source,
                    ..entry(shop, at, "SELECT 1")
                })
                .unwrap();
        }
    }

    /// The app and `dbdesk mcp` are separate processes, and SQLite's locks between processes
    /// differ from those between connections in one; so the second writer here really is
    /// another process: this test binary again, running [`write_as_second_process`].
    #[test]
    fn two_processes_write_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = HistoryFile::new(dir.path().join("history.sqlite"));
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "history::tests::write_as_second_process",
                "--ignored",
                "--quiet",
            ])
            .env(CHILD_PATH, file.path())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();

        write_many(&file, HistorySource::App);
        assert!(child.wait().unwrap().success(), "the second process failed");

        let counts: Vec<(String, i64)> = rusqlite::Connection::open(file.path())
            .unwrap()
            .prepare("SELECT source, count(*) FROM history GROUP BY source ORDER BY source")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            counts,
            [
                ("app".to_owned(), WRITES_PER_PROCESS),
                ("mcp".to_owned(), WRITES_PER_PROCESS)
            ]
        );
    }
}
