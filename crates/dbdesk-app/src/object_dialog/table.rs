//! Helpers that act on an existing table: Rename…, Truncate… and Drop….

use dbdesk_core::{CatalogPath, NodeKind};
use dbdesk_sql::{DdlError, DdlOperation, Dialect, ObjectName};
use egui::{TextEdit, Ui};

use super::{Destruction, FormContext, ObjectForm, object_name};

/// The table at a catalog path, as both the builder and the user see it.
#[derive(Debug, Clone)]
pub(crate) struct TableAt {
    path: CatalogPath,
    name: ObjectName,
}

impl TableAt {
    /// `None` unless `path` is a table's.
    pub(crate) fn new(path: CatalogPath) -> Option<Self> {
        if path.last()?.kind != NodeKind::Table {
            return None;
        }
        let name = object_name(&path)?;
        Some(Self { path, name })
    }

    pub(crate) fn path(&self) -> &CatalogPath {
        &self.path
    }

    pub(crate) fn name(&self) -> &ObjectName {
        &self.name
    }

    /// The folder the table is listed in, which changes when the table does.
    fn folder(&self) -> CatalogPath {
        self.path.parent().unwrap_or_default()
    }

    fn qualified(&self, dialect: Dialect) -> String {
        self.name.to_sql(dialect)
    }
}

/// Gives the table another name in its schema.
pub(crate) struct RenameTable {
    table: TableAt,
    new_name: String,
}

impl RenameTable {
    pub(crate) fn new(table: TableAt) -> Self {
        Self {
            new_name: table.name.name.clone(),
            table,
        }
    }
}

impl ObjectForm for RenameTable {
    fn title(&self) -> String {
        format!("Rename table {}", self.table.name.name)
    }

    fn show(&mut self, ui: &mut Ui, _context: &FormContext<'_>) {
        ui.horizontal(|ui| {
            let label = ui.label("New name");
            ui.add(TextEdit::singleline(&mut self.new_name))
                .labelled_by(label.id);
        });
    }

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        Ok(vec![DdlOperation::RenameTable {
            table: self.table.name.clone(),
            new_name: self.new_name.trim().to_owned(),
        }])
    }

    fn changed(&self) -> CatalogPath {
        self.table.folder()
    }
}

/// Deletes every row and keeps the table.
pub(crate) struct TruncateTable {
    table: TableAt,
}

impl TruncateTable {
    pub(crate) fn new(table: TableAt) -> Self {
        Self { table }
    }
}

impl ObjectForm for TruncateTable {
    fn title(&self) -> String {
        format!("Truncate table {}", self.table.name.name)
    }

    fn show(&mut self, _ui: &mut Ui, _context: &FormContext<'_>) {}

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        Ok(vec![DdlOperation::TruncateTable(self.table.name.clone())])
    }

    fn changed(&self) -> CatalogPath {
        self.table.folder()
    }

    fn destruction(&self, dialect: Dialect) -> Option<Destruction> {
        Some(Destruction {
            name: self.table.name.name.clone(),
            summary: format!(
                "Deletes every row in the table {}. The table stays, empty.",
                self.table.qualified(dialect)
            ),
            consequences: Vec::new(),
        })
    }
}

/// Drops the table, and with `CASCADE` (where the dialect has it) what depends on it.
pub(crate) struct DropTable {
    table: TableAt,
    cascade: bool,
}

impl DropTable {
    pub(crate) fn new(table: TableAt) -> Self {
        Self {
            table,
            cascade: false,
        }
    }
}

impl ObjectForm for DropTable {
    fn title(&self) -> String {
        format!("Drop table {}", self.table.name.name)
    }

    fn show(&mut self, ui: &mut Ui, context: &FormContext<'_>) {
        if context.target.dialect.capabilities().drop_cascade {
            ui.checkbox(&mut self.cascade, "Also drop what depends on it (CASCADE)");
        }
    }

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        Ok(vec![DdlOperation::DropTable {
            table: self.table.name.clone(),
            cascade: self.cascade,
        }])
    }

    fn changed(&self) -> CatalogPath {
        self.table.folder()
    }

    fn destruction(&self, dialect: Dialect) -> Option<Destruction> {
        let consequences = if self.cascade {
            vec![
                "CASCADE also drops the views that read it and the foreign keys of other tables \
                 that point at it."
                    .to_owned(),
            ]
        } else if dialect.capabilities().drop_cascade {
            vec![
                "If a view or another table's foreign key depends on it, the database refuses: \
                 tick CASCADE to drop those too."
                    .to_owned(),
            ]
        } else {
            Vec::new()
        };
        Some(Destruction {
            name: self.table.name.name.clone(),
            summary: format!(
                "Drops the table {}, with every row in it and its indexes, keys and triggers.",
                self.table.qualified(dialect)
            ),
            consequences,
        })
    }
}
