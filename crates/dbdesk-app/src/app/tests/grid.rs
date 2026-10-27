//! Power in the grid (W5), driven through the whole app against SQLite.

use super::*;

/// The orders as the database has them, by id.
fn orders(dir: &Path) -> Vec<(i64, String)> {
    let connection = rusqlite::Connection::open(dir.join("shop.db")).unwrap();
    let mut statement = connection
        .prepare("SELECT id, customer FROM orders ORDER BY id")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The grid's filter field, by its hint, while the database sorts and filters.
fn grid_filter<'a>(harness: &'a Harness<'_, DbDeskApp>) -> egui_kittest::Node<'a> {
    harness.get_by(|node| {
        node.role() == Role::TextInput && node.placeholder() == Some("Filter every row, then Enter")
    })
}

/// Waits until a submit has been answered: the pending changes and "Saving…" are gone.
fn wait_for_submit(harness: &mut Harness<'_, DbDeskApp>) {
    step_until(harness, "the changes being saved", |harness| {
        harness.query_by_label_contains("changed cell").is_none()
            && harness.query_by_label_contains("new row").is_none()
            && harness.query_by_label("Saving…").is_none()
    });
}

#[test]
fn adds_edits_and_deletes_rows_in_one_submit() {
    let dir = tempfile::tempdir().unwrap();
    save_shop(
        dir.path(),
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT NOT NULL DEFAULT 'walk-in');
         INSERT INTO orders VALUES (1, 'ada'), (2, 'bob');",
    );
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir.path(), feed);
    open_orders(&mut harness, "2 rows");
    rename_ada(&mut harness);

    harness.get_by_label("bob").click_secondary();
    harness.run();
    harness.get_by_label("Delete row").click();
    harness.run();
    harness.get_by_label("Add row").click();
    harness.run();
    harness.get_by_label("1 changed cell · 1 new row · 1 deleted row");
    // The new row's id has no default but SQLite numbers it; its customer has one.
    harness.get_by_label("DEFAULT").click_secondary();
    harness.run();
    harness.get_by_label("Edit value").click();
    harness.run();
    harness
        .get_by(|node| node.role() == Role::TextInput && node.is_focused())
        .type_text("cy");
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(harness.query_by_label("DEFAULT").is_none());

    harness.get_by_label("Submit").click();
    wait_for_submit(&mut harness);
    // Adding a row runs the statement again, so the id SQLite gave it shows: the deleted
    // row's, since the delete ran first.
    step_until(&mut harness, "the statement running again", |harness| {
        harness.query_by_label("cy").is_some() && harness.query_by_label("bob").is_none()
    });
    assert_eq!(
        orders(dir.path()),
        [(1, "Ada Lovelace".to_owned()), (2, "cy".to_owned())]
    );
}

#[test]
fn a_failed_submit_names_the_new_row_and_keeps_every_change() {
    let dir = tempfile::tempdir().unwrap();
    save_shop(
        dir.path(),
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT NOT NULL);
         INSERT INTO orders VALUES (1, 'ada');",
    );
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir.path(), feed);
    open_orders(&mut harness, "1 rows");
    rename_ada(&mut harness);
    harness.get_by_label("Add row").click();
    harness.run();

    // The customer column has no default and refuses NULL.
    harness.get_by_label("Submit").click();
    step_until_visible(&mut harness, "New row 1: NOT NULL constraint failed");
    harness.get_by_label("1 changed cell · 1 new row");
    assert_eq!(orders(dir.path()), [(1, "ada".to_owned())]);

    harness.get_by_label("Revert").click();
    harness.run();
    assert!(harness.query_by_label("Add row").is_some());
    assert!(harness.query_by_label_contains("new row").is_none());
}

/// From an order to its customer through the key's link, then from the customer back to their
/// orders through Referencing rows: each opens in a new console.
#[test]
fn follows_a_foreign_key_to_its_row_and_back_to_the_rows_that_refer_to_it() {
    let dir = tempfile::tempdir().unwrap();
    save_shop(
        dir.path(),
        "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE orders (id INTEGER PRIMARY KEY,
             customer_id INTEGER REFERENCES customers (id));
         INSERT INTO customers VALUES (7, 'ada'), (8, 'bob');
         INSERT INTO orders VALUES (10, 8), (11, 7), (12, 8);",
    );
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir.path(), feed);
    open_orders(&mut harness, "3 rows");
    // Add row shows once the table is described, and with it its keys.
    step_until_visible(&mut harness, "Add row");

    harness.get_by_label("7").click_secondary();
    harness.run();
    harness.get_by_label("Open customers row").click();
    step_until_visible(&mut harness, "ada");
    harness.get_by_label_contains("1 rows");
    assert!(harness.query_by_label("bob").is_none());
    step_until_visible(&mut harness, "Add row");

    harness.get_by_label("ada").click_secondary();
    harness.run();
    harness.get_by_label_contains("Referencing rows").click();
    harness.run();
    harness.get_by_label("orders.customer_id").click();
    step_until_visible(&mut harness, "11");
    harness.get_by_label_contains("1 rows");
    assert!(harness.query_by_label("10").is_none());
}

