use tracing::Level;

use super::LogEntry;

/// What the log console currently shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFilter {
    /// The most verbose level shown. `tracing` orders levels by verbosity, so
    /// `Level::ERROR < Level::WARN < … < Level::TRACE`.
    pub verbosity: Level,
    /// Case-insensitive text matched against message, target and fields. Empty matches all.
    pub search: String,
}

impl Default for LogFilter {
    fn default() -> Self {
        Self {
            verbosity: Level::INFO,
            search: String::new(),
        }
    }
}

impl LogFilter {
    pub fn matches(&self, entry: &LogEntry) -> bool {
        entry.level <= self.verbosity && self.matches_search(entry)
    }

    fn matches_search(&self, entry: &LogEntry) -> bool {
        if self.search.is_empty() {
            return true;
        }
        let needle = self.search.to_lowercase();
        [&entry.message, &entry.target, &entry.fields]
            .into_iter()
            .any(|haystack| haystack.to_lowercase().contains(&needle))
    }
}

#[cfg(test)]
mod tests {
    use jiff::Zoned;

    use super::*;

    fn entry(level: Level, message: &str) -> LogEntry {
        LogEntry {
            time: Zoned::now(),
            level,
            target: "dbrover_engine::command_loop".to_owned(),
            message: message.to_owned(),
            fields: "rows=3".to_owned(),
        }
    }

    #[test]
    fn matches_by_verbosity_and_search() {
        let cases = [
            // (verbosity, search, entry level, matches)
            (Level::INFO, "", Level::ERROR, true),
            (Level::INFO, "", Level::INFO, true),
            (Level::INFO, "", Level::DEBUG, false),
            (Level::TRACE, "", Level::TRACE, true),
            (Level::INFO, "ENGINE", Level::INFO, true),
            (Level::INFO, "rows=3", Level::INFO, true),
            (Level::INFO, "started", Level::INFO, true),
            (Level::INFO, "postgres", Level::INFO, false),
            (Level::INFO, "started", Level::DEBUG, false),
        ];

        for (verbosity, search, level, expected) in cases {
            let filter = LogFilter {
                verbosity,
                search: search.to_owned(),
            };
            assert_eq!(
                filter.matches(&entry(level, "engine started")),
                expected,
                "verbosity={verbosity} search={search:?} level={level}"
            );
        }
    }
}
