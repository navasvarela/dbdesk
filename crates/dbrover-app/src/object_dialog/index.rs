//! New index… on a table, and Drop index….

use dbrover_core::{CatalogPath, FolderKind, NodeKind};
use dbrover_sql::{DdlError, DdlOperation, Dialect, IndexMethod, IndexSpec, ObjectName};
use egui::{Grid, TextEdit, Ui, WidgetType};

use super::fields::{column_picker, columns_folder, loaded_columns};
use super::table::TableAt;
use super::{Destruction, FormContext, ObjectForm};
use crate::widgets;

const NAME_WIDTH: f32 = 260.0;

/// A table's indexes folder, which changes when one of its indexes does.
fn indexes_folder(table: &CatalogPath) -> CatalogPath {
    table.child(
        NodeKind::Folder(FolderKind::Indexes),
        FolderKind::Indexes.label(),
    )
}

/// The table whose node is at `path`, or whose indexes folder or index is.
fn table_of(path: &CatalogPath) -> Option<TableAt> {
    let table = match path.last()?.kind {
        NodeKind::Table => path.clone(),
        NodeKind::Folder(FolderKind::Indexes) => path.parent()?,
        NodeKind::Index => path.parent()?.parent()?,
        _ => return None,
    };
    TableAt::new(table)
}

pub(crate) struct NewIndex {
    table: TableAt,
    name: String,
    /// Whether the user typed the name; until then it follows the columns.
    named: bool,
    columns: Vec<String>,
    unique: bool,
    method: Option<IndexMethod>,
}

impl NewIndex {
    /// On a table or its indexes folder.
    pub(crate) fn new(path: &CatalogPath) -> Option<Self> {
        if path.last()?.kind == NodeKind::Index {
            return None;
        }
        Some(Self {
            table: table_of(path)?,
            name: String::new(),
            named: false,
            columns: Vec::new(),
            unique: false,
            method: None,
        })
    }

    /// `orders_customer_placed_idx`, as PostgreSQL would name it.
    fn suggested_name(&self) -> String {
        let mut parts = vec![self.table.name().name.clone()];
        parts.extend(self.columns.iter().cloned());
        parts.push(if self.unique { "key" } else { "idx" }.to_owned());
        parts.join("_").replace(' ', "_")
    }
}

impl ObjectForm for NewIndex {
    fn title(&self) -> String {
        format!("New index on {}", self.table.name().name)
    }

    fn show(&mut self, ui: &mut Ui, context: &FormContext<'_>) {
        let columns = loaded_columns(context.catalog, self.table.path());
        let methods = context.target.dialect.capabilities().index_methods;
        Grid::new("new_index").num_columns(2).show(ui, |ui| {
            ui.label("Columns");
            match &columns {
                Some(columns) => column_picker(ui, &mut self.columns, columns, "Index columns"),
                None => {
                    ui.weak("Loading the table's columns…");
                }
            }
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut self.unique, "Unique");
            ui.end_row();
            if !methods.is_empty() {
                ui.label("Method");
                method_picker(ui, &mut self.method, methods);
                ui.end_row();
            }
            let label = ui.label("Name");
            if !self.named {
                self.name = self.suggested_name();
            }
            let field = ui
                .add(TextEdit::singleline(&mut self.name).desired_width(NAME_WIDTH))
                .labelled_by(label.id);
            if field.changed() {
                self.named = true;
            }
            ui.end_row();
        });
    }

    fn catalog_requests(&self) -> Vec<CatalogPath> {
        vec![columns_folder(self.table.path())]
    }

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        Ok(vec![DdlOperation::CreateIndex(IndexSpec {
            name: self.name.trim().to_owned(),
            table: self.table.name().clone(),
            columns: self.columns.clone(),
            unique: self.unique,
            method: self.method,
        })])
    }

    fn changed(&self) -> CatalogPath {
        indexes_folder(self.table.path())
    }
}

fn method_picker(ui: &mut Ui, method: &mut Option<IndexMethod>, methods: &[IndexMethod]) {
    let label = |method: Option<IndexMethod>| method.map_or("Default", IndexMethod::label);
    let combo = egui::ComboBox::from_id_salt("index_method")
        .selected_text(label(*method))
        .show_ui(ui, |ui| {
            ui.selectable_value(method, None, label(None));
            for option in methods {
                ui.selectable_value(method, Some(*option), option.label());
            }
        });
    widgets::name(combo.response, WidgetType::ComboBox, "Index method", true);
}

pub(crate) struct DropIndex {
    table: TableAt,
    index: String,
}

impl DropIndex {
    /// On an index.
    pub(crate) fn new(path: &CatalogPath) -> Option<Self> {
        let index = path.last()?;
        if index.kind != NodeKind::Index {
            return None;
        }
        Some(Self {
            index: index.name.clone(),
            table: table_of(path)?,
        })
    }
}

impl ObjectForm for DropIndex {
    fn title(&self) -> String {
        format!("Drop index {}", self.index)
    }

    fn show(&mut self, _ui: &mut Ui, _context: &FormContext<'_>) {}

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        Ok(vec![DdlOperation::DropIndex {
            table: self.table.name().clone(),
            index: self.index.clone(),
        }])
    }

    fn changed(&self) -> CatalogPath {
        indexes_folder(self.table.path())
    }

    fn destruction(&self, dialect: Dialect) -> Option<Destruction> {
        let table = self.table.name();
        let index = ObjectName::new(table.schema.as_deref(), &self.index);
        Some(Destruction {
            name: self.index.clone(),
            summary: format!(
                "Drops the index {} on {}. The table and its rows stay; queries that used the \
                 index may get slower.",
                index.to_sql(dialect),
                table.to_sql(dialect)
            ),
            consequences: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orders() -> CatalogPath {
        CatalogPath::root()
            .child(NodeKind::Schema, "main")
            .child(NodeKind::Folder(FolderKind::Tables), "tables")
            .child(NodeKind::Table, "orders")
    }

    #[test]
    fn finds_the_table_from_the_table_its_indexes_or_an_index() {
        let folder = indexes_folder(&orders());
        let index = folder.child(NodeKind::Index, "orders_customer_idx");
        for path in [orders(), folder.clone()] {
            let form = NewIndex::new(&path).map(|form| form.changed());
            assert_eq!(form, Some(folder.clone()), "{path}");
        }
        assert!(NewIndex::new(&index).is_none());
        let drop = DropIndex::new(&index).map(|form| form.changed());
        assert_eq!(drop, Some(folder));
        assert!(DropIndex::new(&orders()).is_none());
    }

    #[test]
    fn names_the_index_after_its_columns_until_named_by_hand() {
        let mut form = NewIndex::new(&orders()).unwrap();
        form.columns = vec!["customer".to_owned(), "placed at".to_owned()];
        assert_eq!(form.suggested_name(), "orders_customer_placed_at_idx");
        form.unique = true;
        assert_eq!(form.suggested_name(), "orders_customer_placed_at_key");
    }
}
