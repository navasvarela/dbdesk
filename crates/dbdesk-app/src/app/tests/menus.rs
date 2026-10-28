//! The menu bar egui draws (D5a), built from the command list: the same items, shortcuts and
//! states as before it was, and each item doing what it says.

use egui::ThemePreference;
use egui_kittest::kittest::NodeT;

use crate::commands::{self, AppCommand, CommandState, Entry, Menu};

use super::*;

fn start_without_data_sources(dir: &Path) -> Harness<'static, DbDeskApp> {
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir, feed);
    step_until_visible(&mut harness, "Engine running");
    harness
}

fn click(harness: &mut Harness<'_, DbDeskApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run_steps(2);
}

/// A submenu's button has an arrow after its title.
fn open_submenu(harness: &mut Harness<'_, DbDeskApp>, title: &str) {
    harness
        .get_by(|node| node.label().is_some_and(|text| text.starts_with(title)))
        .click();
    harness.run_steps(2);
}

/// A menu item's accessible label is its own, then its shortcut if it has one.
fn item<'h>(harness: &'h Harness<'_, DbDeskApp>, command: &AppCommand) -> egui_kittest::Node<'h> {
    let label = command.label().into_owned();
    let shortcut = command
        .shortcut()
        .and_then(|action| Keymap::default().hint(&harness.ctx, action))
        .unwrap_or_default();
    harness.get_by(move |node| {
        node.label().is_some_and(|text| {
            text.strip_prefix(label.as_str())
                .is_some_and(|rest| rest.trim() == shortcut)
        })
    })
}

fn check_entries(harness: &mut Harness<'_, DbDeskApp>, entries: &[Entry], state: &CommandState) {
    for entry in entries {
        match entry {
            Entry::Command(command) => check_command(harness, command, state),
            Entry::Submenu(Menu { title, entries }) => {
                open_submenu(harness, title);
                check_entries(harness, entries, state);
            }
            Entry::Note(text) => {
                harness.get_by_label(text);
            }
            Entry::Separator => {}
        }
    }
}

fn check_command(harness: &Harness<'_, DbDeskApp>, command: &AppCommand, state: &CommandState) {
    let node = item(harness, command);
    let node = node.accesskit_node();
    let label = node.label().unwrap_or_default();
    assert_eq!(
        node.is_disabled(),
        !command.enabled(state),
        "{label} is enabled when it shouldn't be, or the other way round"
    );
    if command.checked(state).is_some() {
        assert_eq!(node.role(), Role::RadioButton, "{label}");
    }
}

#[test]
fn lists_every_command_with_its_shortcut_and_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = start_without_data_sources(dir.path());
    let state = CommandState {
        console_focused: false,
        has_data_sources: false,
        theme: ThemePreference::System,
    };

    for menu in commands::egui_menu_bar([]) {
        click(&mut harness, menu.title);
        check_entries(&mut harness, &menu.entries, &state);
        // Clicking the title again closes the menu and any submenu it opened.
        click(&mut harness, menu.title);
    }
}

#[test]
fn disabled_items_say_what_they_wait_for() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = start_without_data_sources(dir.path());
    harness
        .ctx
        .all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);

    click(&mut harness, "File");
    item(&harness, &AppCommand::Save).hover();
    harness.run_steps(2);
    harness.get_by_label("Select a query console first");
    item(&harness, &AppCommand::ExportDataSources).hover();
    harness.run_steps(2);
    harness.get_by_label("There are no data sources to export yet");
}

#[test]
fn items_run_their_commands() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = start_without_data_sources(dir.path());

    click(&mut harness, "View");
    open_submenu(&mut harness, "Theme");
    harness
        .get_by_role_and_label(Role::RadioButton, "Dark")
        .click();
    harness.run_steps(2);
    assert_eq!(
        harness.ctx.options(|options| options.theme_preference),
        ThemePreference::Dark
    );
    assert!(
        harness.query_by_label("Match system").is_none(),
        "picking a theme closes the menu"
    );

    click(&mut harness, "View");
    click(&mut harness, "Keymap…");
    harness.get_by_label("Create keymap.toml");

    click(&mut harness, "Help");
    click(&mut harness, "About dbdesk");
    harness.get_by_label("Licensed under MIT OR Apache-2.0");
}
