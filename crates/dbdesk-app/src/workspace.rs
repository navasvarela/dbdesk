//! The dockable panel layout: which tabs exist, where they start, and what each one draws.

use std::collections::BTreeMap;

use dbdesk_store::ConsoleKey;
use egui::{Id, Ui, WidgetText};
use egui_dock::tab_viewer::OnCloseResponse;
use egui_dock::{DockState, NodeIndex, TabViewer};
use serde::{Deserialize, Serialize};

use crate::console::{self, ConsoleAction, QueryConsole};
use crate::consoles::SavedConsoles;
use crate::explorer::{self, ExplorerAction, ExplorerOptions};
use crate::keymap::Keymap;
use crate::log_console::LogConsole;
use crate::state::DataSources;

/// Share of the window height for the explorer and query consoles; the log console gets the
/// rest, spanning the full width as DataGrip's bottom tool windows do.
const UPPER_AREA_SHARE: f32 = 0.72;
/// Share of the upper area's width for query consoles; the explorer gets the rest.
const CONSOLES_SHARE_OF_UPPER_AREA: f32 = 0.8;
/// egui_dock's split `fraction` is the share of the first (left or top) node, whatever its
/// docs say about "the old node", so consoles to the right of the explorer take the rest.
const EXPLORER_SHARE_OF_UPPER_AREA: f32 = 1.0 - CONSOLES_SHARE_OF_UPPER_AREA;

/// Every kind of tab the dock can hold.
///
/// Saved with the layout in eframe's storage. A saved layout that names a variant which no
/// longer exists fails to load, and the app falls back to [`default_layout`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    Explorer,
    Welcome,
    LogConsole,
    /// A console, by its file. Restored at start-up once the engine has read the console
    /// files; see [`restore_consoles`].
    QueryConsole(ConsoleKey),
}

/// The consoles whose tabs the layout holds, so they can be reopened at start-up.
pub fn console_tabs(layout: &DockState<Tab>) -> Vec<ConsoleKey> {
    layout
        .iter_all_tabs()
        .filter_map(|(_, tab)| match tab {
            Tab::QueryConsole(key) => Some(key.clone()),
            _ => None,
        })
        .collect()
}

/// Drops the console tabs `reopen` can't reopen (their file or data source is gone), so a
/// restored layout never shows an empty tab.
pub fn restore_consoles(layout: &mut DockState<Tab>, mut reopen: impl FnMut(&ConsoleKey) -> bool) {
    for key in console_tabs(layout) {
        if !reopen(&key) {
            close_console_tab(layout, &key);
        }
    }
}

pub fn close_console_tab(layout: &mut DockState<Tab>, key: &ConsoleKey) {
    if let Some(path) = layout.find_tab(&Tab::QueryConsole(key.clone())) {
        layout.remove_tab(path);
    }
}

/// Adds a console tab next to the other consoles (or the Welcome tab) and focuses it.
pub fn open_console_tab(layout: &mut DockState<Tab>, key: ConsoleKey) {
    let tab = Tab::QueryConsole(key);
    let tree = layout.main_surface_mut();
    match tree.find_tab_from(|tab| matches!(tab, Tab::QueryConsole(_) | Tab::Welcome)) {
        Some((node, _)) => tree[node].append_tab(tab.clone()),
        // The consoles' area was closed; give them a new one beside everything else.
        None => {
            tree.split_right(
                NodeIndex::root(),
                EXPLORER_SHARE_OF_UPPER_AREA,
                vec![tab.clone()],
            );
        }
    }
    if let Some(path) = layout.find_tab(&tab) {
        // The tab was just found, so activating it can't fail.
        let _ = layout.set_active_tab(path);
    }
}

pub fn default_layout() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Welcome]);
    let tree = dock.main_surface_mut();
    let [upper_area, _log_console] =
        tree.split_below(NodeIndex::root(), UPPER_AREA_SHARE, vec![Tab::LogConsole]);
    tree.split_left(
        upper_area,
        EXPLORER_SHARE_OF_UPPER_AREA,
        vec![Tab::Explorer],
    );
    dock
}

/// Draws the content of each tab.
pub struct Panels<'a> {
    pub log_console: &'a mut LogConsole,
    pub data_sources: &'a DataSources,
    pub saved_consoles: &'a SavedConsoles,
    pub explorer_options: &'a mut ExplorerOptions,
    /// What the user asked for in the explorer this frame, for the app to act on.
    pub explorer_actions: &'a mut Vec<ExplorerAction>,
    pub consoles: &'a mut BTreeMap<ConsoleKey, QueryConsole>,
    pub keymap: &'a Keymap,
    /// What the user asked of each console this frame.
    pub console_actions: &'a mut Vec<(ConsoleKey, ConsoleAction)>,
    /// Consoles whose tabs the user asked to close this frame. The tabs stay until the app
    /// closes them, which may first ask about rolling back an open transaction.
    pub close_requests: &'a mut Vec<ConsoleKey>,
}

impl TabViewer for Panels<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> Id {
        Id::new(&*tab)
    }

    fn title(&mut self, tab: &mut Tab) -> WidgetText {
        match tab {
            Tab::Explorer => "Explorer".into(),
            Tab::Welcome => "Welcome".into(),
            Tab::LogConsole => "Log console".into(),
            Tab::QueryConsole(key) => self
                .consoles
                .get(key)
                .map_or("console", QueryConsole::title)
                .into(),
        }
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Tab) {
        match tab {
            Tab::Explorer => explorer::show(
                ui,
                &explorer::ExplorerView {
                    data_sources: self.data_sources,
                    saved_consoles: self.saved_consoles,
                },
                self.explorer_options,
                self.explorer_actions,
            ),
            Tab::Welcome => welcome(ui),
            Tab::LogConsole => self.log_console.ui(ui),
            Tab::QueryConsole(key) => match self.consoles.get_mut(key) {
                Some(query_console) => {
                    let mut actions = Vec::new();
                    console::show(ui, query_console, self.keymap, &mut actions);
                    self.console_actions
                        .extend(actions.into_iter().map(|action| (key.clone(), action)));
                }
                // Restored from the saved layout, waiting for the engine to read its file.
                None => {
                    ui.weak("Loading…");
                }
            },
        }
    }

    /// The explorer and log console can't be reopened from a menu yet, so they can't be closed.
    fn is_closeable(&self, tab: &Tab) -> bool {
        matches!(tab, Tab::Welcome | Tab::QueryConsole(_))
    }

    fn on_close(&mut self, tab: &mut Tab) -> OnCloseResponse {
        match tab {
            Tab::QueryConsole(key) => {
                self.close_requests.push(key.clone());
                OnCloseResponse::Ignore
            }
            _ => OnCloseResponse::Close,
        }
    }
}

fn welcome(ui: &mut Ui) {
    ui.heading("dbdesk");
    ui.label("Query consoles open here: right-click a data source and choose New query console.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_has_every_tab_once() {
        let dock = default_layout();

        for tab in [Tab::Explorer, Tab::Welcome, Tab::LogConsole] {
            let count = dock.iter_all_tabs().filter(|(_, t)| **t == tab).count();
            assert_eq!(count, 1, "{tab:?}");
        }
    }
}
