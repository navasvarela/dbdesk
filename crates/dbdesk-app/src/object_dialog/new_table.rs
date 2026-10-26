//! New table: the name and schema, a grid of columns, and tabs for unique keys, foreign keys
//! and checks.

use dbdesk_core::{CatalogPath, FolderKind, NodeKind};
use dbdesk_sql::{
    Capabilities, Check, ColumnSpec, DdlError, DdlOperation, ForeignKey, ObjectName,
    ReferentialAction, TableSpec, UniqueKey,
};
use egui::{Grid, Response, TextEdit, Ui, WidgetInfo, WidgetType};

use super::{FormContext, ObjectForm};
use crate::catalog_tree::CatalogTree;
use crate::{icons, widgets};

const NAME_WIDTH: f32 = 140.0;
const TYPE_WIDTH: f32 = 130.0;
const DEFAULT_WIDTH: f32 = 110.0;
const EXPRESSION_WIDTH: f32 = 260.0;

pub(crate) struct NewTable {
    /// The schema's tables folder, where the new table appears.
    folder: CatalogPath,
    schema: String,
    name: String,
    columns: Vec<ColumnRow>,
    unique_keys: Vec<UniqueRow>,
    foreign_keys: Vec<ForeignRow>,
    checks: Vec<CheckRow>,
    tab: Tab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Columns,
    UniqueKeys,
    ForeignKeys,
    Checks,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Self::Columns => "Columns",
            Self::UniqueKeys => "Unique keys",
            Self::ForeignKeys => "Foreign keys",
            Self::Checks => "Checks",
        }
    }

    /// The tabs the dialect has a use for.
    fn offered(capabilities: &Capabilities) -> Vec<Self> {
        [
            (Self::Columns, true),
            (Self::UniqueKeys, capabilities.unique_keys),
            (Self::ForeignKeys, capabilities.foreign_keys),
            (Self::Checks, true),
        ]
        .into_iter()
        .filter_map(|(tab, offered)| offered.then_some(tab))
        .collect()
    }
}

struct ColumnRow {
    name: String,
    data_type: String,
    nullable: bool,
    default: String,
    primary_key: bool,
}

#[derive(Default)]
struct UniqueRow {
    name: String,
    columns: Vec<String>,
}

#[derive(Default)]
struct ForeignRow {
    name: String,
    columns: Vec<String>,
    target: Option<Target>,
    target_columns: Vec<String>,
    on_delete: ReferentialAction,
    on_update: ReferentialAction,
}

/// The table a foreign key points at.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// The table being created, for a key between its own rows.
    ItSelf,
    /// A table in the catalog, at its path.
    Table { path: CatalogPath, name: ObjectName },
}

#[derive(Default)]
struct CheckRow {
    name: String,
    expression: String,
}

impl NewTable {
    /// A table in the schema at `schema`, or in the schema whose tables folder that is. `None`
    /// for any other node. Starts with one column, `id`, of the dialect's first common type,
    /// as the primary key: most tables begin that way.
    pub(crate) fn new(path: CatalogPath, first_type: &str) -> Option<Self> {
        let folder = match path.last()?.kind {
            NodeKind::Schema => path.child(
                NodeKind::Folder(FolderKind::Tables),
                FolderKind::Tables.label(),
            ),
            NodeKind::Folder(FolderKind::Tables) => path,
            _ => return None,
        };
        Some(Self {
            schema: folder.name_of(NodeKind::Schema)?.to_owned(),
            folder,
            name: String::new(),
            columns: vec![ColumnRow {
                name: "id".to_owned(),
                data_type: first_type.to_owned(),
                nullable: false,
                default: String::new(),
                primary_key: true,
            }],
            unique_keys: Vec::new(),
            foreign_keys: Vec::new(),
            checks: Vec::new(),
            tab: Tab::Columns,
        })
    }

