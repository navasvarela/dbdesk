//! `actions.toml`, `snippets.toml` and `theme.toml` (H1), driven through the whole app against
//! SQLite with a temporary config folder.

use super::*;

#[test]
fn counts_each_files_problems_and_lists_them_until_a_reload_finds_none() {
    let dir = tempfile::tempdir().unwrap();
    save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
    let actions = dir.path().join("actions.toml");
    std::fs::write(
        &actions,
        "[[action]]\nid = \"row-count\"\nname = \"Row count\"\napplies_to = [\"tabel\"]\n\
         sql = \"SELECT count(*) FROM {qualified_name}\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("theme.toml"),
        "[dark]\nprimary = \"#7c5cff\"\nbackground = \"#000000\"\nbase = \"dark\"\n",
    )
    .unwrap();
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir.path(), feed);
    step_until_visible(&mut harness, "actions.toml: 1 problem");
    harness.get_by_label("theme.toml: 2 problems");
    assert!(harness.query_by_label_contains("snippets.toml").is_none());

    harness.get_by_label("actions.toml: 1 problem").click();
    step_until_visible(&mut harness, "0 actions");
    harness.get_by_label_contains("\"tabel\" is not something actions apply to");

    std::fs::write(
        &actions,
        std::fs::read_to_string(&actions)
            .unwrap()
            .replace("tabel", "table"),
    )
    .unwrap();
    harness.get_by_label("Reload").click();
    step_until_visible(&mut harness, "1 action");
    harness.get_by_label("No problems.");
    assert!(harness.query_by_label("actions.toml: 1 problem").is_none());
    harness.get_by_label("theme.toml: 2 problems");
}
