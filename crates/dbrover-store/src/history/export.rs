//! Writing a search of the history, every match of it, to a CSV or JSON file.

use std::collections::HashMap;
use std::path::Path;

use dbrover_core::{DataSourceId, Row};
use dbrover_export::{ExportColumn, ExportError, ExportFormat, ExportWriter};

use super::{History, HistoryEntry, HistoryError, HistoryQuery, HistorySource, format_timestamp};

/// Entries written to the file at a time: what the export holds in memory.
const PAGE: usize = 500;

/// The file's columns, in order; [`row`] writes them.
const COLUMNS: [&str; 10] = [
    "executed_at",
    "data_source",
    "data_source_id",
    "source",
    "detail",
    "outcome",
    "duration_ms",
    "rows",
    "sql",
    "change",
];

/// How a data source no longer saved is named in the file.
const REMOVED_DATA_SOURCE: &str = "(removed)";

#[derive(Debug, thiserror::Error)]
pub enum HistoryExportError {
    #[error(transparent)]
    Read(#[from] HistoryError),
    #[error(transparent)]
    Write(#[from] ExportError),
}

impl History {
    /// Writes every statement `query` finds, newest first, to `path`, naming data sources by
    /// `names`. Returns how many were written. Nothing is left at `path` if it fails.
    ///
    /// # Errors
    ///
    /// [`HistoryExportError`] if the history can't be read or the file written.
    pub fn export(
        &self,
        query: &HistoryQuery,
        target: (&Path, ExportFormat),
        names: &HashMap<DataSourceId, String>,
    ) -> Result<usize, HistoryExportError> {
        let (path, format) = target;
        let columns = COLUMNS.map(ExportColumn::text);
        let mut writer = ExportWriter::create(path, &format.default_spec(), &columns)?;
        let mut page = Vec::with_capacity(PAGE);
        let written = self.each_match(query, None, |entry| {
            page.push(row(&entry, names));
            if page.len() == PAGE {
                writer.write(&page)?;
                page.clear();
            }
            Ok::<_, HistoryExportError>(())
        });
        match written.and_then(|()| Ok(writer.write(&page)?)) {
            Ok(()) => {
                let rows = writer.rows();
                writer.finish()?;
                Ok(rows)
            }
            Err(error) => {
                writer.abandon();
                Err(error)
            }
        }
    }
}

/// One entry as the file's columns. Values are text, as the grid's exports are.
fn row(entry: &HistoryEntry, names: &HashMap<DataSourceId, String>) -> Row {
    let name = names
        .get(&entry.data_source)
        .map_or(REMOVED_DATA_SOURCE, String::as_str);
    let change = match &entry.source {
        HistorySource::GridEdit(Some(change)) => Some(change.to_json()),
        _ => None,
    };
    vec![
        Some(format_timestamp(entry.executed_at)),
        Some(name.to_owned()),
        Some(entry.data_source.to_string()),
        Some(entry.source.kind().as_str().to_owned()),
        entry.source.detail(),
        Some(entry.outcome.as_str().to_owned()),
        entry
            .duration
            .map(|duration| duration.as_millis().to_string()),
        entry.rows.map(|rows| rows.to_string()),
        Some(entry.sql.clone()),
        change,
    ]
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use jiff::Timestamp;

    use super::*;
    use crate::{HistoryFile, HistoryOutcome, NamedCell, RowChange, RowChangeKind};

    fn entry(data_source: DataSourceId, at_ms: i64, sql: &str) -> HistoryEntry {
        HistoryEntry {
            data_source,
            executed_at: Timestamp::from_millisecond(at_ms).unwrap(),
            sql: sql.to_owned(),
            duration: Some(Duration::from_millis(7)),
            rows: Some(1),
            outcome: HistoryOutcome::Ok,
            source: HistorySource::Console,
        }
    }

    #[test]
    fn writes_a_search_as_csv_and_json() {
        let dir = tempfile::tempdir().unwrap();
        let history = HistoryFile::new(dir.path().join("history.sqlite"))
            .open()
            .unwrap();
        let (shop, removed) = (DataSourceId::generate(), DataSourceId::generate());
        let edit = HistoryEntry {
            source: HistorySource::GridEdit(Some(RowChange {
                table: "main.people".to_owned(),
                kind: RowChangeKind::Inserted {
                    values: vec![NamedCell {
                        column: "name".to_owned(),
                        value: Some("ada".to_owned()),
                    }],
                },
                revert: None,
            })),
            ..entry(shop, 2_000, "INSERT INTO people (name) VALUES ('ada')")
        };
        history
            .record(&entry(removed, 1_000, "SELECT 1, 2"))
            .unwrap();
        history.record(&edit).unwrap();
        let names = HashMap::from([(shop, "shop".to_owned())]);

        let csv = dir.path().join("history.csv");
        let written = history
            .export(
                &HistoryQuery::everywhere(),
                (&csv, ExportFormat::Csv),
                &names,
            )
            .unwrap();
        assert_eq!(written, 2);
        assert_eq!(
            std::fs::read_to_string(&csv).unwrap(),
            format!(
                "executed_at,data_source,data_source_id,source,detail,outcome,duration_ms,rows,sql,change\r\n\
                 1970-01-01T00:00:02.000Z,shop,{shop},grid,,ok,7,1,INSERT INTO people (name) VALUES ('ada'),\
                 \"{{\"\"table\"\":\"\"main.people\"\",\"\"change\"\":\"\"inserted\"\",\"\"values\"\":[{{\"\"column\"\":\"\"name\"\",\"\"value\"\":\"\"ada\"\"}}]}}\"\r\n\
                 1970-01-01T00:00:01.000Z,(removed),{removed},console,,ok,7,1,\"SELECT 1, 2\",\r\n"
            )
        );

        let json = dir.path().join("history.json");
        let only_shop = HistoryQuery::all(shop);
        history
            .export(&only_shop, (&json, ExportFormat::Json), &names)
            .unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(parsed.as_array().map(Vec::len), Some(1));
        assert_eq!(parsed[0]["source"], "grid");
        assert_eq!(parsed[0]["data_source"], "shop");
        assert_eq!(parsed[0]["detail"], serde_json::Value::Null);
    }

    /// More entries than a page, without the search's usual limit.
    #[test]
    fn writes_every_match_a_page_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let history = HistoryFile::new(dir.path().join("history.sqlite"))
            .open()
            .unwrap();
        let shop = DataSourceId::generate();
        let count = 2 * PAGE + 3;
        for at in 0..count {
            history
                .record(&entry(shop, i64::try_from(at).unwrap(), "SELECT 1"))
                .unwrap();
        }
        let path = dir.path().join("history.json");
        let written = history
            .export(
                &HistoryQuery::all(shop),
                (&path, ExportFormat::Json),
                &HashMap::new(),
            )
            .unwrap();
        assert_eq!(written, count);
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed.as_array().map(Vec::len), Some(count));
    }

    #[test]
    fn leaves_no_file_when_it_cant_write() {
        let dir = tempfile::tempdir().unwrap();
        let history = HistoryFile::new(dir.path().join("history.sqlite"))
            .open()
            .unwrap();
        let path = dir.path().join("missing folder/history.csv");
        let failed = history.export(
            &HistoryQuery::everywhere(),
            (&path, ExportFormat::Csv),
            &HashMap::new(),
        );
        assert!(matches!(failed, Err(HistoryExportError::Write(_))));
        assert!(!path.exists());
    }
}
