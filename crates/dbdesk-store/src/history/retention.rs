//! Deleting the statements the retention setting no longer keeps, beside other processes
//! writing the same file.
//!
//! Each batch is its own short transaction: another process's write waits for one batch,
//! never for the whole pruning, and its [`BUSY_TIMEOUT`](super::BUSY_TIMEOUT) is never near.
//! A pruning cut short (a crash, the app quitting) leaves a consistent file with less left to
//! prune next time.

use jiff::{SignedDuration, Timestamp};
use rusqlite::params;

use super::{History, HistoryError, format_timestamp};
use crate::HistorySettings;

/// Rows deleted per transaction. A few milliseconds of deleting, measured on a laptop: long
/// enough that a large pruning isn't mostly transaction overhead, short enough that a write
/// from another process waiting behind it doesn't notice.
const BATCH: i64 = 2_000;

const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

impl History {
    /// Deletes the statements `retention` no longer keeps, counting age back from `now`.
    /// Returns how many went.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Database`] if a batch can't be deleted; those before it stay deleted.
    pub fn prune(&self, retention: &HistorySettings, now: Timestamp) -> Result<u64, HistoryError> {
        let mut deleted = 0;
        if let Some(days) = retention.max_age_days {
            let age = SignedDuration::from_secs(i64::from(days.get()) * SECONDS_PER_DAY);
            // A cut-off before the earliest timestamp can't be older than any row.
            if let Ok(cutoff) = now.checked_sub(age) {
                deleted += self.delete_in_batches(
                    "DELETE FROM history WHERE id IN
                         (SELECT id FROM history WHERE executed_at < ?1 LIMIT ?2)",
                    &format_timestamp(cutoff),
                    None,
                )?;
            }
        }
        if let Some(kept) = retention.max_per_data_source {
            for data_source in self.data_sources()? {
                deleted += self.keep_newest(&data_source, i64::from(kept.get()))?;
            }
        }
        Ok(deleted)
    }

