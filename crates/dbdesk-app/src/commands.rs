//! What the menus can do. The menu bar is data ([`egui_menu_bar`]) built from [`AppCommand`]s,
//! and every item runs through `DbDeskApp::run_command`, so a menu that draws it differently
//! (the native one on macOS) can't make an item do something else. See `docs/macos-look.md`.

use std::borrow::Cow;
use std::path::PathBuf;

use egui::ThemePreference;

use crate::{icons, keymap};

/// Something a menu item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    OpenFile,
    OpenRecent(PathBuf),
    ClearRecentFiles,
    Save,
    SaveAs,
    ExportDataSources,
    ImportDataSources,
    Theme(ThemePreference),
    ShowKeymap,
    ShowSettings,
    ResetLayout,
    Format,
    FormatAll,
    CheckForUpdates,
    About,
}

/// What decides whether a command can run and which item is checked, gathered once a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandState {
    /// The dock's focused tab is a query console.
    pub console_focused: bool,
    pub has_data_sources: bool,
    pub theme: ThemePreference,
}

impl AppCommand {
    /// A stable name for the command, for native menu items and tests. Commands that are
    /// keymap actions share the action's id.
    pub fn id(&self) -> &'static str {
        match self {
            Self::OpenFile => keymap::Action::OpenFile.id(),
            Self::Save => keymap::Action::Save.id(),
            Self::SaveAs => keymap::Action::SaveAs.id(),
            Self::Format => keymap::Action::Format.id(),
            Self::FormatAll => keymap::Action::FormatAll.id(),
            Self::OpenRecent(_) => "file.open-recent",
            Self::ClearRecentFiles => "file.clear-recent",
            Self::ExportDataSources => "file.export-data-sources",
            Self::ImportDataSources => "file.import-data-sources",
            Self::Theme(ThemePreference::System) => "view.theme.system",
            Self::Theme(ThemePreference::Light) => "view.theme.light",
            Self::Theme(ThemePreference::Dark) => "view.theme.dark",
            Self::ShowKeymap => "view.keymap",
            Self::ShowSettings => "view.settings",
            Self::ResetLayout => "view.reset-layout",
            Self::CheckForUpdates => "help.check-for-updates",
            Self::About => "help.about",
        }
    }

    pub fn label(&self) -> Cow<'_, str> {
        let label = match self {
            Self::OpenRecent(path) => {
                return path.file_name().map_or_else(
                    || Cow::Owned(path.display().to_string()),
                    |name| name.to_string_lossy(),
                );
            }
            Self::OpenFile => "Open…",
            Self::ClearRecentFiles => "Clear recent files",
            Self::Save => "Save",
            Self::SaveAs => "Save as…",
            Self::ExportDataSources => "Export data sources…",
            Self::ImportDataSources => "Import data sources…",
            Self::Theme(ThemePreference::System) => "Match system",
            Self::Theme(ThemePreference::Light) => "Light",
            Self::Theme(ThemePreference::Dark) => "Dark",
            Self::ShowKeymap => "Keymap…",
            Self::ShowSettings => "Settings…",
            Self::ResetLayout => "Reset layout",
            Self::Format => "Format",
            Self::FormatAll => "Format all",
            Self::CheckForUpdates => "Check for updates…",
            Self::About => "About dbdesk",
        };
        Cow::Borrowed(label)
    }

    /// The keymap action whose keys run this command too, and which the menu shows.
    pub fn shortcut(&self) -> Option<keymap::Action> {
        match self {
            Self::OpenFile => Some(keymap::Action::OpenFile),
            Self::Save => Some(keymap::Action::Save),
            Self::SaveAs => Some(keymap::Action::SaveAs),
            Self::Format => Some(keymap::Action::Format),
            Self::FormatAll => Some(keymap::Action::FormatAll),
            _ => None,
        }
    }

    /// The command a keymap action runs when it is one of the menus' commands.
    pub fn for_action(action: keymap::Action) -> Option<Self> {
        match action {
            keymap::Action::OpenFile => Some(Self::OpenFile),
            keymap::Action::Save => Some(Self::Save),
            keymap::Action::SaveAs => Some(Self::SaveAs),
            keymap::Action::Format => Some(Self::Format),
            keymap::Action::FormatAll => Some(Self::FormatAll),
            _ => None,
        }
    }

    pub fn tooltip(&self) -> Option<Cow<'_, str>> {
        if let Some(action) = self.shortcut() {
            return Some(Cow::Borrowed(action.description()));
        }
        match self {
            Self::OpenRecent(path) => Some(Cow::Owned(path.display().to_string())),
            Self::ExportDataSources => Some(Cow::Borrowed(
                "Write data sources and projects to a file, without passwords",
            )),
            Self::ImportDataSources => Some(Cow::Borrowed(
                "Add or update data sources from an exported file",
            )),
            _ => None,
        }
    }

    /// What the command is waiting for while it is disabled.
    pub fn disabled_reason(&self) -> Option<&'static str> {
        match self {
            Self::ExportDataSources => Some("There are no data sources to export yet"),
            _ if self.shortcut().is_some() => Some("Select a query console first"),
            _ => None,
        }
    }

    pub fn enabled(&self, state: &CommandState) -> bool {
        match self {
            Self::Save | Self::SaveAs | Self::Format | Self::FormatAll => state.console_focused,
            Self::ExportDataSources => state.has_data_sources,
            _ => true,
        }
    }

    /// Whether the item shows as picked, for commands that are one of a set of choices.
    pub fn checked(&self, state: &CommandState) -> Option<bool> {
        match self {
            Self::Theme(preference) => Some(*preference == state.theme),
            _ => None,
        }
    }

    pub fn icon(&self) -> Option<&'static str> {
        match self {
            Self::Theme(ThemePreference::System) => Some(icons::MONITOR),
            Self::Theme(ThemePreference::Light) => Some(icons::SUN),
            Self::Theme(ThemePreference::Dark) => Some(icons::MOON),
            _ => None,
        }
    }
}

