//! `.sql` files (W6a), driven through the whole app against SQLite, in temporary folders.

use std::fs;

use crate::sql_files::Remembered;

use super::*;

const UNSAVED: &str = " \u{2022}";

/// Starts the app with one `shop` data source, File › Open… answering `file`.
fn start_with_file(dir: &Path, file: &Path) -> Harness<'static, DbDeskApp> {
    save_shop(dir, "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir, feed);
    let picked = file.to_owned();
    harness.state_mut().pick_sql_file = Box::new(move || Some(picked.clone()));
    step_until_visible(&mut harness, "Expand shop");
    harness
}

fn open_from_the_menu(harness: &mut Harness<'_, DbDeskApp>) {
    harness.get_by_label("File").click();
    harness.run();
    // A menu item's label ends with its shortcut.
    harness.get_by_label_contains("Open…").click();
    step_until(harness, "the file opening", |harness| {
        harness.state().consoles.files().count() == 1
    });
    harness.run();
}

/// The titles of the console tabs, which the dock doesn't expose to accessibility.
fn tab_titles(harness: &Harness<'_, DbDeskApp>) -> Vec<String> {
    harness
        .state()
        .open_consoles()
        .map(crate::console::QueryConsole::tab_title)
        .collect()
}

fn type_in_editor(harness: &mut Harness<'_, DbDeskApp>, text: &str) {
    harness.get_by_role(Role::MultilineTextInput).focus();
    harness.run();
    harness
        .get_by_role(Role::MultilineTextInput)
        .type_text(text);
    harness.run();
}

fn file_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

#[test]
fn opens_a_sql_file_and_saves_it_only_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("report.sql");
    fs::write(&file, "select 41;").unwrap();
    let mut harness = start_with_file(dir.path(), &file);

    open_from_the_menu(&mut harness);
    assert_eq!(editor_text(&harness), "select 41;");
    assert_eq!(tab_titles(&harness), ["report.sql"]);
    type_in_editor(&mut harness, " -- mine");
    assert_eq!(tab_titles(&harness), [format!("report.sql{UNSAVED}")]);
    assert_eq!(file_text(&file), "select 41;", "not saved by itself");

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    step_until(&mut harness, "the file being saved", |_| {
        file_text(&file).contains("-- mine")
    });
    step_until(&mut harness, "the unsaved marker going", |harness| {
        tab_titles(harness) == ["report.sql"]
    });
    assert!(
        !dir.path().join("consoles").exists(),
        "a file's console has no copy in the config folder"
    );
}

#[test]
fn save_as_moves_a_console_from_the_config_folder_to_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let shop = save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir.path(), feed);
    let file = dir.path().join("seven.sql");
    let picked = file.clone();
    harness.state_mut().pick_sql_save_path = Box::new(move |_| Some(picked.clone()));
    let config_copy = ConsoleFiles::new(dir.path().join("consoles")).path(&ConsoleKey {
        data_source: shop.id,
        name: ConsoleName::numbered(1),
    });
    open_console(&mut harness);
    harness
        .get_by_role(Role::MultilineTextInput)
        .type_text("select 7;");
    step_until(&mut harness, "the console saving itself", |_| {
        config_copy.exists()
    });

    harness.get_by_label("File").click();
    harness.run();
    harness.get_by_label_contains("Save as…").click();

    step_until(&mut harness, "the file being written", |_| {
        file.exists() && file_text(&file) == "select 7;"
    });
    step_until(&mut harness, "the config folder's copy going", |_| {
        !config_copy.exists()
    });
    assert_eq!(tab_titles(&harness), ["seven.sql"]);
}

#[test]
fn asks_before_closing_a_file_with_unsaved_changes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("report.sql");
    fs::write(&file, "select 1;").unwrap();
    let mut harness = start_with_file(dir.path(), &file);
    open_from_the_menu(&mut harness);
    type_in_editor(&mut harness, " -- mine");
    let key = workspace::console_tabs(&harness.state().dock)[0].clone();
    let close = |harness: &mut Harness<'_, DbDeskApp>| {
        let ctx = harness.ctx.clone();
        harness
            .state_mut()
            .request_close(Closing::Console(key.clone()), &ctx);
        step_until_visible(harness, "Save the changes to report.sql?");
        // Two frames for the modal to be placed; see the rollback test.
        harness.run_steps(2);
    };

    close(&mut harness);
    // Escape answers Cancel; the toolbar has a Cancel button of its own.
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(workspace::console_tabs(&harness.state().dock).len(), 1);

    close(&mut harness);
    harness.get_by_label("Save").click();
    step_until(&mut harness, "the tab closing", |harness| {
        workspace::console_tabs(&harness.state().dock).is_empty()
    });
    assert!(file_text(&file).contains("-- mine"));
}

#[test]
fn follows_changes_made_on_disk_and_asks_when_it_has_unsaved_ones() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("report.sql");
    fs::write(&file, "select 1;").unwrap();
    let mut harness = start_with_file(dir.path(), &file);
    open_from_the_menu(&mut harness);

    fs::write(&file, "select 2;").unwrap();
    step_until(&mut harness, "the console following the file", |harness| {
        editor_text(harness) == "select 2;"
    });

    type_in_editor(&mut harness, " -- mine");
    fs::write(&file, "select 3;").unwrap();
    step_until_visible(&mut harness, "changed on disk");
    harness.get_by_label("Reload").click();
    harness.run();
    assert_eq!(editor_text(&harness), "select 3;");
}

#[test]
fn restores_file_tabs_and_opens_files_named_on_the_command_line() {
    let dir = tempfile::tempdir().unwrap();
    let shop = save_shop(dir.path(), "CREATE TABLE orders (id INTEGER PRIMARY KEY)");
    let restored = dir.path().join("restored.sql");
    let named = dir.path().join("named.sql");
    fs::write(&restored, "select 'restored';").unwrap();
    fs::write(&named, "select 'named';").unwrap();
    let key = ConsoleKey {
        data_source: shop.id,
        name: ConsoleName::try_from("file_1".to_owned()).unwrap(),
    };
    let (_layer, feed) = log_console::pipeline();
    let config = engine_config(dir.path());
    let (tab, path) = (key.clone(), restored.clone());
    // As if the last session's layout held the tab, and the command line named a file.
    let mut harness = harness_at_default_size().build_eframe(|cc| {
        let mut app = DbDeskApp::new(cc, feed, config)
            .unwrap()
            .with_files_to_open(vec![named.clone()]);
        app.sql_files = SqlFiles::new(Remembered {
            recent: Vec::new(),
            open: vec![(tab.clone(), path)],
        });
        workspace::open_console_tab(&mut app.dock, tab);
        app
    });

    step_until(&mut harness, "both files opening", |harness| {
        harness.state().consoles.files().count() == 2
    });
    let app = harness.state();
    let remembered = app.sql_files.remembered(app.consoles.files());
    assert!(remembered.open.contains(&(key, restored)));
    assert_eq!(remembered.recent[0].path, named);
    assert_eq!(remembered.recent[0].data_source, shop.id);
    assert_eq!(workspace::console_tabs(&app.dock).len(), 2);
}
