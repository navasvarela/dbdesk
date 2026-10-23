//! The connection explorer panel: data sources and their catalogs as a lazily loaded tree.
//! It only reports what the user asked for; the app turns that into engine commands.

use dbdesk_core::{CatalogNode, CatalogPath, DataSourceId, DriverKind, NodeKind, Quoting};
use dbdesk_engine::SessionState;
use dbdesk_sql::Dialect;
use dbdesk_store::ConsoleKey;
use egui::{
    Color32, CornerRadius, Id, Label, Rect, Response, ScrollArea, Sense, Shape, TextEdit,
    TextStyle, Ui, WidgetInfo, WidgetType, vec2,
};

use crate::catalog_tree::{CatalogTree, Children};
use crate::consoles::SavedConsoles;
use crate::icons;
use crate::state::{DataSourceEntry, DataSources};
use crate::theme::Palette;
use crate::widgets::{self, IconButton};

const STATUS_DOT_RADIUS: f32 = 3.5;
/// How strongly a data source's colour tints its row.
const SOURCE_ROW_TINT: f32 = 0.16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplorerAction {
    New(DriverKind),
    Edit(DataSourceId),
    Delete(DataSourceId),
    Connect(DataSourceId),
    Disconnect(DataSourceId),
    Expand(DataSourceId, CatalogPath),
    Collapse(DataSourceId, CatalogPath),
    Refresh(DataSourceId, CatalogPath),
    ShowDefinition {
        id: DataSourceId,
        path: CatalogPath,
        title: String,
    },
    NewConsole(DataSourceId),
    /// Reopens a console from its file.
    OpenSavedConsole(ConsoleKey),
    /// Opens a console that selects everything from the table or view at `path`.
    OpenData(DataSourceId, CatalogPath),
}

/// The explorer's own view settings.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExplorerOptions {
    /// Case-insensitive text matched against loaded node labels. Never triggers loading, so
    /// typing doesn't send a catalog query per keystroke.
    pub filter: String,
    /// Show `pg_catalog`, `information_schema`, `sqlite_sequence` and the like.
    pub show_system: bool,
}

/// What the explorer shows.
pub struct ExplorerView<'a> {
    pub data_sources: &'a DataSources,
    /// Offered for reopening in each data source's menu.
    pub saved_consoles: &'a SavedConsoles,
}

pub fn show(
    ui: &mut Ui,
    view: &ExplorerView<'_>,
    options: &mut ExplorerOptions,
    actions: &mut Vec<ExplorerAction>,
) {
    toolbar(ui, options, actions);
    ui.separator();
    match view.data_sources {
        DataSources::Loading => {
            ui.weak("Loading data sources…");
        }
        DataSources::Unavailable(error) => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Can't read data sources: {error}"),
            );
        }
        DataSources::Loaded(entries) if entries.is_empty() => {
            ui.weak("No data sources yet. Add one with +.");
        }
        DataSources::Loaded(entries) => {
            let needle = options.filter.trim().to_lowercase();
            let filter = TreeFilter {
                needle: (!needle.is_empty()).then_some(needle.as_str()),
                show_system: options.show_system,
            };
            ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                for entry in entries {
                    data_source_tree(ui, entry, view.saved_consoles, &filter, actions);
                }
            });
        }
    }
}

fn toolbar(ui: &mut Ui, options: &mut ExplorerOptions, actions: &mut Vec<ExplorerAction>) {
    let palette = Palette::for_visuals(ui.visuals());
    widgets::toolbar(ui, |ui| {
        let new = ui.menu_button(widgets::glyph(icons::PLUS).size(15.0), |ui| {
            new_data_source_menu(ui, actions);
        });
        widgets::name(
            new.response.on_hover_text("New data source"),
            WidgetType::Button,
            "New data source",
            true,
        );
        let (system_icon, system_color) = if options.show_system {
            (icons::EYE, palette.primary)
        } else {
            (icons::EYE_SLASH, palette.text_weak)
        };
        if IconButton::new(system_icon, "Show system schemas and tables")
            .color(system_color)
            .show(ui)
            .clicked()
        {
            options.show_system = !options.show_system;
        }
        ui.add_space(4.0);
        ui.label(widgets::glyph(icons::MAGNIFYING_GLASS).color(palette.text_weak));
        ui.add(
            TextEdit::singleline(&mut options.filter)
                .hint_text("Filter loaded nodes")
                .desired_width(f32::INFINITY),
        );
    });
}

