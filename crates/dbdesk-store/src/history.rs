//! Query history: every statement a console runs, in `history.sqlite`. Only the SQL and how
//! it went are kept, never the rows.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dbdesk_core::DataSourceId;
use jiff::Timestamp;
use rusqlite::params;

/// Lexically sortable and RFC 3339: fixed millisecond precision, always UTC. A variable
/// number of fraction digits would sort `…:00.5Z` before `…:00Z`.
const TIMESTAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS history (
    id            INTEGER PRIMARY KEY,
    data_source   TEXT NOT NULL,
    executed_at   TEXT NOT NULL,
    sql           TEXT NOT NULL,
    duration_ms   INTEGER,
    rows          INTEGER,
    outcome       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS history_by_source ON history (data_source, executed_at DESC);
";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub data_source: DataSourceId,
    pub executed_at: Timestamp,
    pub sql: String,
    pub duration: Option<Duration>,
    /// Rows changed or returned, when the database reported it.
    pub rows: Option<u64>,
    pub outcome: HistoryOutcome,
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
        let connection = rusqlite::Connection::open(&self.path).map_err(open_error)?;
        connection.execute_batch(SCHEMA).map_err(open_error)?;
        Ok(History { connection })
    }
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
            "INSERT INTO history (data_source, executed_at, sql, duration_ms, rows, outcome)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                entry.data_source.to_string(),
                format_timestamp(entry.executed_at),
                entry.sql,
                duration_ms,
                rows,
                entry.outcome.as_str(),
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
            "SELECT executed_at, sql, duration_ms, rows, outcome FROM history
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
}