    fn column_names(&self) -> Vec<String> {
        self.columns
            .iter()
            .map(|column| column.name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .collect()
    }

    fn spec(&self) -> TableSpec {
        TableSpec {
            name: ObjectName::new(Some(&self.schema), self.name.trim()),
            columns: self
                .columns
                .iter()
                .map(|column| ColumnSpec {
                    name: column.name.trim().to_owned(),
                    data_type: column.data_type.trim().to_owned(),
                    nullable: column.nullable && !column.primary_key,
                    default: optional(&column.default),
                })
                .collect(),
            primary_key: self
                .columns
                .iter()
                .filter(|column| column.primary_key)
                .map(|column| column.name.trim().to_owned())
                .collect(),
            unique_keys: self
                .unique_keys
                .iter()
                .map(|key| UniqueKey {
                    name: optional(&key.name),
                    columns: key.columns.clone(),
                })
                .collect(),
            foreign_keys: self
                .foreign_keys
                .iter()
                .map(|key| ForeignKey {
                    name: optional(&key.name),
                    columns: key.columns.clone(),
                    target: match &key.target {
                        Some(Target::ItSelf) => {
                            ObjectName::new(Some(&self.schema), self.name.trim())
                        }
                        Some(Target::Table { name, .. }) => name.clone(),
                        None => ObjectName::new(None, ""),
                    },
                    target_columns: key.target_columns.clone(),
                    on_delete: key.on_delete,
                    on_update: key.on_update,
                })
                .collect(),
            checks: self
                .checks
                .iter()
                .map(|check| Check {
                    name: optional(&check.name),
                    expression: check.expression.clone(),
                })
                .collect(),
        }
    }

    fn columns_tab(&mut self, ui: &mut Ui, capabilities: &Capabilities) {
        let mut removed = None;
        Grid::new("new_table_columns")
            .num_columns(6)
            .striped(true)
            .show(ui, |ui| {
                for heading in ["Name", "Type", "Nullable", "Default", "Primary key", ""] {
                    ui.strong(heading);
                }
                ui.end_row();
                for (index, column) in self.columns.iter_mut().enumerate() {
                    let row = index + 1;
                    text_cell(
                        ui,
                        &mut column.name,
                        NAME_WIDTH,
                        &format!("Column {row} name"),
                    );
                    type_cell(ui, column, capabilities.common_types, row);
                    let nullable = ui.add_enabled(
                        !column.primary_key,
                        egui::Checkbox::without_text(&mut column.nullable),
                    );
                    widgets::name(
                        nullable,
                        WidgetType::Checkbox,
                        &format!("Column {row} nullable"),
                        !column.primary_key,
                    );
                    text_cell(
                        ui,
                        &mut column.default,
                        DEFAULT_WIDTH,
                        &format!("Column {row} default"),
                    );
                    let key = ui.checkbox(&mut column.primary_key, "");
                    widgets::name(
                        key,
                        WidgetType::Checkbox,
                        &format!("Column {row} primary key"),
                        true,
                    );
                    if remove_button(ui, &format!("Remove column {row}")).clicked() {
                        removed = Some(index);
                    }
                    ui.end_row();
                }
            });
        if let Some(index) = removed {
            self.columns.remove(index);
        }
        if ui.button("Add column").clicked() {
            self.columns.push(ColumnRow {
                name: String::new(),
                data_type: String::new(),
                nullable: true,
                default: String::new(),
                primary_key: false,
            });
        }
    }

    fn unique_keys_tab(&mut self, ui: &mut Ui) {
        let columns = self.column_names();
        let mut removed = None;
        Grid::new("new_table_unique_keys")
            .num_columns(3)
            .show(ui, |ui| {
                for heading in ["Name (optional)", "Columns", ""] {
                    ui.strong(heading);
                }
                ui.end_row();
                for (index, key) in self.unique_keys.iter_mut().enumerate() {
                    let row = index + 1;
                    text_cell(
                        ui,
                        &mut key.name,
                        NAME_WIDTH,
                        &format!("Unique key {row} name"),
                    );
                    column_picker(
                        ui,
                        &mut key.columns,
                        &columns,
                        &format!("Unique key {row} columns"),
                    );
                    if remove_button(ui, &format!("Remove unique key {row}")).clicked() {
                        removed = Some(index);
                    }
                    ui.end_row();
                }
            });
        if let Some(index) = removed {
            self.unique_keys.remove(index);
        }
        if ui.button("Add unique key").clicked() {
            self.unique_keys.push(UniqueRow::default());
        }
    }

    fn foreign_keys_tab(&mut self, ui: &mut Ui, context: &FormContext<'_>) {
        let columns = self.column_names();
        let tables = cached_tables(context.catalog, &self.folder);
        let actions = context.target.dialect.capabilities().referential_actions;
        let mut removed = None;
        for (index, key) in self.foreign_keys.iter_mut().enumerate() {
            let row = index + 1;
            ui.group(|ui| {
                Grid::new(("new_table_foreign_key", index))
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Name (optional)");
                        text_cell(
                            ui,
                            &mut key.name,
                            NAME_WIDTH,
                            &format!("Foreign key {row} name"),
                        );
                        ui.end_row();
                        ui.label("Columns");
                        column_picker(
                            ui,
                            &mut key.columns,
                            &columns,
                            &format!("Foreign key {row} columns"),
                        );
                        ui.end_row();
                        ui.label("References");
                        target_picker(ui, key, &tables, row);
                        ui.end_row();
                        ui.label("Their columns");
                        let target_columns = match &key.target {
                            Some(Target::ItSelf) => Some(columns.clone()),
                            Some(Target::Table { path, .. }) => {
                                loaded_columns(context.catalog, path)
                            }
                            None => Some(Vec::new()),
                        };
                        match target_columns {
                            Some(target_columns) => column_picker(
                                ui,
                                &mut key.target_columns,
                                &target_columns,
                                &format!("Foreign key {row} referenced columns"),
                            ),
                            None => {
                                ui.weak("Loading its columns…");
                            }
                        }
                        ui.end_row();
                        if actions.len() > 1 {
                            ui.label("On delete");
                            action_picker(ui, &mut key.on_delete, actions, row, "delete");
                            ui.end_row();
                            ui.label("On update");
                            action_picker(ui, &mut key.on_update, actions, row, "update");
                            ui.end_row();
                        }
                    });
                if remove_button(ui, &format!("Remove foreign key {row}")).clicked() {
                    removed = Some(index);
                }
            });
        }
        if let Some(index) = removed {
            self.foreign_keys.remove(index);
        }
        if ui.button("Add foreign key").clicked() {
            self.foreign_keys.push(ForeignRow::default());
        }
    }

    fn checks_tab(&mut self, ui: &mut Ui) {
        let mut removed = None;
        Grid::new("new_table_checks").num_columns(3).show(ui, |ui| {
            for heading in ["Name (optional)", "Condition", ""] {
                ui.strong(heading);
            }
            ui.end_row();
            for (index, check) in self.checks.iter_mut().enumerate() {
                let row = index + 1;
                text_cell(
                    ui,
                    &mut check.name,
                    NAME_WIDTH,
                    &format!("Check {row} name"),
                );
                text_cell(
                    ui,
                    &mut check.expression,
                    EXPRESSION_WIDTH,
                    &format!("Check {row} condition"),
                );
                if remove_button(ui, &format!("Remove check {row}")).clicked() {
                    removed = Some(index);
                }
                ui.end_row();
            }
        });
        if let Some(index) = removed {
            self.checks.remove(index);
        }
        if ui.button("Add check").clicked() {
            self.checks.push(CheckRow::default());
        }
    }
}

