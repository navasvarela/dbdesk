//! Golden tests of the SQL formats in every dialect, and a round trip through SQLite.

use dbrover_core::{Cell, ColumnInfo};
use dbrover_sql::Dialect;

use super::*;

fn text(value: &str) -> Cell {
    Some(value.to_owned())
}

/// `id` (the key), `note` and `price` are the table's; `total` is computed.
fn columns(dialect: Dialect) -> Vec<ExportColumn> {
    [
        ("id", Some("integer")),
        ("note", Some("text")),
        ("total", None),
        ("price", Some("decimal")),
    ]
    .map(|(name, type_name)| {
        let column = ColumnInfo {
            name: name.to_owned(),
            type_name: type_name.map(str::to_owned),
        };
        ExportColumn::new(&column, dialect)
    })
    .to_vec()
}

fn rows() -> Vec<Vec<Cell>> {
    vec![
        vec![text("1"), text(r"it's C:\x"), text("19"), text("9.50")],
        vec![text("2"), None, None, text("NaN")],
    ]
}

fn source(dialect: Dialect) -> RowSource {
    RowSource {
        dialect,
        table: Some(SourceTable {
            name: SqlTable {
                schema: Some("shop".to_owned()),
                name: "order items".to_owned(),
            },
            columns: vec![0, 1, 3],
            key: Some(vec![0]),
        }),
    }
}

fn insert(dialect: Dialect, rows_per_statement: u16) -> String {
    let options = InsertOptions {
        rows_per_statement: RowsPerInsert::clamped(rows_per_statement),
    };
    let spec = InsertSpec::for_rows(&source(dialect), options).unwrap();
    render(
        &ExportSpec::SqlInsert(Box::new(spec)),
        &columns(dialect),
        &rows(),
    )
    .unwrap()
}

fn update(dialect: Dialect) -> String {
    let spec = UpdateSpec::for_rows(&source(dialect)).unwrap();
    render(
        &ExportSpec::SqlUpdate(Box::new(spec)),
        &columns(dialect),
        &rows(),
    )
    .unwrap()
}

#[test]
fn writes_an_insert_a_row_in_every_dialect() {
    let expected = [
        (
            Dialect::Postgres,
            r#"INSERT INTO "shop"."order items" ("id", "note", "price") VALUES (1, 'it''s C:\x', 9.50);
INSERT INTO "shop"."order items" ("id", "note", "price") VALUES (2, NULL, 'NaN');
"#,
        ),
        (
            Dialect::Sqlite,
            r#"INSERT INTO "shop"."order items" ("id", "note", "price") VALUES (1, 'it''s C:\x', 9.50);
INSERT INTO "shop"."order items" ("id", "note", "price") VALUES (2, NULL, 'NaN');
"#,
        ),
        (
            Dialect::Mysql,
            r"INSERT INTO `shop`.`order items` (`id`, `note`, `price`) VALUES (1, 'it''s C:\\x', 9.50);
INSERT INTO `shop`.`order items` (`id`, `note`, `price`) VALUES (2, NULL, 'NaN');
",
        ),
        (
            Dialect::SqlServer,
            r"INSERT INTO [shop].[order items] ([id], [note], [price]) VALUES (1, N'it''s C:\x', 9.50);
INSERT INTO [shop].[order items] ([id], [note], [price]) VALUES (2, NULL, N'NaN');
",
        ),
        (
            Dialect::DuckDb,
            r#"INSERT INTO "shop"."order items" ("id", "note", "price") VALUES (1, 'it''s C:\x', 9.50);
INSERT INTO "shop"."order items" ("id", "note", "price") VALUES (2, NULL, 'NaN');
"#,
        ),
        (
            Dialect::ClickHouse,
            r"INSERT INTO `shop`.`order items` (`id`, `note`, `price`) VALUES (1, 'it''s C:\\x', 9.50);
INSERT INTO `shop`.`order items` (`id`, `note`, `price`) VALUES (2, NULL, 'NaN');
",
        ),
    ];
    for (dialect, sql) in expected {
        assert_eq!(insert(dialect, 1), sql, "{dialect:?}");
    }
}

