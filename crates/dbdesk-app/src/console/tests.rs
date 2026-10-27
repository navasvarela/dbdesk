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
        assert!(
            matches!(&console.storage, file::Storage::Config(file) if file.unsaved_since.is_some())
        );
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

    let recent = RecentCompletions::default();
    console.open_completion(false);
    let _ = console.refresh_completion(None, &recent);
    assert!(
        console.completion.is_some(),
        "keywords match without a catalog"
    );
    console.accept_completion(Instant::now());

    assert_eq!(console.text, "select * from t where SELECT");
    assert!(console.completion.is_none());
    assert_eq!(console.pending_cursor, Some(console.text.len()));

    console.open_completion(false);
    let _ = console.refresh_completion(None, &recent);
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
    assert_eq!(console.replaced_text.as_deref(), Some(text));
    assert!(
        matches!(&console.storage, file::Storage::Config(file) if file.unsaved_since.is_some())
    );
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
    assert!(console.replaced_text.is_none());
    assert!(console.pending_cursor.is_none());
}

#[test]
fn accepting_a_function_puts_the_cursor_inside_its_parentheses_and_reports_it() {
    let mut console = console("select coal");
    console.selection = console.text.len()..console.text.len();
    let recent = RecentCompletions::default();

    console.open_completion(false);
    let _ = console.refresh_completion(None, &recent);
    console.accept_completion(Instant::now());

    assert_eq!(console.text, "select coalesce()");
    assert_eq!(console.pending_cursor, Some(console.text.len() - 1));
    let accepted = console.take_accepted_completion().unwrap();
    assert_eq!(accepted.label, "coalesce");
    assert!(console.take_accepted_completion().is_none());
}

#[test]
fn keeps_clickhouse_results_read_only_without_describing_the_table() {
    let cases = [(Dialect::Postgres, true), (Dialect::ClickHouse, false)];
    for (dialect, describes) in cases {
        let mut console = console_with("select n from t", dialect);
        let statements = console.statements_to_run(RunScope::All);
        let run = RunId(1);
        console.begin_run(run, &statements);
        console.apply(&Event::StatementStarted {
            run,
            statement: 0,
            columns: vec![ColumnInfo {
                name: "n".to_owned(),
                type_name: None,
            }],
        });

        let tables = console.tables_to_describe();

        assert_eq!(!tables.is_empty(), describes, "{dialect:?}");
        let editability = &console.run.as_ref().unwrap().statements[0]
            .edits
            .editability;
        assert_eq!(
            matches!(editability, Editability::ReadOnly(_)),
            !describes,
            "{dialect:?}: {editability:?}"
        );
    }
}

/// A read-only data source's results are never editable, but their table is still described,
/// so its foreign keys can be followed.
#[test]
fn describes_a_read_only_data_sources_tables_for_their_keys() {
    let mut console = console_with("select n from t", Dialect::Postgres);
    console.safety = SafetyLevel::ReadOnly;
    console.data_source_name = "production".to_owned();
    let statements = console.statements_to_run(RunScope::All);
    let run = RunId(1);
    console.begin_run(run, &statements);
    console.apply(&Event::StatementStarted {
        run,
        statement: 0,
        columns: vec![ColumnInfo {
            name: "n".to_owned(),
            type_name: None,
        }],
    });

    let tables = console.tables_to_describe();

    assert_eq!(tables.len(), 1);
    let statement = &console.run.as_ref().unwrap().statements[0];
    assert_eq!(
        statement.edits.editability,
        Editability::ReadOnly("\u{201c}production\u{201d} is read-only".to_owned())
    );
    assert_eq!(statement.describing.as_ref(), tables.first());
}

/// Sorting in the database runs the shown statement again, wrapped; its new result keeps the
/// statement as typed (which decides what can be edited) and the sort, and switching back
/// runs it as typed.
#[test]
fn runs_a_statement_again_wrapped_and_keeps_it_as_typed() {
    let mut console = console_with("select id, note from t;", Dialect::Postgres);
    let statements = console.statements_to_run(RunScope::All);
    console.begin_run(RunId(1), &statements);
    console.apply(&Event::StatementStarted {
        run: RunId(1),
        statement: 0,
        columns: ["id", "note"]
            .into_iter()
            .map(|name| ColumnInfo {
                name: name.to_owned(),
                type_name: None,
            })
            .collect(),
    });
    let grid = &mut console.run.as_mut().unwrap().statements[0].grid;
    grid.set_in_database(true);
    grid.toggle_sort(1);

    let wrapped = console.rearrange().unwrap();
    let statement = &wrapped.statements[0];
    assert_eq!(
        statement.sql,
        "SELECT * FROM (\nselect id, note from t\n) AS dbdesk_rows\nORDER BY 2 ASC"
    );
    assert_eq!(&statement.sql[statement.prefix_len..][..6], "select");
    console.begin_run(RunId(2), &wrapped.statements);
    let view = &console.run.as_ref().unwrap().statements[0];
    assert_eq!(view.sql, "select id, note from t");
    assert!(view.wrapped && view.grid.is_in_database());
    assert_eq!(view.grid.sort().map(|sort| sort.column), Some(1));

    console.run.as_mut().unwrap().statements[0]
        .grid
        .set_in_database(false);
    let as_typed = console.rearrange().unwrap();
    assert_eq!(as_typed.statements[0].sql, "select id, note from t");
    assert_eq!(as_typed.statements[0].in_database, None);
}

#[test]
fn finds_from_the_selection_and_replaces_one_match_at_a_time() {
    let mut console = console("select id from orders; select id from order_items;");
    console.selection = 7..9;
    console.open_find(false);
    assert_eq!(
        console.find_bar().query,
        "id",
        "the selection is searched for"
    );

    console.find_next(true);
    assert_eq!(console.selection, 30..32);
    assert_eq!(console.found().current, Some(1));
    console.find_next(true);
    assert_eq!(console.selection, 7..9, "wraps around");

    let bar = console.find_bar_mut();
    bar.replacement = "order_id".to_owned();
    bar.options.whole_word = true;
    console.replace_current(Instant::now());
    assert_eq!(
        console.text(),
        "select order_id from orders; select id from order_items;"
    );
    assert_eq!(console.selection, 36..38, "the next match is selected");
    assert_eq!(
        console.replaced_text.as_deref(),
        Some("select id from orders; select id from order_items;"),
        "one undo step"
    );
}

#[test]
fn replaces_every_match_with_regular_expression_groups() {
    let mut console = console("select a1, b2 from t");
    console.open_find(true);
    let bar = console.find_bar_mut();
    bar.query = r"([a-z])(\d)".to_owned();
    bar.replacement = "$2$1".to_owned();
    bar.options.regex = true;

    assert_eq!(console.replace_all(Instant::now()), 2);
    assert_eq!(console.text(), "select 1a, 2b from t");

    console.find_bar_mut().query = "(".to_owned();
    assert!(console.found().error.is_some());
    assert_eq!(console.replace_all(Instant::now()), 0);
    console.close_find();
    assert_eq!(console.found(), Found::default());
}
