//! The history as an audit trail (W4), driven through the whole app against SQLite.

use dbdesk_store::{HistoryEntry, HistoryFile, HistoryOutcome, HistorySource};

use super::*;

/// Writes entries straight into the history file, before the app starts, as another process
/// would.
fn record(dir: &Path, data_source: dbdesk_core::DataSourceId, entries: &[(&str, HistorySource)]) {
    let history = HistoryFile::new(dir.join("history.sqlite")).open().unwrap();
    for (at, (sql, source)) in (1_i64..).zip(entries) {
        history
            .record(&HistoryEntry {
                data_source,
                executed_at: jiff::Timestamp::from_second(at).unwrap(),
                sql: (*sql).to_owned(),
                duration: None,
                rows: Some(1),
                outcome: HistoryOutcome::Ok,
                source: source.clone(),
            })
            .unwrap();
    }
}

fn open_history(harness: &mut Harness<'_, DbDeskApp>, entries: usize) {
    harness.get_by_label("History").click();
    step_until(harness, "the history loading", |harness| {
        harness.query_all_by_label("Insert").count() == entries
    });
}

fn choose(harness: &mut Harness<'_, DbDeskApp>, menu: &str, choice: &str) {
    harness
        .get_by(|node| node.role() == Role::ComboBox && node.value().as_deref() == Some(menu))
        .click();
    harness.run();
    harness.get_by_label(choice).click();
}

#[test]
fn marks_each_statement_with_where_it_came_from_and_filters_by_it() {
    let dir = tempfile::tempdir().unwrap();
    let shop = save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
    record(
        dir.path(),
        shop.id,
        &[
            (
                "CREATE TABLE refunds (id INTEGER)",
                HistorySource::ObjectHelper,
            ),
            (
                "UPDATE \"orders\" SET \"id\" = '2' WHERE \"id\" = '1'",
                HistorySource::GridEdit,
            ),
            (
                "INSERT INTO \"orders\" (\"id\") VALUES\n\u{2026}",
                HistorySource::Import {
                    file: "/data/orders.csv".into(),
                },
            ),
            ("SELECT 'from before'", HistorySource::Dbdesk),
        ],
    );
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir.path(), feed);
    open_orders(&mut harness, "0 rows");

    // The four recorded, and the console's own SELECT.
    open_history(&mut harness, 5);
    for badge in ["Console", "Object", "Grid", "Import", "dbdesk"] {
        harness.get_by_label(badge);
    }
    choose(&mut harness, "All sources", "Grid edits");
    step_until(&mut harness, "the grid edits alone", |harness| {
        harness.query_all_by_label("Insert").count() == 1
    });
    harness.get_by_label_contains("UPDATE \"orders\"");
}