#[test]
fn batches_rows_into_one_insert() {
    assert_eq!(
        insert(Dialect::Postgres, 2),
        r#"INSERT INTO "shop"."order items" ("id", "note", "price") VALUES
(1, 'it''s C:\x', 9.50),
(2, NULL, 'NaN');
"#
    );
    assert_eq!(insert(Dialect::Postgres, 5), insert(Dialect::Postgres, 2));
    assert_eq!(RowsPerInsert::clamped(0).get(), 1);
    assert_eq!(RowsPerInsert::clamped(5_000).get(), MAX_ROWS_PER_INSERT);
}

#[test]
fn writes_an_update_a_row_by_its_key_in_every_dialect() {
    let expected = [
        (
            Dialect::Postgres,
            r#"UPDATE "shop"."order items" SET "note" = 'it''s C:\x', "price" = '9.50' WHERE "id" = '1';
UPDATE "shop"."order items" SET "note" = NULL, "price" = 'NaN' WHERE "id" = '2';
"#,
        ),
        (
            Dialect::Mysql,
            r"UPDATE `shop`.`order items` SET `note` = 'it''s C:\\x', `price` = '9.50' WHERE `id` = '1';
UPDATE `shop`.`order items` SET `note` = NULL, `price` = 'NaN' WHERE `id` = '2';
",
        ),
        (
            Dialect::SqlServer,
            r"UPDATE [shop].[order items] SET [note] = N'it''s C:\x', [price] = N'9.50' WHERE [id] = N'1';
UPDATE [shop].[order items] SET [note] = NULL, [price] = N'NaN' WHERE [id] = N'2';
",
        ),
    ];
    for (dialect, sql) in expected {
        assert_eq!(update(dialect), sql, "{dialect:?}");
    }
    assert_eq!(update(Dialect::Sqlite), update(Dialect::Postgres));
    assert_eq!(update(Dialect::DuckDb), update(Dialect::Postgres));
}

#[test]
fn says_why_rows_cant_be_written_as_sql() {
    let untabled = RowSource::untabled(Dialect::Postgres);
    for format in [ExportFormat::SqlInsert, ExportFormat::SqlUpdate] {
        let refusal = format.default_spec(&untabled).unwrap_err();
        assert!(refusal.contains("one table"), "{format:?}: {refusal}");
    }

    let mut keyless = source(Dialect::Postgres);
    if let Some(table) = &mut keyless.table {
        table.key = None;
    }
    assert!(ExportFormat::SqlInsert.default_spec(&keyless).is_ok());
    let refusal = ExportFormat::SqlUpdate.default_spec(&keyless).unwrap_err();
    assert!(refusal.contains("primary key"), "{refusal}");

    let mut only_key = source(Dialect::Postgres);
    if let Some(table) = &mut only_key.table {
        table.columns = vec![0];
    }
    let refusal = ExportFormat::SqlUpdate.default_spec(&only_key).unwrap_err();
    assert!(refusal.contains("nothing to update"), "{refusal}");
}