    /// Every data source with statements in the file, as stored.
    fn data_sources(&self) -> Result<Vec<String>, HistoryError> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT DISTINCT data_source FROM history")?;
        let data_sources = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(data_sources)
    }

    /// Deletes all but the newest `kept` statements of `data_source`: everything from the
    /// first one past them, on `history_by_source`.
    fn keep_newest(&self, data_source: &str, kept: i64) -> Result<u64, HistoryError> {
        let first_gone: Option<(String, i64)> = self
            .connection
            .prepare_cached(
                "SELECT executed_at, id FROM history WHERE data_source = ?1
                 ORDER BY executed_at DESC, id DESC LIMIT 1 OFFSET ?2",
            )?
            .query_row(params![data_source, kept], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        let Some((executed_at, id)) = first_gone else {
            return Ok(0);
        };
        self.delete_in_batches(
            "DELETE FROM history WHERE id IN
                 (SELECT id FROM history WHERE data_source = ?3
                    AND (executed_at < ?1 OR (executed_at = ?1 AND id <= ?4))
                  LIMIT ?2)",
            &executed_at,
            Some((data_source, id)),
        )
    }

    /// Runs `delete` (with the cut-off as `?1`, the batch size as `?2`, and the data source and
    /// id as `?3` and `?4` where given) until a batch deletes less than a whole one.
    fn delete_in_batches(
        &self,
        delete: &str,
        cutoff: &str,
        row: Option<(&str, i64)>,
    ) -> Result<u64, HistoryError> {
        let mut statement = self.connection.prepare_cached(delete)?;
        let mut deleted = 0;
        loop {
            let batch = match row {
                Some((data_source, id)) => {
                    statement.execute(params![cutoff, BATCH, data_source, id])?
                }
                None => statement.execute(params![cutoff, BATCH])?,
            };
            deleted += u64::try_from(batch).unwrap_or(u64::MAX);
            if i64::try_from(batch).unwrap_or(i64::MAX) < BATCH {
                return Ok(deleted);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::time::Duration;

    use dbdesk_core::DataSourceId;

    use super::*;
    use crate::{HistoryEntry, HistoryFile, HistoryOutcome, HistoryQuery, HistorySource};

    const DAY_MS: i64 = SECONDS_PER_DAY * 1_000;

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_millisecond(ms).unwrap()
    }

    fn entry(data_source: DataSourceId, executed_at: Timestamp, sql: &str) -> HistoryEntry {
        HistoryEntry {
            data_source,
            executed_at,
            sql: sql.to_owned(),
            duration: Some(Duration::from_millis(1)),
            rows: None,
            outcome: HistoryOutcome::Ok,
            source: HistorySource::Console,
        }
    }

    /// `count` rows of `data_source`, a millisecond apart from `first`, in one transaction.
    fn fill(file: &HistoryFile, data_source: DataSourceId, first: Timestamp, count: i64) {
        let mut connection = rusqlite::Connection::open(file.path()).unwrap();
        let transaction = connection.transaction().unwrap();
        for offset in 0..count {
            let executed_at = format_timestamp(at(first.as_millisecond() + offset));
            transaction
                .execute(
                    "INSERT INTO history (data_source, executed_at, sql, outcome, origin)
                     VALUES (?1, ?2, 'SELECT 1', 'ok', 'console')",
                    params![data_source.to_string(), executed_at],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
    }

    fn sqls(history: &History, data_source: DataSourceId) -> Vec<String> {
        history
            .search(&HistoryQuery::all(data_source), 100)
            .unwrap()
            .into_iter()
            .map(|entry| entry.sql)
            .collect()
    }

    fn days(days: u32) -> Option<NonZeroU32> {
        NonZeroU32::new(days)
    }

    #[test]
    fn keeps_everything_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let history = HistoryFile::new(dir.path().join("history.sqlite"))
            .open()
            .unwrap();
        let shop = DataSourceId::generate();
        history.record(&entry(shop, at(0), "SELECT 1")).unwrap();
        assert_eq!(
            history
                .prune(&HistorySettings::default(), Timestamp::now())
                .unwrap(),
            (0)
        );
        assert_eq!(sqls(&history, shop), ["SELECT 1"]);
    }

    #[test]
    fn deletes_statements_older_than_the_age() {
        let dir = tempfile::tempdir().unwrap();
        let history = HistoryFile::new(dir.path().join("history.sqlite"))
            .open()
            .unwrap();
        let shop = DataSourceId::generate();
        let now = at(100 * DAY_MS);
        for (age, sql) in [
            (31 * DAY_MS, "old"),
            (30 * DAY_MS, "the limit"),
            (DAY_MS, "new"),
        ] {
            history
                .record(&entry(shop, at(now.as_millisecond() - age), sql))
                .unwrap();
        }
        let retention = HistorySettings {
            max_age_days: days(30),
            ..HistorySettings::default()
        };

        assert_eq!(history.prune(&retention, now).unwrap(), (1));
        assert_eq!(sqls(&history, shop), ["new", "the limit"]);
    }

    /// The newest of each data source stay, rows run in the same millisecond told apart by
    /// the order they were recorded in.
    #[test]
    fn keeps_the_newest_of_each_data_source() {
        let dir = tempfile::tempdir().unwrap();
        let history = HistoryFile::new(dir.path().join("history.sqlite"))
            .open()
            .unwrap();
        let (shop, crm) = (DataSourceId::generate(), DataSourceId::generate());
        for (ms, sql) in [(1, "a"), (2, "b"), (2, "c"), (3, "d")] {
            history.record(&entry(shop, at(ms), sql)).unwrap();
        }
        history.record(&entry(crm, at(0), "only")).unwrap();
        let retention = HistorySettings {
            max_per_data_source: NonZeroU32::new(2),
            ..HistorySettings::default()
        };

        assert_eq!(history.prune(&retention, Timestamp::now()).unwrap(), (2));
        assert_eq!(sqls(&history, shop), ["d", "c"]);
        assert_eq!(sqls(&history, crm), ["only"]);
    }

    /// Many batches, by age and by count at once.
    #[test]
    fn prunes_more_than_a_batch() {
        let dir = tempfile::tempdir().unwrap();
        let file = HistoryFile::new(dir.path().join("history.sqlite"));
        let history = file.open().unwrap();
        let shop = DataSourceId::generate();
        let now = at(10 * DAY_MS);
        fill(&file, shop, at(0), 2 * BATCH + 500);
        fill(&file, shop, at(9 * DAY_MS), 2 * BATCH + 500);
        let retention = HistorySettings {
            max_age_days: days(5),
            max_per_data_source: NonZeroU32::new(1_000),
        };

        let total = u64::try_from(4 * BATCH + 1_000).unwrap();
        assert_eq!(history.prune(&retention, now).unwrap(), (total - 1_000));
        assert_eq!(
            history
                .search(&HistoryQuery::all(shop), 5_000)
                .unwrap()
                .len(),
            1_000
        );
    }

    /// Set to a history file's path in the child process of
    /// [`prunes_while_another_process_writes`].
    const CHILD_PATH: &str = "DBDESK_HISTORY_TEST_RECENT_CHILD_PATH";
    const RECENT_WRITES: usize = 200;

    /// Runs only as the child of [`prunes_while_another_process_writes`]: recent statements,
    /// which pruning by age keeps.
    #[test]
    #[ignore = "a helper process for prunes_while_another_process_writes"]
    fn write_recent_as_second_process() {
        let Ok(path) = std::env::var(CHILD_PATH) else {
            return;
        };
        let history = HistoryFile::new(path).open().unwrap();
        let crm = DataSourceId::generate();
        for _ in 0..RECENT_WRITES {
            history
                .record(&entry(crm, Timestamp::now(), "recent"))
                .unwrap();
        }
    }

    /// The app prunes while `dbdesk mcp` goes on recording: neither waits long enough to fail,
    /// and only the old statements go.
    #[test]
    fn prunes_while_another_process_writes() {
        let dir = tempfile::tempdir().unwrap();
        let file = HistoryFile::new(dir.path().join("history.sqlite"));
        let history = file.open().unwrap();
        let shop = DataSourceId::generate();
        let old = 5 * BATCH;
        fill(&file, shop, at(0), old);
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "history::retention::tests::write_recent_as_second_process",
                "--ignored",
                "--quiet",
            ])
            .env(CHILD_PATH, file.path())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();

        let retention = HistorySettings {
            max_age_days: days(1),
            ..HistorySettings::default()
        };
        let pruned = history.prune(&retention, Timestamp::now());
        // libtest prints a failed test's panic to stdout, so that is where the child's is.
        let child = child.wait_with_output().unwrap();
        assert!(
            child.status.success(),
            "the second process failed:\n{}",
            String::from_utf8_lossy(&child.stdout)
        );
        assert_eq!(pruned.unwrap(), u64::try_from(old).unwrap());
        let left: Vec<String> = history
            .search(&HistoryQuery::everywhere(), 1_000)
            .unwrap()
            .into_iter()
            .map(|entry| entry.sql)
            .collect();
        assert_eq!(left, vec!["recent"; RECENT_WRITES]);
    }
}
