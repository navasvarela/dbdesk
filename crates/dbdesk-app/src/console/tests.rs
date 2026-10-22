use super::*;

fn console(text: &str) -> QueryConsole {
    QueryConsole::new(
        ConsoleId(1),
        ConsoleKey {
            data_source: DataSourceId::generate(),
            name: dbdesk_store::ConsoleName::numbered(1),
        },
        "test".to_owned(),
        Dialect::Postgres,
        text.to_owned(),
    )
}

fn sqls(statements: &[SubmittedStatement]) -> Vec<&str> {
    statements.iter().map(|s| s.sql.as_str()).collect()
}

#[test]
fn runs_the_selection_the_statement_under_the_cursor_or_everything() {
    let text = "select 1;\nselect 2;\nselect 3";
    let second = text.find("select 2").unwrap();
    let cases: [(Range<usize>, RunScope, &[&str]); 5] = [
        (second..second, RunScope::Current, &["select 2"]),
        // Just after the `;`, the statement before it is meant.
        (second - 1..second - 1, RunScope::Current, &["select 1"]),
        // A selection runs exactly what is selected, split into statements, in either
        // selection direction.
        (
            text.len()..second,
            RunScope::Current,
            &["select 2", "select 3"],
        ),
        (second..second + 6, RunScope::Current, &["select"]),
        (0..0, RunScope::All, &["select 1", "select 2", "select 3"]),
    ];

    for (selection, scope, expected) in cases {
        let mut console = console(text);
        console.selection = selection.clone();
        let statements = console.statements_to_run(scope);
        assert_eq!(sqls(&statements), expected, "{selection:?} {scope:?}");
        for statement in &statements {
            assert_eq!(&text[statement.range.clone()], statement.sql);
        }
    }
}

#[test]
fn follows_a_run_through_rows_failure_and_completion() {
    let text = "select n from t;\nselect broken";
    let mut console = console(text);
    let statements = console.statements_to_run(RunScope::All);
    let run = RunId(7);
    console.begin_run(run, &statements);
    assert!(console.is_running());

    console.apply(&Event::StatementStarted {
        run,
        statement: 0,
        columns: vec![ColumnInfo {
            name: "n".to_owned(),
            type_name: None,
        }],
    });
    console.apply(&Event::RowsFetched {
        run,
        statement: 0,
        rows: vec![vec![Some("1".to_owned())]],
        more: true,
    });
    assert_eq!(console.request_more(), Some(run));
    assert_eq!(console.request_more(), None, "a fetch is already in flight");
    console.apply(&Event::StatementFinished {
        run,
        statement: 1,
        result: Err(StatementFailure {
            message: "syntax error".to_owned(),
            position: Some("select ".len()),
        }),
    });
    console.apply(&Event::RunFinished {
        run,
        outcome: RunOutcome::Failed,
        elapsed: Duration::from_millis(3),
    });

    assert!(!console.is_running());
    let marker = console.error_marker.clone().unwrap();
    assert_eq!(&text[marker], "broken");
    assert_eq!(console.run.as_ref().unwrap().selected, 1);
}

#[test]
fn ignores_events_from_older_runs() {
    let mut console = console("select 1");
    let statements = console.statements_to_run(RunScope::All);
    console.begin_run(RunId(1), &statements);
    console.begin_run(RunId(2), &statements);

    console.apply(&Event::RunFinished {
        run: RunId(1),
        outcome: RunOutcome::Completed,
        elapsed: Duration::ZERO,
    });

    assert!(console.is_running());
}

#[test]
fn a_lost_connection_fails_the_running_statement() {
    let mut console = console("select 1; select 2");
    let statements = console.statements_to_run(RunScope::All);
    console.begin_run(RunId(1), &statements);

    console.apply(&Event::ConsoleSessionChanged {
        console: ConsoleId(1),
        state: SessionState::Failed("could not connect: refused".to_owned()),
    });

    assert!(!console.is_running());
    let first = &console.run.as_ref().unwrap().statements[0];
    assert!(matches!(&first.result, Some(Err(failure)) if failure.message.contains("refused")));
}