/// A menu or submenu: its title and what it lists, top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub title: &'static str,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Command(AppCommand),
    Separator,
    Submenu(Menu),
    /// Greyed-out text standing in for an empty list.
    Note(&'static str),
}

/// The menu bar drawn by egui: on Windows and Linux, and wherever there is no native one.
/// `recent_files` are the files for File › Open recent, most recent first.
pub fn egui_menu_bar(recent_files: impl IntoIterator<Item = PathBuf>) -> Vec<Menu> {
    use AppCommand as C;
    use Entry::{Command, Separator, Submenu};
    vec![
        Menu {
            title: "File",
            entries: vec![
                Command(C::OpenFile),
                Submenu(open_recent(recent_files)),
                Command(C::Save),
                Command(C::SaveAs),
                Separator,
                Command(C::ExportDataSources),
                Command(C::ImportDataSources),
            ],
        },
        Menu {
            title: "View",
            entries: vec![
                Submenu(theme()),
                Command(C::ShowKeymap),
                Command(C::ShowSettings),
                Command(C::ResetLayout),
            ],
        },
        code(),
        Menu {
            title: "Help",
            entries: vec![Command(C::CheckForUpdates), Command(C::About)],
        },
    ]
}

fn open_recent(files: impl IntoIterator<Item = PathBuf>) -> Menu {
    let mut entries: Vec<Entry> = files
        .into_iter()
        .map(|path| Entry::Command(AppCommand::OpenRecent(path)))
        .collect();
    if entries.is_empty() {
        entries.push(Entry::Note("No recent files"));
    } else {
        entries.push(Entry::Separator);
        entries.push(Entry::Command(AppCommand::ClearRecentFiles));
    }
    Menu {
        title: "Open recent",
        entries,
    }
}

fn theme() -> Menu {
    Menu {
        title: "Theme",
        entries: [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ]
        .into_iter()
        .map(|preference| Entry::Command(AppCommand::Theme(preference)))
        .collect(),
    }
}

/// Formatting, for the console whose tab has focus.
fn code() -> Menu {
    Menu {
        title: "Code",
        entries: vec![
            Entry::Command(AppCommand::Format),
            Entry::Command(AppCommand::FormatAll),
        ],
    }
}

