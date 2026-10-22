//! The dockable panel layout: which tabs exist, where they start, and what each one draws.

use egui::{Id, Ui, WidgetText};
use egui_dock::{DockState, NodeIndex, TabViewer};
use serde::{Deserialize, Serialize};

use crate::explorer::{self, ExplorerAction, ExplorerOptions};
use crate::log_console::LogConsole;
use crate::state::DataSources;

/// Share of the window height for the explorer and query consoles; the log console gets the
/// rest, spanning the full width as DataGrip's bottom tool windows do.
const UPPER_AREA_SHARE: f32 = 0.72;
/// Share of the upper area's width for query consoles; the explorer gets the rest.
const CONSOLES_SHARE_OF_UPPER_AREA: f32 = 0.8;

/// Every kind of tab the dock can hold.
///
/// Saved with the layout in eframe's storage. A saved layout that names a variant which no
/// longer exists fails to load, and the app falls back to [`default_layout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    Explorer,
    Welcome,
    LogConsole,
}

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Self::Explorer => "Explorer",
            Self::Welcome => "Welcome",
            Self::LogConsole => "Log console",
        }
    }
}

pub fn default_layout() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Welcome]);
    let tree = dock.main_surface_mut();
    let [upper_area, _log_console] =
        tree.split_below(NodeIndex::root(), UPPER_AREA_SHARE, vec![Tab::LogConsole]);
    tree.split_left(
        upper_area,
        CONSOLES_SHARE_OF_UPPER_AREA,
        vec![Tab::Explorer],
    );
    dock
}

/// Draws the content of each tab.
pub struct Panels<'a> {
    pub log_console: &'a mut LogConsole,
    pub data_sources: &'a DataSources,
    pub explorer_options: &'a mut ExplorerOptions,
    /// What the user asked for in the explorer this frame, for the app to act on.
    pub explorer_actions: &'a mut Vec<ExplorerAction>,
}

impl TabViewer for Panels<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> Id {
        Id::new(*tab)
    }

    fn title(&mut self, tab: &mut Tab) -> WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Tab) {
        match tab {
            Tab::Explorer => explorer::show(
                ui,
                self.data_sources,
                self.explorer_options,
                self.explorer_actions,
            ),
            Tab::Welcome => welcome(ui),
            Tab::LogConsole => self.log_console.ui(ui),
        }
    }

    /// The explorer and log console can't be reopened from a menu yet, so they can't be closed.
    fn is_closeable(&self, tab: &Tab) -> bool {
        matches!(tab, Tab::Welcome)
    }
}

fn welcome(ui: &mut Ui) {
    ui.heading("dbdesk");
    ui.label("Query consoles will open here.");
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
