//! The history as an audit trail, through the engine on SQLite: where each statement came
//! from.

use dbdesk_core::TableName;
use dbdesk_engine::RowsImport;
use dbdesk_store::{ExportFormat, HistoryEntry, HistorySource, Separator};

use super::*;

/// Every entry of the data source, newest first.
fn history(engine: &mut TestEngine, data_source: DataSourceId) -> Vec<HistoryEntry> {
    engine.send(Command::SearchHistory(HistoryQuery::all(data_source)));
    loop {
        match next_answer(engine) {
            Event::HistoryFound {
                result: Ok(entries),
                ..
            } => return entries,
            Event::TransactionChanged { .. } => {}
            other => panic!("expected the history, got {other:?}"),
        }
    }
}

fn apply_ddl(engine: &mut TestEngine, data_source: DataSourceId, sql: &str) {
    engine.send(Command::ApplyDdl {
        data_source,
        statements: vec![sql.to_owned()],
    });
    loop {
        match next_answer(engine) {
            Event::DdlApplied { result, .. } => return result.unwrap(),
            Event::TransactionChanged { .. } => {}
            other => panic!("expected DdlApplied, got {other:?}"),
        }
    }
}

/// A console run, an object helper's DDL, a grid edit, an export of every row and a CSV import,
/// each recorded with where it came from; the import once, without its rows.
#[test]
fn records_where_each_statement_came_from() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, id) = engine_with_sqlite(dir.path());
    execute(&engine, id, 1, &["SELECT 1"]);
    finish_run(&mut engine, 1);
    apply_ddl(
        &mut engine,
        id,
        "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT)",
    );
    edits(
        &engine,
        &["INSERT INTO people (id, name) VALUES ('1', 'ada')"],
    );
    assert_eq!(expect_edits(&mut engine), Ok(()));
    let exported = dir.path().join("people.csv");
    let (_, written) = grid::export_all(
        &mut engine,
        2,
        "SELECT id, name FROM people",
        (&exported, ExportFormat::Csv),
    );
    assert_eq!(written, Ok(1));
    let imported = dir.path().join("more.csv");
    std::fs::write(&imported, "2,bob\n3,cy\n").unwrap();
    let (_, result) = grid::import_rows(
        &mut engine,
        id,
        RowsImport {
            table: TableName {
                schema: "main".to_owned(),
                name: "people".to_owned(),
            },
            file: imported.clone(),
            separator: Separator::Comma,
            header: false,
            mapping: vec![("id".to_owned(), 0), ("name".to_owned(), 1)],
        },
    );
    assert_eq!(result, Ok(2));

    let found: Vec<(HistorySource, String, Option<u64>)> = history(&mut engine, id)
        .into_iter()
        .map(|entry| (entry.source, entry.sql, entry.rows))
        .collect();
    assert_eq!(
        found,
        [
            (
                HistorySource::Import { file: imported },
                "INSERT INTO \"main\".\"people\" (\"id\", \"name\") VALUES\n\u{2026}".to_owned(),
                Some(2)
            ),
            (
                HistorySource::Export,
                "SELECT id, name FROM people".to_owned(),
                Some(1)
            ),
            (
                HistorySource::GridEdit,
                "INSERT INTO people (id, name) VALUES ('1', 'ada')".to_owned(),
                Some(1)
            ),
            (
                HistorySource::ObjectHelper,
                "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT)".to_owned(),
                None
            ),
            (HistorySource::Console, "SELECT 1".to_owned(), None),
        ]
    );
    engine.shut_down();
}
