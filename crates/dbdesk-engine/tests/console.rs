//! Query console runs, driven headlessly: statements, paging, failures and cancellation.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::{TestEngine, postgres_test_server, sqlite_data_source, sqlite_database};
use dbdesk_core::TransactionStatus;
use dbdesk_core::{ConnectionConfig, DataSource, DataSourceId};
use dbdesk_engine::{
    Command, ConsoleId, Event, PasswordUpdate, RunId, RunOutcome, SessionState, StatementFailure,
    TransactionEnd, TransactionMode,
};
use dbdesk_store::HistoryOutcome;

const CONSOLE: ConsoleId = ConsoleId(1);
const ROWS: usize = 1200;

/// An engine with one saved SQLite data source, and its id.
fn engine_with_sqlite(dir: &std::path::Path) -> (TestEngine, DataSourceId) {
    let mut engine = TestEngine::start(&dir.join("data_sources.toml"), Arc::default());
    engine.expect_started(vec![]);
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
    execute_in(engine, data_source, run, statements, TransactionMode::Auto);
}

fn execute_in(
    engine: &TestEngine,
    data_source: DataSourceId,
    run: u64,
    statements: &[&str],
    transactions: TransactionMode,
) {
    engine.send(Command::Execute {
        console: CONSOLE,
        data_source,
        run: RunId(run),
        statements: statements.iter().map(|s| (*s).to_owned()).collect(),
        transactions,
    });
}

fn expect_transaction(engine: &mut TestEngine, expected: TransactionStatus) {
    match next_run_event(engine) {
        Event::TransactionChanged { console, status } if console == CONSOLE => {
            assert_eq!(status, expected);
        }
        other => panic!("expected the transaction to become {expected:?}, got {other:?}"),
    }
}

/// Skips the run's remaining events up to and including its end.
fn finish_run(engine: &mut TestEngine, run: u64) {
    loop {
        if let Event::RunFinished { run: RunId(r), .. } = next_run_event(engine)
            && r == run
        {
            return;
        }
    }
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

#[test]
fn records_each_statement_in_the_history() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, id) = engine_with_sqlite(dir.path());
    execute(&engine, id, 1, &["SELECT 1", "SELECT * FROM missing"]);
    while !matches!(next_run_event(&mut engine), Event::RunFinished { .. }) {}

    engine.send(Command::SearchHistory {
        data_source: id,
        text: "select".to_owned(),
    });

    let Event::HistoryFound {
        data_source,
        text,
        result: Ok(entries),
    } = next_run_event(&mut engine)
    else {
        panic!("expected history entries");
    };
    assert_eq!((data_source, text.as_str()), (id, "select"));
    let found: Vec<(&str, HistoryOutcome, Option<u64>)> = entries
        .iter()
        .map(|entry| (entry.sql.as_str(), entry.outcome, entry.rows))
        .collect();
    assert_eq!(
        found,
        [
            ("SELECT * FROM missing", HistoryOutcome::Error, None),
            ("SELECT 1", HistoryOutcome::Ok, None),
        ]
    );
}

#[test]
fn manual_mode_keeps_changes_pending_until_rolled_back() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, id) = engine_with_sqlite(dir.path());

    execute_in(
        &engine,
        id,
        1,
        &["SELECT count(*) FROM t", "INSERT INTO t VALUES (1)"],
        TransactionMode::Manual,
    );
    // The transaction opens before the first statement, even one that only reads.
    expect_transaction(&mut engine, TransactionStatus::InTransaction);
    finish_run(&mut engine, 1);

    engine.send(Command::EndTransaction {
        console: CONSOLE,
        end: TransactionEnd::Rollback,
    });
    expect_transaction(&mut engine, TransactionStatus::Idle);
    assert_eq!(
        next_run_event(&mut engine),
        Event::TransactionEnded {
            console: CONSOLE,
            result: Ok(TransactionEnd::Rollback),
        }
    );

    execute(&engine, id, 2, &["SELECT count(*) FROM t"]);
    assert!(matches!(
        next_run_event(&mut engine),
        Event::StatementStarted { .. }
    ));
    match next_run_event(&mut engine) {
        Event::RowsFetched { rows, .. } => assert_eq!(rows, [[Some("0".to_owned())]]),
        other => panic!("expected the count, got {other:?}"),
    }
}

#[test]
fn notices_a_transaction_the_user_begins_and_disconnecting_ends_it() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, id) = engine_with_sqlite(dir.path());

    execute(&engine, id, 1, &["BEGIN", "INSERT INTO t VALUES (1)"]);
    assert!(matches!(
        next_run_event(&mut engine),
        Event::StatementFinished {
            statement: 0,
            result: Ok(_),
            ..
        }
    ));
    expect_transaction(&mut engine, TransactionStatus::InTransaction);
    finish_run(&mut engine, 1);

    engine.send(Command::Disconnect(id));
    assert_eq!(
        engine.next_event(),
        Event::ConsoleSessionChanged {
            console: CONSOLE,
            state: SessionState::Disconnected,
        }
    );
    let rows: i64 = rusqlite::Connection::open(dir.path().join("app.db"))
        .unwrap()
        .query_row("SELECT count(*) FROM t", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "the insert was rolled back");
}

/// Needs `DBDESK_TEST_POSTGRES_URL`; skipped without it.
#[test]
fn committing_a_failed_postgres_transaction_rolls_it_back() {
    let Some(server) = postgres_test_server() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut engine = TestEngine::start(&dir.path().join("data_sources.toml"), Arc::default());
    engine.expect_started(vec![]);
    let data_source = DataSource {
        id: DataSourceId::generate(),
        name: "test server".to_owned(),
        config: ConnectionConfig::Postgres(server.config),
    };
    let id = data_source.id;
    engine.send(Command::SaveDataSource {
        data_source,
        password: PasswordUpdate::Set(server.password),
    });
    assert!(matches!(engine.next_event(), Event::DataSourceSaved(_)));

    execute_in(&engine, id, 1, &["SELECT 1 / 0"], TransactionMode::Manual);
    let mut statuses = Vec::new();
    loop {
        match next_run_event(&mut engine) {
            Event::TransactionChanged { status, .. } => statuses.push(status),
            Event::RunFinished { outcome, .. } => {
                assert_eq!(outcome, RunOutcome::Failed);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(
        statuses,
        [TransactionStatus::InTransaction, TransactionStatus::Failed]
    );

    engine.send(Command::EndTransaction {
        console: CONSOLE,
        end: TransactionEnd::Commit,
    });
    expect_transaction(&mut engine, TransactionStatus::Idle);
    assert_eq!(
        next_run_event(&mut engine),
        Event::TransactionEnded {
            console: CONSOLE,
            result: Ok(TransactionEnd::Rollback),
        }
    );
}