/// Rows read from a SQLite table, exported as `INSERT`s and run into an empty copy of it,
/// come back the same, types included.
#[test]
fn inserts_run_into_an_empty_copy_give_the_same_rows() {
    let database = rusqlite::Connection::open_in_memory().unwrap();
    database
        .execute_batch(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, note TEXT, price NUMERIC, code TEXT);
             CREATE TABLE copy (id INTEGER PRIMARY KEY, note TEXT, price NUMERIC, code TEXT);
             INSERT INTO items VALUES
               (1, 'it''s ''quoted'', with \\ and ;', 9.5, '007'),
               (2, NULL, NULL, ''),
               (3, '🦀 line
break', -1e3, 'x'),
               (4, 'NaN', 'NaN', '1,5');",
        )
        .unwrap();
    let (columns, rows) = read(&database, "SELECT * FROM items ORDER BY id");

    for rows_per_statement in [1, 3] {
        let source = RowSource {
            dialect: Dialect::Sqlite,
            table: Some(SourceTable {
                name: SqlTable {
                    schema: Some("main".to_owned()),
                    name: "copy".to_owned(),
                },
                columns: (0..columns.len()).collect(),
                key: Some(vec![0]),
            }),
        };
        let options = InsertOptions {
            rows_per_statement: RowsPerInsert::clamped(rows_per_statement),
        };
        let spec = ExportSpec::SqlInsert(Box::new(InsertSpec::for_rows(&source, options).unwrap()));
        let sql = render(&spec, &columns, &rows).unwrap();
        database.execute_batch(&sql).unwrap();

        let typed = "SELECT id, typeof(note), note, typeof(price), price, typeof(code), code";
        assert_eq!(
            read(&database, &format!("{typed} FROM copy ORDER BY id")).1,
            read(&database, &format!("{typed} FROM items ORDER BY id")).1,
            "{sql}"
        );
        database.execute_batch("DELETE FROM copy").unwrap();
    }
}

/// The columns and rows of `sql`, as the SQLite driver gives them: text, `NULL` as `None`.
fn read(database: &rusqlite::Connection, sql: &str) -> (Vec<ExportColumn>, Vec<Vec<Cell>>) {
    let mut statement = database.prepare(sql).unwrap();
    let columns: Vec<ExportColumn> = statement
        .columns()
        .iter()
        .map(|column| {
            let info = ColumnInfo {
                name: column.name().to_owned(),
                type_name: column.decl_type().map(str::to_lowercase),
            };
            ExportColumn::new(&info, Dialect::Sqlite)
        })
        .collect();
    let count = columns.len();
    let rows = statement
        .query_map([], |row| {
            (0..count)
                .map(|index| {
                    Ok(match row.get_ref(index)? {
                        rusqlite::types::ValueRef::Null => None,
                        rusqlite::types::ValueRef::Integer(n) => Some(n.to_string()),
                        rusqlite::types::ValueRef::Real(x) => Some(x.to_string()),
                        rusqlite::types::ValueRef::Text(t) | rusqlite::types::ValueRef::Blob(t) => {
                            Some(String::from_utf8_lossy(t).into_owned())
                        }
                    })
                })
                .collect()
        })
        .unwrap()
        .collect::<Result<Vec<Vec<Cell>>, _>>()
        .unwrap();
    (columns, rows)
}

/// Copying some of the columns keeps the table's among them, and the key only when all of it
/// is selected.
#[test]
fn narrows_the_source_to_the_selected_columns() {
    let source = source(Dialect::Postgres);
    let narrowed = source.select_columns(&[3, 0]);
    let table = narrowed.table.as_ref().unwrap();
    assert_eq!(table.columns, [1, 0]);
    assert_eq!(table.key.as_deref(), Some(&[1][..]));
    assert_eq!(
        insert_of(&narrowed, &[3, 0]),
        "INSERT INTO \"shop\".\"order items\" (\"id\", \"price\") VALUES (1, 9.50);\n\
         INSERT INTO \"shop\".\"order items\" (\"id\", \"price\") VALUES (2, 'NaN');\n"
    );

    let without_key = source.select_columns(&[1, 3]);
    assert_eq!(without_key.table.as_ref().unwrap().key, None);
    assert!(UpdateSpec::for_rows(&without_key).is_err());
}

/// `rows()` as `INSERT`s of only the columns at `selected`.
fn insert_of(source: &RowSource, selected: &[usize]) -> String {
    let all = columns(source.dialect);
    let columns: Vec<ExportColumn> = selected.iter().map(|&index| all[index].clone()).collect();
    let rows: Vec<Vec<Cell>> = rows()
        .into_iter()
        .map(|row| selected.iter().map(|&index| row[index].clone()).collect())
        .collect();
    let spec = InsertSpec::for_rows(source, InsertOptions::default()).unwrap();
    render(&ExportSpec::SqlInsert(Box::new(spec)), &columns, &rows).unwrap()
}
