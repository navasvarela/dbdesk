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