impl ObjectForm for NewTable {
    fn title(&self) -> String {
        "New table".to_owned()
    }

    fn show(&mut self, ui: &mut Ui, context: &FormContext<'_>) {
        let capabilities = context.target.dialect.capabilities();
        Grid::new("new_table_name").num_columns(2).show(ui, |ui| {
            let label = ui.label("Schema");
            ui.add(TextEdit::singleline(&mut self.schema).desired_width(NAME_WIDTH * 2.0))
                .labelled_by(label.id);
            ui.end_row();
            let label = ui.label("Name");
            ui.add(TextEdit::singleline(&mut self.name).desired_width(NAME_WIDTH * 2.0))
                .labelled_by(label.id);
            ui.end_row();
        });
        ui.horizontal(|ui| {
            for tab in Tab::offered(capabilities) {
                ui.selectable_value(&mut self.tab, tab, tab.label());
            }
        });
        ui.separator();
        match self.tab {
            Tab::Columns => self.columns_tab(ui, capabilities),
            Tab::UniqueKeys => self.unique_keys_tab(ui),
            Tab::ForeignKeys => self.foreign_keys_tab(ui, context),
            Tab::Checks => self.checks_tab(ui),
        }
    }

    /// The schema's tables, for picking a foreign key's target, and each target's columns.
    fn catalog_requests(&self) -> Vec<CatalogPath> {
        let mut requests = vec![self.folder.clone()];
        for key in &self.foreign_keys {
            if let Some(Target::Table { path, .. }) = &key.target {
                requests.push(columns_folder(path));
            }
        }
        requests
    }

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        Ok(vec![DdlOperation::CreateTable(self.spec())])
    }

    fn changed(&self) -> CatalogPath {
        self.folder.clone()
    }
}