/// One entry per driver, each with its icon. Also used by the Welcome tab.
pub fn new_data_source_menu(ui: &mut Ui, actions: &mut Vec<ExplorerAction>) {
    let palette = Palette::for_visuals(ui.visuals());
    let logo_size = ui.text_style_height(&TextStyle::Button);
    for driver in DriverKind::ALL {
        let logo = icons::driver_logo_sized(driver, palette, logo_size);
        let response = widgets::name(
            ui.add(egui::Button::image_and_text(logo, driver.display_name())),
            WidgetType::Button,
            driver.display_name(),
            true,
        );
        if response.clicked() {
            actions.push(ExplorerAction::New(driver));
        }
    }
}

fn data_source_tree(
    ui: &mut Ui,
    entry: &DataSourceEntry,
    saved_consoles: &SavedConsoles,
    filter: &TreeFilter<'_>,
    actions: &mut Vec<ExplorerAction>,
) {
    let id = entry.data_source.id;
    let root = CatalogPath::root();
    let expanded = entry.catalog.is_expanded(&root);
    let palette = Palette::for_visuals(ui.visuals());
    let row_background = ui.painter().add(Shape::Noop);
    let row = ui.horizontal(|ui| {
        if expander(ui, expanded, &entry.data_source.name).clicked() {
            actions.push(toggle_expansion(id, &root, expanded));
        }
        let logo_size = ui.text_style_height(&TextStyle::Body);
        ui.add(icons::driver_logo_sized(
            entry.data_source.config.driver(),
            palette,
            logo_size,
        ));
        let name = ui.add(
            Label::new(&entry.data_source.name)
                .selectable(false)
                .sense(Sense::click()),
        );
        status_dot(ui, &entry.session, palette);
        name
    });
    if let Some(color) = entry.data_source.color {
        let rect = Rect::from_x_y_ranges(ui.max_rect().x_range(), row.response.rect.y_range());
        ui.painter().set(
            row_background,
            Shape::rect_filled(
                rect.expand2(vec2(0.0, 1.0)),
                CornerRadius::same(4),
                palette.tint(palette.source(color), SOURCE_ROW_TINT),
            ),
        );
    }
    let response = row.inner.on_hover_text(tooltip(entry));

    if response.double_clicked() {
        actions.push(toggle_expansion(id, &root, expanded));
    }
    response.context_menu(|ui| data_source_menu(ui, entry, saved_consoles, actions));
    if expanded {
        let tree = TreeView {
            id,
            catalog: &entry.catalog,
            quoting: Dialect::from(entry.data_source.config.driver()).quoting(),
        };
        ui.indent(Id::new(id), |ui| {
            children(ui, &tree, &root, filter, actions)
        });
    }
}

fn data_source_menu(
    ui: &mut Ui,
    entry: &DataSourceEntry,
    saved_consoles: &SavedConsoles,
    actions: &mut Vec<ExplorerAction>,
) {
    let id = entry.data_source.id;
    let connected = matches!(
        entry.session,
        SessionState::Connecting | SessionState::Connected(_)
    );
    if connected {
        if widgets::menu_item(ui, icons::PLUGS, "Disconnect").clicked() {
            actions.push(ExplorerAction::Disconnect(id));
        }
        if widgets::menu_item(ui, icons::ARROWS_CLOCKWISE, "Refresh").clicked() {
            actions.push(ExplorerAction::Refresh(id, CatalogPath::root()));
        }
    } else if widgets::menu_item(ui, icons::PLUGS_CONNECTED, "Connect").clicked() {
        actions.push(ExplorerAction::Connect(id));
    }
    if widgets::menu_item(ui, icons::TERMINAL_WINDOW, "New query console").clicked() {
        actions.push(ExplorerAction::NewConsole(id));
    }
    saved_consoles_menu(ui, id, saved_consoles, actions);
    ui.separator();
    if widgets::menu_item(ui, icons::PENCIL_SIMPLE, "Edit…").clicked() {
        actions.push(ExplorerAction::Edit(id));
    }
    if widgets::menu_item(ui, icons::TRASH, "Delete…").clicked() {
        actions.push(ExplorerAction::Delete(id));
    }
}

/// One data source's loaded catalog, as the tree functions need it.
struct TreeView<'a> {
    id: DataSourceId,
    catalog: &'a CatalogTree,
    /// For the names the context menu copies.
    quoting: Quoting,
}

/// Which loaded nodes to draw.
struct TreeFilter<'a> {
    /// Lowercase filter text; `None` shows everything.
    needle: Option<&'a str>,
    show_system: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Hidden,
    /// Drawn as usual: there is no filter, or the node itself matches it.
    Shown,
    /// Drawn only because something loaded below it matches; forced open to reveal it.
    LeadsToMatch,
}