/// With In the database ticked, sorting and filtering run the statement again wrapped, so
/// they cover rows that weren't loaded, and the rows stay editable.
#[test]
fn sorts_and_filters_every_row_in_the_database() {
    let dir = tempfile::tempdir().unwrap();
    save_shop(
        dir.path(),
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT);
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 600)
         INSERT INTO orders SELECT i, 'customer ' || i FROM n;",
    );
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir.path(), feed);
    open_orders(&mut harness, "500 rows, more available");
    step_until_visible(&mut harness, "Add row");

    harness.get_by_label("In the database").click();
    harness.run();
    harness.get_by_label("id").click();
    step_until_visible(&mut harness, "id ▲");
    step_until_visible(&mut harness, "Sorted and filtered by the database");
    harness.get_by_label("id ▲").click();
    step_until_visible(&mut harness, "customer 600");

    grid_filter(&harness).focus();
    harness.run();
    grid_filter(&harness).type_text("ER 59");
    harness.key_press(egui::Key::Enter);
    // customer 59 and customer 590 to 599, sorted by id descending.
    step_until_visible(&mut harness, "11 rows");
    harness.get_by_label("customer 599");
    assert!(harness.query_by_label("customer 600").is_none());
    step_until_visible(&mut harness, "Add row");

    harness.get_by_label("In the database").click();
    step_until_visible(&mut harness, "500 rows, more available");
    harness.get_by_label("customer 1");
}

/// Export › All rows runs the statement again and writes every row, not only the 500 loaded;
/// sorted in the database, the file is in the grid's order.
#[test]
fn exports_every_row_not_only_those_loaded() {
    let dir = tempfile::tempdir().unwrap();
    save_shop(
        dir.path(),
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT);
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 600)
         INSERT INTO orders SELECT i, 'customer ' || i FROM n;",
    );
    let export_path = dir.path().join("orders.csv");
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir.path(), feed);
    let picked = export_path.clone();
    harness.state_mut().pick_export_path = Box::new(move |_, _| Some(picked.clone()));
    open_orders(&mut harness, "500 rows, more available");
    step_until_visible(&mut harness, "Add row");
    harness.get_by_label("In the database").click();
    harness.run();
    harness.get_by_label("id").click();
    step_until_visible(&mut harness, "id ▲");
    harness.get_by_label("id ▲").click();
    step_until_visible(&mut harness, "customer 600");

    harness.get_by_label("Export").click();
    harness.run();
    harness.get_by_label("All rows as CSV…").click();
    step_until_visible(&mut harness, "Exported 600 rows");

    let text = std::fs::read_to_string(&export_path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 601);
    assert_eq!(
        (lines[0], lines[1], lines[600]),
        ("id,customer", "600,customer 600", "1,customer 1")
    );
}

/// Saves `shop` with an orders table at `safety`, starts the app, and opens Import CSV… on
/// the table with `csv` as the file.
fn import_csv_into_orders(
    dir: &Path,
    safety: dbdesk_core::SafetyLevel,
    csv: &str,
) -> Harness<'static, DbDeskApp> {
    let shop = save_shop(
        dir,
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT NOT NULL, qty INTEGER,
             note TEXT DEFAULT 'none');
         INSERT INTO orders VALUES (1, 'ada', 1, 'first');",
    );
    DataSourceFile::new(dir.join("data_sources.toml"))
        .save(&[dbdesk_core::DataSource { safety, ..shop }])
        .unwrap();
    let file = dir.join("orders.csv");
    std::fs::write(&file, csv).unwrap();
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir, feed);
    harness.state_mut().pick_csv_file = Box::new(move || Some(file.clone()));
    expand_shop_tables(&mut harness, "orders");
    harness.get_by_label("orders").click_secondary();
    harness.run();
    harness.get_by_label("Import CSV…").click();
    step_until_visible(&mut harness, "Source of customer");
    // The dialog settles where it is centred before a pointer can find its buttons.
    harness.run();
    harness
}

/// The file's columns are matched to the table's by header name, a value that doesn't fit
/// its column is flagged, and Import inserts the rows, the unmatched column at its default.
#[test]
fn imports_a_csv_file_matching_its_columns_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = import_csv_into_orders(
        dir.path(),
        dbdesk_core::SafetyLevel::Normal,
        "Customer,ID,Qty\r\nbob,2,3\r\n\"cy, jr\",3,lots\r\n",
    );
    harness.get_by_label("Import into main.orders");
    harness.get_by(|node| {
        node.role() == Role::ComboBox
            && node.label().as_deref() == Some("Source of id")
            && node.value().as_deref() == Some("ID")
    });
    harness.get_by_label_contains("Line 3, qty: \u{201c}lots\u{201d} isn't a whole number");

    harness.get_by_label("Import").click();
    step_until_visible(&mut harness, "Imported 2 rows into main.orders");
    let connection = rusqlite::Connection::open(dir.path().join("shop.db")).unwrap();
    let imported: Vec<(i64, String, String)> = connection
        .prepare("SELECT id, customer, note FROM orders WHERE id > 1 ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        imported,
        [
            (2, "bob".to_owned(), "none".to_owned()),
            (3, "cy, jr".to_owned(), "none".to_owned())
        ]
    );
    // The dialog grew by the status line, and is centred again before Close can be clicked.
    harness.run();
    harness.get_by_label("Close").click();
    harness.run();
    assert!(harness.query_by_label("Import into main.orders").is_none());
}

/// A careful data source asks first; a row the database refuses rolls everything back, and
/// the dialog names its line.
#[test]
fn a_failed_import_names_its_line_and_imports_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = import_csv_into_orders(
        dir.path(),
        dbdesk_core::SafetyLevel::Careful,
        "id,customer\n2,bob\n1,again\n",
    );
    harness.get_by_label("Import").click();
    harness.run();
    harness.get_by_label_contains("\u{201c}shop\u{201d} is careful: this inserts the file's rows");
    harness.get_by_label("Import").click();

    step_until_visible(&mut harness, "Line 3: UNIQUE constraint failed: orders.id");
    harness.get_by_label_contains("Nothing was imported.");
    let count: i64 = rusqlite::Connection::open(dir.path().join("shop.db"))
        .unwrap()
        .query_row("SELECT count(*) FROM orders", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
