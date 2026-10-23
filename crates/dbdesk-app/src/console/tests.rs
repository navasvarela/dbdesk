use super::*;

fn console(text: &str) -> QueryConsole {
    console_with(text, Dialect::Postgres)
}

fn console_with(text: &str, dialect: Dialect) -> QueryConsole {
    QueryConsole::new(
        ConsoleId(1),
        ConsoleKey {
            data_source: DataSourceId::generate(),
            name: dbdesk_store::ConsoleName::numbered(1),
        },
        "test".to_owned(),
        dialect,
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

#[test]
fn inserts_a_statement_on_a_line_of_its_own_at_the_cursor() {
    let cases: [(&str, Range<usize>, &str, &str); 3] = [
        ("", 0..0, "select 1", "select 1;\n"),
        ("select 1;", 9..9, "select 2;", "select 1;\nselect 2;\n"),
        // A selection is replaced.
        ("select 1;\nold;\n", 10..14, "new  ", "select 1;\nnew;\n\n"),
    ];
    for (text, selection, sql, expected) in cases {
        let mut console = console(text);
        console.selection = selection;

        console.insert_statement(sql, Instant::now());

        assert_eq!(console.text, expected, "{text:?}");
        assert_eq!(console.pending_cursor, Some(console.selection.end));
        assert!(console.file.unsaved_since.is_some());
    }
}

#[test]
fn explains_the_statement_at_the_cursor_and_marks_errors_in_it() {
    let text = "select 1;\nselect broken from;";
    let second = text.find("select broken").unwrap();
    let mut console = console(text);
    console.selection = second..second;

    let statements = console.statements_to_run(RunScope::Explain);

    assert_eq!(sqls(&statements), ["EXPLAIN select broken from"]);
    console.begin_run(RunId(1), &statements);
    console.apply(&Event::StatementFinished {
        run: RunId(1),
        statement: 0,
        result: Err(StatementFailure {
            message: "syntax error".to_owned(),
            position: Some("EXPLAIN select broken ".len()),
        }),
    });
    assert_eq!(&text[console.error_marker.clone().unwrap()], "from");
}

#[test]
fn shades_the_statement_at_the_cursor_unless_text_is_selected() {
    let text = "select 1;\nselect 2;";
    let second = text.find("select 2").unwrap();
    let mut console = console(text);

    console.selection = second + 3..second + 3;
    assert_eq!(console.current_statement(), Some(second..text.len() - 1));

    console.selection = 0..4;
    assert_eq!(console.current_statement(), None);
}

#[test]
fn completion_replaces_the_typed_prefix_and_closes() {
    let mut console = console("select * from t where SEL");
    console.selection = console.text.len()..console.text.len();

    console.open_completion(false);
    let _ = console.refresh_completion(None);
    assert!(
        console.completion.is_some(),
        "keywords match without a catalog"
    );
    console.accept_completion(Instant::now());

    assert_eq!(console.text, "select * from t where SELECT");
    assert!(console.completion.is_none());
    assert_eq!(console.pending_cursor, Some(console.text.len()));

    console.open_completion(false);
    let _ = console.refresh_completion(None);
    assert!(
        console.completion.is_none(),
        "nothing starts with SELECT but SELECT itself"
    );
}

#[test]
fn formats_the_statement_at_the_cursor_and_keeps_the_cursor_on_its_token() {
    let text = "select 1;\nselect a,b from t where x=1;";
    let mut console = console(text);
    let cursor = text.find("from").unwrap() + 2;
    console.selection = cursor..cursor;

    console.format(
        FormatScope::Current,
        &FormatOptions::default(),
        Instant::now(),
    );

    let expected = "select 1;\nSELECT\n    a,\n    b\nFROM t\nWHERE x = 1;";
    assert_eq!(console.text, expected);
    let moved = expected.find("FROM").unwrap() + 2;
    assert_eq!(console.selection, moved..moved);
    assert_eq!(console.pending_cursor, Some(moved));
    assert_eq!(console.format_undo.as_deref(), Some(text));
    assert!(console.file.unsaved_since.is_some());
}

#[test]
fn formats_everything_or_only_whole_tokens_of_the_selection() {
    let text = "select 'a,  b', c from t; select d,e from u";
    let mut console = console(text);
    console.format(FormatScope::All, &FormatOptions::default(), Instant::now());
    assert_eq!(
        console.text,
        "SELECT\n    'a,  b',\n    c\nFROM t; SELECT\n    d,\n    e\nFROM u"
    );

    // A selection starting inside the string formats from the token after it; a fragment
    // that isn't a whole query only has its keywords' case changed.
    let mut console = console_with(text, Dialect::Postgres);
    let start = text.find("  b").unwrap();
    console.selection = start..text.find(';').unwrap();
    console.format(
        FormatScope::Current,
        &FormatOptions::default(),
        Instant::now(),
    );
    assert_eq!(console.text, "select 'a,  b', c FROM t; select d,e from u");
}

#[test]
fn a_format_that_changes_nothing_leaves_no_undo_point() {
    let text = "SELECT 1";
    let mut console = console(text);
    console.format(FormatScope::All, &FormatOptions::default(), Instant::now());
    assert_eq!(console.text, text);
    assert!(console.format_undo.is_none());
    assert!(console.pending_cursor.is_none());
}