impl TreeFilter<'_> {
    fn visibility(&self, catalog: &CatalogTree, node: &CatalogNode) -> Visibility {
        if node.system && !self.show_system {
            return Visibility::Hidden;
        }
        let Some(needle) = self.needle else {
            return Visibility::Shown;
        };
        if node.label.to_lowercase().contains(needle) {
            Visibility::Shown
        } else if self.has_matching_descendant(catalog, &node.path) {
            Visibility::LeadsToMatch
        } else {
            Visibility::Hidden
        }
    }

    fn has_matching_descendant(&self, catalog: &CatalogTree, path: &CatalogPath) -> bool {
        let Some(Children::Loaded(nodes)) = catalog.children(path) else {
            return false;
        };
        nodes
            .iter()
            .any(|node| self.visibility(catalog, node) != Visibility::Hidden)
    }

    /// Below a node that matched, everything is shown: the match is what the user looked for.
    fn below_match(&self) -> Self {
        Self {
            needle: None,
            show_system: self.show_system,
        }
    }
}

fn children(
    ui: &mut Ui,
    tree: &TreeView<'_>,
    path: &CatalogPath,
    filter: &TreeFilter<'_>,
    actions: &mut Vec<ExplorerAction>,
) {
    match tree.catalog.children(path) {
        None => {}
        Some(Children::Loading) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Loading…");
            });
        }
        Some(Children::Failed(error)) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        Some(Children::Loaded(nodes)) if nodes.is_empty() => {
            ui.weak("empty");
        }
        Some(Children::Loaded(nodes)) => {
            for node in nodes {
                match filter.visibility(tree.catalog, node) {
                    Visibility::Hidden => {}
                    Visibility::Shown => {
                        node_row(ui, tree, node, false, &filter.below_match(), actions);
                    }
                    Visibility::LeadsToMatch => node_row(ui, tree, node, true, filter, actions),
                }
            }
        }
    }
}

fn node_row(
    ui: &mut Ui,
    tree: &TreeView<'_>,
    node: &CatalogNode,
    forced_open: bool,
    filter: &TreeFilter<'_>,
    actions: &mut Vec<ExplorerAction>,
) {
    let expanded = forced_open || tree.catalog.is_expanded(&node.path);
    let response = ui
        .horizontal(|ui| {
            if node.has_children {
                if expander(ui, expanded, &node.label).clicked() {
                    actions.push(toggle_expansion(tree.id, &node.path, expanded));
                }
            } else {
                ui.add_space(ui.spacing().icon_width);
            }
            if let Some(kind) = node.kind() {
                ui.label(icons::node(kind).rich_text(Palette::for_visuals(ui.visuals())));
            }
            let label = ui.add(
                Label::new(&node.label)
                    .selectable(false)
                    .sense(Sense::click()),
            );
            if let Some(detail) = &node.detail {
                ui.weak(detail);
            }
            if let Some(count) = folder_count(tree.catalog, node, filter.show_system) {
                widgets::count_badge(ui, count);
            }
            label
        })
        .inner;

    if node.has_children && response.double_clicked() {
        actions.push(toggle_expansion(tree.id, &node.path, expanded));
    }
    response.context_menu(|ui| node_menu(ui, tree, node, actions));
    if expanded {
        ui.indent(Id::new((tree.id, &node.path)), |ui| {
            children(ui, tree, &node.path, filter, actions);
        });
    }
}

fn node_menu(
    ui: &mut Ui,
    tree: &TreeView<'_>,
    node: &CatalogNode,
    actions: &mut Vec<ExplorerAction>,
) {
    let id = tree.id;
    let holds_rows = matches!(
        node.kind(),
        Some(NodeKind::Table | NodeKind::View | NodeKind::MaterializedView)
    );
    if holds_rows && widgets::menu_item(ui, icons::TABLE, "Open data").clicked() {
        actions.push(ExplorerAction::OpenData(id, node.path.clone()));
    }
    if let Some(name) = node.path.qualified_name(tree.quoting)
        && widgets::menu_item(ui, icons::COPY, "Copy name").clicked()
    {
        ui.ctx().copy_text(name);
    }
    if node.has_definition && widgets::menu_item(ui, icons::FILE_SQL, "Show definition").clicked() {
        actions.push(ExplorerAction::ShowDefinition {
            id,
            path: node.path.clone(),
            title: node.label.clone(),
        });
    }
    if node.has_children && widgets::menu_item(ui, icons::ARROWS_CLOCKWISE, "Refresh").clicked() {
        actions.push(ExplorerAction::Refresh(id, node.path.clone()));
    }
}

