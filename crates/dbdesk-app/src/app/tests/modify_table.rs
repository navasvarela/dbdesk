//! Modify table (E4), driven through the whole app against SQLite.

use egui_kittest::kittest::NodeT;

use super::*;

/// The `shop` table's columns, as SQLite lists them.
fn columns(dir: &Path, table: &str) -> Vec<(String, String)> {
    let connection = rusqlite::Connection::open(dir.join("shop.db")).unwrap();
    let mut statement = connection
        .prepare("SELECT name, type FROM pragma_table_info(?1) ORDER BY cid")
        .unwrap();
    statement
        .query_map([table], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Replaces the text of the one-line field labelled `label`.
fn replace(harness: &mut Harness<'_, DbDeskApp>, label: &str, text: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    harness.run_steps(2);
}

/// Opens Modify table on `orders` and waits for the table to load into the form.
fn modify_orders(harness: &mut Harness<'_, DbDeskApp>) {
    expand_shop_tables(harness, "orders");
    node_menu(harness, "orders", "Modify table…");
    step_until(harness, "the table loading into the form", |harness| {
        harness.query_by_label("Column 3 name").is_some()
    });
}

#[test]
fn renames_drops_and_adds_columns_keeping_the_rows() {
    let dir = tempfile::tempdir().unwrap();
    save_shop(
        dir.path(),
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, status TEXT, legacy INTEGER);
         INSERT INTO orders VALUES (1, 'paid', 7), (2, 'new', NULL);",
    );
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir.path(), feed);
    modify_orders(&mut harness);

    replace(&mut harness, "Column 2 name", "state");
    harness.get_by_label("Remove column 3").click();
    harness.run_steps(2);
    harness.get_by_label("Add column").click();
    harness.run_steps(2);
    fill(&mut harness, "Column 3 name", "note");
    fill(&mut harness, "Column 3 type", "text");

    let expected = "ALTER TABLE main.orders DROP COLUMN legacy;\n\n\
                    ALTER TABLE main.orders RENAME COLUMN status TO state;\n\n\
                    ALTER TABLE main.orders ADD COLUMN note text;\n";
    assert!(
        harness.query_all_by_value(expected).next().is_some(),
        "the review panel shows the statements"
    );
    harness
        .get_by_label("Drops the column legacy of the table main.orders, with every value in it.");
    harness.get_by_label("Execute").click();
    step_until(&mut harness, "the dialog closing", |harness| {
        harness.query_by_label("Execute").is_none()
    });

    let names: Vec<String> = columns(dir.path(), "orders")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["id", "state", "note"]);
    let connection = rusqlite::Connection::open(dir.path().join("shop.db")).unwrap();
    let states: Vec<String> = connection
        .prepare("SELECT state FROM orders ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(states, ["paid", "new"]);
}

#[test]
fn says_why_sqlite_cant_make_a_change_in_place() {
    let dir = tempfile::tempdir().unwrap();
    save_shop(
        dir.path(),
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, status TEXT, total INTEGER);",
    );
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir.path(), feed);
    modify_orders(&mut harness);

    replace(&mut harness, "Column 3 type", "real");
    harness.get_by_label_contains("SQLite can't change an existing column's type.");
    assert!(
        harness
            .get_by_label("Execute")
            .accesskit_node()
            .is_disabled()
    );
    assert_eq!(
        columns(dir.path(), "orders")[2],
        ("total".to_owned(), "INTEGER".to_owned())
    );
}