fn optional(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// A text field in a grid, named for screen readers (and tests) since the grid's heading is
/// its only visible label.
fn text_cell(ui: &mut Ui, text: &mut String, width: f32, label: &str) -> Response {
    // Sized explicitly: in a grid, a text field's desired width yields to the column's width
    // from the frame before, which starts as narrow as the heading.
    let height = ui.spacing().interact_size.y;
    let response = ui.add_sized([width, height], TextEdit::singleline(text));
    let value = text.clone();
    response.widget_info(|| {
        let mut info = WidgetInfo::text_edit(true, &value, &value, "");
        info.label = Some(label.to_owned());
        info
    });
    response
}

/// The type as typed, and a menu of the dialect's common types beside it.
fn type_cell(ui: &mut Ui, column: &mut ColumnRow, common: &[&str], row: usize) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        text_cell(
            ui,
            &mut column.data_type,
            TYPE_WIDTH,
            &format!("Column {row} type"),
        );
        let menu = ui.menu_button(widgets::glyph(icons::CARET_DOWN), |ui| {
            for data_type in common {
                if ui.button(*data_type).clicked() {
                    column.data_type = (*data_type).to_owned();
                    ui.close();
                }
            }
        });
        widgets::name(
            menu.response,
            WidgetType::Button,
            &format!("Column {row} common types"),
            true,
        );
    });
}

fn remove_button(ui: &mut Ui, label: &str) -> Response {
    widgets::IconButton::new(icons::X_CIRCLE, label).show(ui)
}

/// The columns of a key, in the order they were ticked, which is the key's order.
fn column_picker(ui: &mut Ui, chosen: &mut Vec<String>, columns: &[String], label: &str) {
    let text = if chosen.is_empty() {
        "Choose…".to_owned()
    } else {
        chosen.join(", ")
    };
    let menu = ui.menu_button(text, |ui| {
        if columns.is_empty() {
            ui.weak("No columns yet");
        }
        for column in columns {
            let mut ticked = chosen.contains(column);
            if ui.checkbox(&mut ticked, column).changed() {
                if ticked {
                    chosen.push(column.clone());
                } else {
                    chosen.retain(|other| other != column);
                }
            }
        }
    });
    widgets::name(menu.response, WidgetType::Button, label, true);
}

fn target_picker(
    ui: &mut Ui,
    key: &mut ForeignRow,
    tables: &[(CatalogPath, ObjectName)],
    row: usize,
) {
    let selected = match &key.target {
        None => "Choose…".to_owned(),
        Some(Target::ItSelf) => "This table".to_owned(),
        Some(Target::Table { name, .. }) => qualified_label(name),
    };
    let before = key.target.clone();
    let combo = egui::ComboBox::from_id_salt(("foreign_key_target", row))
        .selected_text(selected)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut key.target, Some(Target::ItSelf), "This table");
            for (path, name) in tables {
                let target = Some(Target::Table {
                    path: path.clone(),
                    name: name.clone(),
                });
                ui.selectable_value(&mut key.target, target, qualified_label(name));
            }
        });
    widgets::name(
        combo.response,
        WidgetType::ComboBox,
        &format!("Foreign key {row} references"),
        true,
    );
    if key.target != before {
        key.target_columns.clear();
    }
}

fn action_picker(
    ui: &mut Ui,
    action: &mut ReferentialAction,
    actions: &[ReferentialAction],
    row: usize,
    event: &str,
) {
    let combo = egui::ComboBox::from_id_salt(("foreign_key_action", row, event))
        .selected_text(action.sql())
        .show_ui(ui, |ui| {
            for option in actions {
                ui.selectable_value(action, *option, option.sql());
            }
        });
    widgets::name(
        combo.response,
        WidgetType::ComboBox,
        &format!("Foreign key {row} on {event}"),
        true,
    );
}