/// Lists the data source's closed console files, if there are any.
fn saved_consoles_menu(
    ui: &mut Ui,
    id: DataSourceId,
    saved_consoles: &SavedConsoles,
    actions: &mut Vec<ExplorerAction>,
) {
    if saved_consoles.names(id).next().is_none() {
        return;
    }
    ui.menu_button("Open saved console", |ui| {
        for name in saved_consoles.names(id) {
            if ui.button(format!("{name}.sql")).clicked() {
                actions.push(ExplorerAction::OpenSavedConsole(ConsoleKey {
                    data_source: id,
                    name: name.clone(),
                }));
            }
        }
    });
}

fn toggle_expansion(id: DataSourceId, path: &CatalogPath, expanded: bool) -> ExplorerAction {
    if expanded {
        ExplorerAction::Collapse(id, path.clone())
    } else {
        ExplorerAction::Expand(id, path.clone())
    }
}

/// The disclosure triangle. Labelled "Expand x" / "Collapse x" for screen readers and tests.
fn expander(ui: &mut Ui, expanded: bool, label: &str) -> Response {
    let size = ui.spacing().icon_width;
    let (_, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let verb = if expanded { "Collapse" } else { "Expand" };
    response
        .widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, format!("{verb} {label}")));
    let caret = if expanded {
        icons::CARET_DOWN
    } else {
        icons::CARET_RIGHT
    };
    let color = if response.hovered() {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    ui.painter().text(
        response.rect.center(),
        egui::Align2::CENTER_CENTER,
        caret,
        crate::fonts::icon(size * 0.8),
        color,
    );
    response
}

/// The number of children a loaded folder shows, for the badge after its label.
fn folder_count(catalog: &CatalogTree, node: &CatalogNode, show_system: bool) -> Option<usize> {
    if !matches!(node.kind(), Some(NodeKind::Folder(_))) {
        return None;
    }
    let Some(Children::Loaded(children)) = catalog.children(&node.path) else {
        return None;
    };
    Some(
        children
            .iter()
            .filter(|child| show_system || !child.system)
            .count(),
    )
}

/// The connection state as a dot after the name. Disconnected data sources get none, so the
/// eye goes to the ones that are live or in trouble.
fn status_dot(ui: &mut Ui, session: &SessionState, palette: &Palette) {
    let size = vec2(
        STATUS_DOT_RADIUS * 2.0,
        ui.text_style_height(&TextStyle::Body),
    );
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if let Some(color) = status_color(session, palette) {
        ui.painter()
            .circle_filled(rect.center(), STATUS_DOT_RADIUS, color);
    }
}

fn status_color(session: &SessionState, palette: &Palette) -> Option<Color32> {
    match session {
        SessionState::Disconnected => None,
        SessionState::Connecting => Some(palette.warning),
        SessionState::Connected(_) => Some(palette.success),
        SessionState::Failed(_) => Some(palette.error),
    }
}

fn tooltip(entry: &DataSourceEntry) -> String {
    let config = &entry.data_source.config;
    let status = match &entry.session {
        SessionState::Disconnected => "Disconnected".to_owned(),
        SessionState::Connecting => "Connecting…".to_owned(),
        SessionState::Connected(server) => format!("Connected to {server}"),
        SessionState::Failed(error) => format!("Connection failed: {error}"),
    };
    format!("{} · {config}\n{status}", config.driver().display_name())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(parent: &CatalogPath, name: &str, system: bool) -> CatalogNode {
        CatalogNode {
            path: parent.child(NodeKind::Schema, name),
            label: name.to_owned(),
            detail: None,
            has_children: true,
            has_definition: false,
            system,
        }
    }

    #[test]
    fn filter_shows_matches_their_ancestors_and_respects_system_objects() {
        let root = CatalogPath::root();
        let public = node(&root, "public", false);
        let users = node(&public.path, "users", false);
        let pg_catalog = node(&root, "pg_catalog", true);
        let mut catalog = CatalogTree::default();
        assert!(catalog.expand(&root));
        catalog.loaded(&root, &Ok(vec![public.clone(), pg_catalog.clone()]));
        assert!(catalog.expand(&public.path));
        catalog.loaded(&public.path, &Ok(vec![users.clone()]));

        let cases = [
            (None, false, &public, Visibility::Shown),
            (None, false, &pg_catalog, Visibility::Hidden),
            (None, true, &pg_catalog, Visibility::Shown),
            (Some("user"), false, &public, Visibility::LeadsToMatch),
            (Some("user"), false, &users, Visibility::Shown),
            (Some("pub"), false, &public, Visibility::Shown),
            (Some("orders"), false, &public, Visibility::Hidden),
        ];
        for (needle, show_system, node, expected) in cases {
            let filter = TreeFilter {
                needle,
                show_system,
            };
            assert_eq!(
                filter.visibility(&catalog, node),
                expected,
                "{needle:?} {show_system} {}",
                node.label
            );
        }
    }
}
