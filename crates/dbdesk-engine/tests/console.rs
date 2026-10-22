//! Query console runs, driven headlessly: statements, paging, failures and cancellation.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::{TestEngine, sqlite_data_source, sqlite_database};
use dbdesk_core::DataSourceId;
use dbdesk_engine::{
    Command, ConsoleId, Event, PasswordUpdate, RunId, RunOutcome, StatementFailure,
};

const CONSOLE: ConsoleId = ConsoleId(1);
const ROWS: usize = 1200;

/// An engine with one saved SQLite data source, and its id.
fn engine_with_sqlite(dir: &std::path::Path) -> (TestEngine, DataSourceId) {
    let mut engine = TestEngine::start(&dir.join("data_sources.toml"), Arc::default());
    engine.expect(Event::Started);
    engine.expect(Event::DataSourcesLoaded(vec![]));
    let data_source = sqlite_data_source(sqlite_database(dir));
    let id = data_source.id;
    engine.send(Command::SaveDataSource {
        data_source,
        password: PasswordUpdate::Keep,
    });
    assert!(matches!(engine.next_event(), Event::DataSourceSaved(_)));
    (engine, id)
}

/// The next event that isn't a console connection state change.
fn next_run_event(engine: &mut TestEngine) -> Event {
    loop {
        match engine.next_event() {
            Event::ConsoleSessionChanged { .. } => {}
            event => return event,
        }
    }
}

fn execute(engine: &TestEngine, data_source: DataSourceId, run: u64, statements: &[&str]) {
    engine.send(Command::Execute {
        console: CONSOLE,
        data_source,
        run: RunId(run),
        statements: statements.iter().map(|s| (*s).to_owned()).collect(),
    });
}

fn expect_rows(engine: &mut TestEngine, run: u64, statement: usize) -> (usize, bool) {
    match next_run_event(engine) {
        Event::RowsFetched {
            run: RunId(r),
            statement: s,
            rows,
            more,
        } if r == run && s == statement => (rows.len(), more),
        other => panic!("expected rows of statement {statement}, got {other:?}"),
    }
}

fn expect_statement_ok(engine: &mut TestEngine, run: u64, statement: usize) -> (Option<u64>, bool) {
    match next_run_event(engine) {
        Event::StatementFinished {
            run: RunId(r),
            statement: s,
            result: Ok(summary),
        } if r == run && s == statement => (summary.rows_changed, summary.truncated),
        other => panic!("expected statement {statement} to succeed, got {other:?}"),
    }
}

fn expect_run_finished(engine: &mut TestEngine, run: u64, expected: RunOutcome) {
    match next_run_event(engine) {
        Event::RunFinished {
            run: RunId(r),
            outcome,
            ..
        } if r == run => assert_eq!(outcome, expected),
        other => panic!("expected run {run} to finish, got {other:?}"),
    }
}

const SETUP: [&str; 2] = [
    "CREATE TABLE numbers (n INTEGER)",
    "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 1200) \
     INSERT INTO numbers SELECT n FROM s",
];

#[test]
fn runs_a_script_and_pages_through_the_last_result() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, id) = engine_with_sqlite(dir.path());

    execute(
        &engine,
        id,
        1,
        &[SETUP[0], SETUP[1], "SELECT n FROM numbers ORDER BY n"],
    );

    expect_statement_ok(&mut engine, 1, 0);
    assert_eq!(
        expect_statement_ok(&mut engine, 1, 1),
        (Some(ROWS as u64), false)
    );
    assert!(matches!(
        next_run_event(&mut engine),
        Event::StatementStarted { statement: 2, ref columns, .. } if columns[0].name == "n"
    ));
    assert_eq!(expect_rows(&mut engine, 1, 2), (500, true));
    expect_run_finished(&mut engine, 1, RunOutcome::Completed);

    engine.send(Command::FetchMore {
        console: CONSOLE,
        run: RunId(1),
    });
    assert_eq!(expect_rows(&mut engine, 1, 2), (500, true));
    engine.send(Command::FetchMore {
        console: CONSOLE,
        run: RunId(1),
    });
    assert_eq!(expect_rows(&mut engine, 1, 2), (200, false));
    let (_, truncated) = expect_statement_ok(&mut engine, 1, 2);
    assert!(!truncated);
}

#[test]
fn stops_at_the_first_failure_and_reports_where() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, id) = engine_with_sqlite(dir.path());

    execute(
        &engine,
        id,
        1,
        &["SELECT 1", "SELECT 2 FROM WHERE", "SELECT 3"],
    );

    assert!(matches!(
        next_run_event(&mut engine),
        Event::StatementStarted { statement: 0, .. }
    ));
    assert_eq!(expect_rows(&mut engine, 1, 0), (1, false));
    expect_statement_ok(&mut engine, 1, 0);
    match next_run_event(&mut engine) {
        Event::StatementFinished {
            statement: 1,
            result: Err(StatementFailure { message, position }),
            ..
        } => {
            assert!(message.contains("syntax error"), "{message}");
            assert_eq!(position, Some("SELECT 2 FROM ".len()));
        }
        other => panic!("expected statement 1 to fail, got {other:?}"),
    }
    expect_run_finished(&mut engine, 1, RunOutcome::Failed);
}

#[test]
fn cancels_a_running_statement() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, id) = engine_with_sqlite(dir.path());

    execute(
        &engine,
        id,
        1,
        &["WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c) SELECT count(*) FROM c"],
    );
    assert!(matches!(
        next_run_event(&mut engine),
        Event::StatementStarted { .. }
    ));
    engine.send(Command::Cancel {
        console: CONSOLE,
        run: RunId(1),
    });

    assert!(matches!(
        next_run_event(&mut engine),
        Event::StatementFinished { result: Err(_), .. }
    ));
    expect_run_finished(&mut engine, 1, RunOutcome::Cancelled);
}

#[test]
fn a_new_run_discards_the_previous_unread_rows() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, id) = engine_with_sqlite(dir.path());
    execute(
        &engine,
        id,
        1,
        &[SETUP[0], SETUP[1], "SELECT n FROM numbers"],
    );
    expect_statement_ok(&mut engine, 1, 0);
    expect_statement_ok(&mut engine, 1, 1);
    assert!(matches!(
        next_run_event(&mut engine),
        Event::StatementStarted { .. }
    ));
    assert_eq!(expect_rows(&mut engine, 1, 2), (500, true));
    expect_run_finished(&mut engine, 1, RunOutcome::Completed);

    execute(&engine, id, 2, &["SELECT count(*) FROM numbers"]);

    assert_eq!(expect_statement_ok(&mut engine, 1, 2), (None, true));
    assert!(matches!(
        next_run_event(&mut engine),
        Event::StatementStarted { run: RunId(2), .. }
    ));
    assert_eq!(expect_rows(&mut engine, 2, 0), (1, false));
    expect_statement_ok(&mut engine, 2, 0);
    expect_run_finished(&mut engine, 2, RunOutcome::Completed);
}