fn qualified_label(name: &ObjectName) -> String {
    match &name.schema {
        Some(schema) => format!("{schema}.{}", name.name),
        None => name.name.clone(),
    }
}

/// Every table loaded so far in the schemas beside the new table's, to point a foreign key
/// at. Only what the explorer or completion has loaded: listing a schema's tables is a query,
/// and the new table's own schema is asked for (see `catalog_requests`).
fn cached_tables(catalog: &CatalogTree, folder: &CatalogPath) -> Vec<(CatalogPath, ObjectName)> {
    let schemas_parent = folder
        .parent()
        .and_then(|schema| schema.parent())
        .unwrap_or_default();
    let Some(schemas) = catalog.loaded_children(&schemas_parent) else {
        return Vec::new();
    };
    schemas
        .iter()
        .filter(|schema| !schema.system && schema.kind() == Some(NodeKind::Schema))
        .flat_map(|schema| {
            let tables = schema.path.child(
                NodeKind::Folder(FolderKind::Tables),
                FolderKind::Tables.label(),
            );
            catalog
                .loaded_children(&tables)
                .unwrap_or_default()
                .iter()
                .filter(|table| table.kind() == Some(NodeKind::Table))
                .map(|table| {
                    let name = ObjectName::new(Some(&schema.label), &table.label);
                    (table.path.clone(), name)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn columns_folder(table: &CatalogPath) -> CatalogPath {
    table.child(
        NodeKind::Folder(FolderKind::Columns),
        FolderKind::Columns.label(),
    )
}

/// A table's column names, once its columns folder is loaded.
fn loaded_columns(catalog: &CatalogTree, table: &CatalogPath) -> Option<Vec<String>> {
    let columns = catalog.loaded_children(&columns_folder(table))?;
    Some(
        columns
            .iter()
            .filter(|column| column.kind() == Some(NodeKind::Column))
            .map(|column| column.label.clone())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables_folder() -> CatalogPath {
        CatalogPath::root()
            .child(NodeKind::Schema, "main")
            .child(NodeKind::Folder(FolderKind::Tables), "tables")
    }

    #[test]
    fn opens_on_a_schema_or_its_tables_folder_only() {
        let schema = CatalogPath::root().child(NodeKind::Schema, "main");
        let from_schema = NewTable::new(schema.clone(), "integer").map(|form| form.folder);
        let from_folder = NewTable::new(tables_folder(), "integer").map(|form| form.folder);
        assert_eq!(from_schema, Some(tables_folder()));
        assert_eq!(from_folder, Some(tables_folder()));
        let table = tables_folder().child(NodeKind::Table, "orders");
        assert!(NewTable::new(table, "integer").is_none());
    }

    #[test]
    fn writes_the_spec_the_rows_describe() {
        let mut form = NewTable::new(tables_folder(), "integer").unwrap();
        form.name = " orders ".to_owned();
        form.columns.push(ColumnRow {
            name: "customer".to_owned(),
            data_type: "integer".to_owned(),
            nullable: true,
            default: " ".to_owned(),
            primary_key: false,
        });
        form.foreign_keys.push(ForeignRow {
            columns: vec!["customer".to_owned()],
            target: Some(Target::Table {
                path: tables_folder().child(NodeKind::Table, "customers"),
                name: ObjectName::new(Some("main"), "customers"),
            }),
            target_columns: vec!["id".to_owned()],
            on_delete: ReferentialAction::Cascade,
            ..ForeignRow::default()
        });
        form.checks.push(CheckRow {
            name: String::new(),
            expression: "customer > 0".to_owned(),
        });
        let spec = form.spec();
        assert_eq!(spec.name, ObjectName::new(Some("main"), "orders"));
        assert_eq!(spec.primary_key, ["id"]);
        assert!(!spec.columns[0].nullable, "a key column is never nullable");
        assert_eq!(spec.columns[1].default, None);
        assert_eq!(spec.foreign_keys[0].target.name, "customers");
        assert_eq!(spec.checks[0].name, None);
    }
}
