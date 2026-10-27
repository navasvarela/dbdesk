//! Find and replace (W6e), driven through the whole app against SQLite.

use super::*;

fn field<'a>(harness: &'a Harness<'_, DbDeskApp>, label: &'a str) -> egui_kittest::Node<'a> {
    harness.get_by_role_and_label(Role::TextInput, label)
}

/// A few frames for a key or click to take effect. Not `run()`: the console saving itself and
/// checking its text keep the UI repainting, which on a slow runner outlasts `run()`'s steps.
fn settle(harness: &mut Harness<'_, DbDeskApp>) {
    harness.run_steps(3);
}

#[test]
fn finds_with_the_shortcuts_and_replaces_every_match_in_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir.path(), feed);
    open_console(&mut harness);
    let text = "SELECT * FROM orders;\nselect count(*) from ORDERS;";
    harness
        .get_by_role(Role::MultilineTextInput)
        .type_text(text);
    settle(&mut harness);

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    settle(&mut harness);
    field(&harness, "Find").type_text("orders");
    settle(&mut harness);
    harness.get_by_label("2 matches");
    harness.key_press(egui::Key::Enter);
    settle(&mut harness);
    harness.get_by_label("1 of 2");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::G);
    settle(&mut harness);
    harness.get_by_label("2 of 2");
    harness
        .get_by_role_and_label(Role::CheckBox, "Match case")
        .click();
    settle(&mut harness);
    harness.get_by_label("1 match");

    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::ALT,
        egui::Key::F,
    );
    settle(&mut harness);
    harness
        .get_by_role_and_label(Role::CheckBox, "Match case")
        .click();
    field(&harness, "Replace").focus();
    settle(&mut harness);
    field(&harness, "Replace").type_text("customers");
    settle(&mut harness);
    harness.get_by_label("Replace all").click();
    settle(&mut harness);
    harness.get_by_label("Replaced 2");
    assert_eq!(
        editor_text(&harness),
        "SELECT * FROM customers;\nselect count(*) from customers;"
    );

    harness.get_by_role(Role::MultilineTextInput).focus();
    settle(&mut harness);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    // The console saving itself keeps the UI busy, so this waits rather than settles.
    step_until(
        &mut harness,
        "one undo taking every replacement back",
        |harness| editor_text(harness) == text,
    );
}
