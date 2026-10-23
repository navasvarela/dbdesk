//! The dockable panel layout: which tabs exist, where they start, and what each one draws.

use std::collections::BTreeMap;

use dbdesk_core::SourceColor;
use dbdesk_engine::ConsoleId;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Id, Ui, WidgetText};
use egui_dock::tab_viewer::OnCloseResponse;
use egui_dock::{DockState, NodeIndex, TabStyle, TabViewer};
use serde::{Deserialize, Serialize};

use crate::console::{self, ConsoleAction, QueryConsole};
use crate::explorer::{self, ExplorerAction, ExplorerOptions};
use crate::fonts;
use crate::icons::{self, Icon, Role};
use crate::log_console::LogConsole;
use crate::state::DataSources;
use crate::theme::{self, Palette};
use crate::welcome;

/// Tab titles are a little smaller than body text, like an IDE's.
const TAB_TITLE_SIZE: f32 = 12.5;
/// The band in a console's data source colour along the top of the console.
const SOURCE_BAND_HEIGHT: f32 = 2.0;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    Explorer,
    Welcome,
    LogConsole,
    /// The number of a [`ConsoleId`]. Consoles aren't saved yet (M4), so these tabs are
    /// dropped when a saved layout is loaded; see [`without_consoles`].
    QueryConsole(u64),
}

impl Tab {
    pub fn console(id: ConsoleId) -> Self {
        Self::QueryConsole(id.0)
    }
}

/// `layout` minus its console tabs, whose consoles don't survive a restart yet.
pub fn without_consoles(mut layout: DockState<Tab>) -> DockState<Tab> {
    layout.retain_tabs(|tab| !matches!(tab, Tab::QueryConsole(_)));
    layout
}

/// Adds a console tab next to the other consoles (or the Welcome tab) and focuses it.
pub fn open_console_tab(layout: &mut DockState<Tab>, id: ConsoleId) {
    let tab = Tab::console(id);
    let tree = layout.main_surface_mut();
    match tree.find_tab_from(|tab| matches!(tab, Tab::QueryConsole(_) | Tab::Welcome)) {
        Some((node, _)) => tree[node].append_tab(tab),
        // The consoles' area was closed; give them a new one beside everything else.
        None => {
            tree.split_right(NodeIndex::root(), EXPLORER_SHARE_OF_UPPER_AREA, vec![tab]);
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
    pub palette: &'static Palette,
    pub log_console: &'a mut LogConsole,
    pub data_sources: &'a DataSources,
    pub explorer_options: &'a mut ExplorerOptions,
    /// What the user asked for in the explorer this frame, for the app to act on.
    pub explorer_actions: &'a mut Vec<ExplorerAction>,
    pub consoles: &'a mut BTreeMap<ConsoleId, QueryConsole>,
    /// What the user asked of each console this frame.
    pub console_actions: &'a mut Vec<(ConsoleId, ConsoleAction)>,
    /// Consoles whose tabs were closed this frame.
    pub closed_consoles: &'a mut Vec<ConsoleId>,
}

impl TabViewer for Panels<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> Id {
        Id::new(*tab)
    }

    fn title(&mut self, tab: &mut Tab) -> WidgetText {
        let palette = self.palette;
        let tool_window = |icon: Icon, title: &str| {
            tab_title(palette, icon, title, fonts::semibold(TAB_TITLE_SIZE))
        };
        match tab {
            Tab::Explorer => tool_window(icons::node(dbdesk_core::NodeKind::Database), "Explorer"),
            Tab::Welcome => tool_window(
                Icon {
                    glyph: icons::SPARKLE,
                    role: Role::Primary,
                },
                "Welcome",
            ),
            Tab::LogConsole => tool_window(
                Icon {
                    glyph: icons::LIST_DASHES,
                    role: Role::Weak,
                },
                "Log console",
            ),
            Tab::QueryConsole(id) => {
                let id = ConsoleId(*id);
                let title = self
                    .consoles
                    .get(&id)
                    .map_or("console", QueryConsole::title);
                let role = self.console_color(id).map_or(Role::Secondary, Role::Source);
                let icon = Icon {
                    glyph: icons::FILE_SQL,
                    role,
                };
                tab_title(palette, icon, title, FontId::proportional(TAB_TITLE_SIZE))
            }
        }
    }

    fn tab_style_override(&self, tab: &Tab, global_style: &TabStyle) -> Option<TabStyle> {
        let Tab::QueryConsole(id) = tab else {
            return None;
        };
        let color = self.console_color(ConsoleId(*id))?;
        Some(theme::source_tab_style(global_style, self.palette, color))
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Tab) {
        match tab {
            Tab::Explorer => explorer::show(
                ui,
                self.data_sources,
                self.explorer_options,
                self.explorer_actions,
            ),
            Tab::Welcome => welcome::show(ui, self.data_sources, self.explorer_actions),
            Tab::LogConsole => self.log_console.ui(ui),
            Tab::QueryConsole(id) => {
                let id = ConsoleId(*id);
                if let Some(color) = self.console_color(id) {
                    source_band(ui, self.palette.source(color));
                }
                if let Some(query_console) = self.consoles.get_mut(&id) {
                    let mut actions = Vec::new();
                    console::show(ui, query_console, &mut actions);
                    self.console_actions
                        .extend(actions.into_iter().map(|action| (id, action)));
                }
            }
        }
    }

    /// The explorer and log console can't be reopened from a menu yet, so they can't be closed.
    fn is_closeable(&self, tab: &Tab) -> bool {
        matches!(tab, Tab::Welcome | Tab::QueryConsole(_))
    }

    fn on_close(&mut self, tab: &mut Tab) -> OnCloseResponse {
        if let Tab::QueryConsole(id) = tab {
            self.closed_consoles.push(ConsoleId(*id));
        }
        OnCloseResponse::Close
    }
}

impl Panels<'_> {
    /// The colour of the data source a console runs on, looked up every frame so a colour
    /// changed in the data source dialog shows at once.
    fn console_color(&self, id: ConsoleId) -> Option<SourceColor> {
        let data_source = self.consoles.get(&id)?.data_source();
        self.data_sources.color_of(data_source)
    }
}

/// An icon in its palette colour, then the title in the tab's own text colour.
fn tab_title(palette: &Palette, icon: Icon, title: &str, font: FontId) -> WidgetText {
    let mut job = LayoutJob::default();
    job.append(
        icon.glyph,
        0.0,
        TextFormat::simple(fonts::icon(TAB_TITLE_SIZE + 1.0), icon.role.color(palette)),
    );
    // PLACEHOLDER takes the tab's text colour, which egui_dock varies with the tab's state.
    job.append(title, 6.0, TextFormat::simple(font, Color32::PLACEHOLDER));
    job.into()
}

/// A thin band in the data source's colour along the top of a console.
fn source_band(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), SOURCE_BAND_HEIGHT),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(rect, 1.0, color);
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