/// Every command in `menus` and their submenus, in menu order.
#[cfg(test)]
pub fn commands_in(menus: &[Menu]) -> Vec<&AppCommand> {
    fn walk<'a>(entries: &'a [Entry], found: &mut Vec<&'a AppCommand>) {
        for entry in entries {
            match entry {
                Entry::Command(command) => found.push(command),
                Entry::Submenu(menu) => walk(&menu.entries, found),
                Entry::Separator | Entry::Note(_) => {}
            }
        }
    }
    let mut found = Vec::new();
    for menu in menus {
        walk(&menu.entries, &mut found);
    }
    found
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const NOTHING_OPEN: CommandState = CommandState {
        console_focused: false,
        has_data_sources: false,
        theme: ThemePreference::System,
    };

    fn menu<'a>(menus: &'a [Menu], title: &str) -> &'a Menu {
        menus
            .iter()
            .find(|menu| menu.title == title)
            .unwrap_or_else(|| panic!("no {title} menu"))
    }

    #[test]
    fn ids_are_unique() {
        let menus = egui_menu_bar([PathBuf::from("/tmp/a.sql")]);
        let mut seen = HashSet::new();
        for command in commands_in(&menus) {
            assert!(seen.insert(command.id()), "{} is used twice", command.id());
        }
    }

    #[test]
    fn the_menu_bar_has_the_menus_in_order() {
        let titles: Vec<_> = egui_menu_bar([]).iter().map(|menu| menu.title).collect();
        assert_eq!(titles, ["File", "View", "Code", "Help"]);
    }

    #[test]
    fn open_recent_lists_the_files_then_clear() {
        let menus = egui_menu_bar([PathBuf::from("/work/a.sql"), PathBuf::from("b.sql")]);
        let Some(Entry::Submenu(recent)) = menu(&menus, "File").entries.get(1) else {
            panic!("File's second entry is not Open recent");
        };
        assert_eq!(recent.title, "Open recent");
        let labels: Vec<_> = commands_in(std::slice::from_ref(recent))
            .iter()
            .map(|command| command.label().into_owned())
            .collect();
        assert_eq!(labels, ["a.sql", "b.sql", "Clear recent files"]);
        assert_eq!(
            AppCommand::OpenRecent(PathBuf::from("/work/a.sql"))
                .tooltip()
                .as_deref(),
            Some(PathBuf::from("/work/a.sql").display().to_string().as_str())
        );
    }

    #[test]
    fn open_recent_says_when_there_are_none() {
        let menus = egui_menu_bar([]);
        let Some(Entry::Submenu(recent)) = menu(&menus, "File").entries.get(1) else {
            panic!("File's second entry is not Open recent");
        };
        assert_eq!(recent.entries, [Entry::Note("No recent files")]);
    }

    #[test]
    fn console_commands_wait_for_a_focused_console() {
        let console = CommandState {
            console_focused: true,
            ..NOTHING_OPEN
        };
        for command in [
            AppCommand::Save,
            AppCommand::SaveAs,
            AppCommand::Format,
            AppCommand::FormatAll,
        ] {
            assert!(!command.enabled(&NOTHING_OPEN), "{command:?}");
            assert!(command.enabled(&console), "{command:?}");
            assert_eq!(
                command.disabled_reason(),
                Some("Select a query console first")
            );
        }
        assert!(AppCommand::OpenFile.enabled(&NOTHING_OPEN));
    }

    #[test]
    fn exporting_waits_for_a_data_source() {
        let export = AppCommand::ExportDataSources;
        assert!(!export.enabled(&NOTHING_OPEN));
        assert!(export.enabled(&CommandState {
            has_data_sources: true,
            ..NOTHING_OPEN
        }));
        assert_eq!(
            export.disabled_reason(),
            Some("There are no data sources to export yet")
        );
    }

    #[test]
    fn the_picked_theme_is_checked() {
        let dark = CommandState {
            theme: ThemePreference::Dark,
            ..NOTHING_OPEN
        };
        let checked: Vec<_> = [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ]
        .into_iter()
        .map(|preference| AppCommand::Theme(preference).checked(&dark))
        .collect();
        assert_eq!(checked, [Some(false), Some(false), Some(true)]);
        assert_eq!(AppCommand::About.checked(&dark), None);
    }

    #[test]
    fn keymap_commands_show_and_answer_to_their_action() {
        for command in commands_in(&egui_menu_bar([])) {
            if let Some(action) = command.shortcut() {
                assert_eq!(AppCommand::for_action(action).as_ref(), Some(command));
                assert_eq!(command.id(), action.id());
                assert_eq!(command.tooltip().as_deref(), Some(action.description()));
            }
        }
        assert_eq!(AppCommand::for_action(keymap::Action::RunCurrent), None);
    }
}
