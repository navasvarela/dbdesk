//! Helpers for views and materialized views: New view…, Edit…, Rename…, Refresh… and Drop….

use dbrover_core::{CatalogPath, FolderKind, NodeKind};
use dbrover_sql::{
    DdlError, DdlOperation, Dialect, ObjectName, ViewKind, ViewReplacement, ViewSpec, view_query,
};
use egui::{Grid, TextEdit, Ui};

use super::fields::sql_editor;
use super::{Destruction, FormContext, ObjectForm, object_name};

const NAME_WIDTH: f32 = 280.0;

fn noun(kind: ViewKind) -> &'static str {
    match kind {
        ViewKind::View => "view",
        ViewKind::Materialized => "materialized view",
    }
}

/// The folder a schema lists its views of `kind` in.
fn views_folder(schema: &CatalogPath, kind: ViewKind) -> CatalogPath {
    let folder = match kind {
        ViewKind::View => FolderKind::Views,
        ViewKind::Materialized => FolderKind::MaterializedViews,
    };
    schema.child(NodeKind::Folder(folder), folder.label())
}

/// The view or materialized view at a catalog path.
#[derive(Debug, Clone)]
pub(crate) struct ViewAt {
    path: CatalogPath,
    name: ObjectName,
    kind: ViewKind,
}

impl ViewAt {
    /// `None` unless `path` is a view's or a materialized view's.
    pub(crate) fn new(path: CatalogPath) -> Option<Self> {
        let kind = match path.last()?.kind {
            NodeKind::View => ViewKind::View,
            NodeKind::MaterializedView => ViewKind::Materialized,
            _ => return None,
        };
        let name = object_name(&path)?;
        Some(Self { path, name, kind })
    }

    /// The folder the view is listed in, which changes when the view does.
    fn folder(&self) -> CatalogPath {
        self.path.parent().unwrap_or_default()
    }
}

/// The fields New view and Edit view share: the query, and for a materialized view whether
/// to fill it straight away.
struct QueryFields {
    kind: ViewKind,
    query: String,
    with_data: bool,
}

impl QueryFields {
    fn new(kind: ViewKind) -> Self {
        Self {
            kind,
            query: String::new(),
            with_data: true,
        }
    }

    fn show(&mut self, ui: &mut Ui, dialect: Dialect) {
        let label = ui.label("Query");
        sql_editor(ui, &mut self.query, dialect, "view_query").labelled_by(label.id);
        if self.kind == ViewKind::Materialized {
            ui.checkbox(
                &mut self.with_data,
                "Fill it now; unticked, it stays empty until refreshed (WITH NO DATA)",
            );
        }
    }

    fn spec(&self, name: ObjectName) -> ViewSpec {
        ViewSpec {
            name,
            kind: self.kind,
            query: self.query.clone(),
            with_data: self.with_data,
        }
    }
}

/// A new view, or materialized view, in a schema.
pub(crate) struct NewView {
    /// The schema's folder of views of this kind, where the new one appears.
    folder: CatalogPath,
    schema: String,
    name: String,
    fields: QueryFields,
    /// Whether a view of this kind by that name is listed already.
    taken: bool,
    /// Replace the view by that name instead of failing.
    replace: bool,
}

impl NewView {
    /// On a schema, or its folder of views of `kind`.
    pub(crate) fn new(path: CatalogPath, kind: ViewKind) -> Option<Self> {
        let schema = match path.last()?.kind {
            NodeKind::Schema => path,
            NodeKind::Folder(FolderKind::Views) => path.parent()?,
            NodeKind::Folder(FolderKind::MaterializedViews) if kind == ViewKind::Materialized => {
                path.parent()?
            }
            _ => return None,
        };
        Some(Self {
            schema: schema.name_of(NodeKind::Schema)?.to_owned(),
            folder: views_folder(&schema, kind),
            name: String::new(),
            fields: QueryFields::new(kind),
            taken: false,
            replace: false,
        })
    }

    fn name(&self) -> ObjectName {
        ObjectName::new(Some(self.schema.trim()), self.name.trim())
    }
}

impl ObjectForm for NewView {
    fn title(&self) -> String {
        format!("New {}", noun(self.fields.kind))
    }

    fn show(&mut self, ui: &mut Ui, context: &FormContext<'_>) {
        Grid::new("new_view_name").num_columns(2).show(ui, |ui| {
            let label = ui.label("Schema");
            ui.add(TextEdit::singleline(&mut self.schema).desired_width(NAME_WIDTH))
                .labelled_by(label.id);
            ui.end_row();
            let label = ui.label("Name");
            ui.add(TextEdit::singleline(&mut self.name).desired_width(NAME_WIDTH))
                .labelled_by(label.id);
            ui.end_row();
        });
        let name = self.name.trim();
        self.taken = !name.is_empty()
            && context
                .catalog
                .loaded_children(&self.folder)
                .is_some_and(|views| views.iter().any(|view| view.label == name));
        if self.taken {
            ui.checkbox(
                &mut self.replace,
                format!(
                    "Replace the {} \u{201c}{name}\u{201d}, which exists",
                    noun(self.fields.kind)
                ),
            );
        }
        self.fields.show(ui, context.target.dialect);
    }

    fn catalog_requests(&self) -> Vec<CatalogPath> {
        vec![self.folder.clone()]
    }

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        let spec = self.fields.spec(self.name());
        Ok(vec![if self.taken && self.replace {
            DdlOperation::ReplaceView(spec)
        } else {
            DdlOperation::CreateView(spec)
        }])
    }

    fn changed(&self) -> CatalogPath {
        self.folder.clone()
    }

    fn destruction(&self, dialect: Dialect) -> Option<Destruction> {
        (self.taken && self.replace)
            .then(|| replacement(&self.name(), self.fields.kind, dialect))
            .flatten()
    }
}

/// What replacing a view's query destroys, where the dialect drops the view to do it: a
/// materialized view's indexes, and the grants on either kind.
fn replacement(view: &ObjectName, kind: ViewKind, dialect: Dialect) -> Option<Destruction> {
    let drops = kind == ViewKind::Materialized
        || dialect.capabilities().view_replacement == ViewReplacement::DropAndCreate;
    drops.then(|| Destruction {
        name: view.name.clone(),
        summary: format!(
            "Drops the {} {} and creates it again with the new query.",
            noun(kind),
            view.to_sql(dialect)
        ),
        consequences: match kind {
            ViewKind::View => Vec::new(),
            ViewKind::Materialized => vec![
                "Its indexes go with it, and the database refuses if other views read it."
                    .to_owned(),
            ],
        },
    })
}

/// Edit view…: the view's query, from its definition, to change and save.
pub(crate) struct EditView {
    view: ViewAt,
    fields: QueryFields,
    definition: Definition,
}

enum Definition {
    Loading,
    Loaded,
    /// The definition came, but not as `CREATE VIEW … AS query`: the whole of it is in the
    /// editor, to cut down to the query by hand.
    Unrecognised,
    Failed(String),
}

impl EditView {
    pub(crate) fn new(view: ViewAt) -> Self {
        Self {
            fields: QueryFields::new(view.kind),
            view,
            definition: Definition::Loading,
        }
    }
}

impl ObjectForm for EditView {
    fn title(&self) -> String {
        format!("Edit {} {}", noun(self.view.kind), self.view.name.name)
    }

    fn show(&mut self, ui: &mut Ui, context: &FormContext<'_>) {
        match &self.definition {
            Definition::Loading => {
                ui.weak(format!(
                    "Loading the {}'s definition…",
                    noun(self.view.kind)
                ));
                return;
            }
            Definition::Failed(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
                return;
            }
            Definition::Unrecognised => {
                ui.weak("This is the whole definition: keep only the query.");
            }
            Definition::Loaded => {}
        }
        self.fields.show(ui, context.target.dialect);
    }

    fn definition_request(&self) -> Option<CatalogPath> {
        Some(self.view.path.clone())
    }

    fn definition_loaded(&mut self, result: &Result<String, String>, dialect: Dialect) {
        match result {
            Ok(definition) => match view_query(definition, dialect) {
                Some(query) => {
                    self.fields.query = query;
                    self.definition = Definition::Loaded;
                }
                None => {
                    self.fields.query = definition.clone();
                    self.definition = Definition::Unrecognised;
                }
            },
            Err(error) => {
                self.definition = Definition::Failed(format!(
                    "Could not load the {}'s definition: {error}",
                    noun(self.view.kind)
                ));
            }
        }
    }

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        Ok(vec![DdlOperation::ReplaceView(
            self.fields.spec(self.view.name.clone()),
        )])
    }

    fn changed(&self) -> CatalogPath {
        self.view.folder()
    }

    fn destruction(&self, dialect: Dialect) -> Option<Destruction> {
        replacement(&self.view.name, self.view.kind, dialect)
    }
}

/// Gives the view another name in its schema.
pub(crate) struct RenameView {
    view: ViewAt,
    new_name: String,
}

impl RenameView {
    pub(crate) fn new(view: ViewAt) -> Self {
        Self {
            new_name: view.name.name.clone(),
            view,
        }
    }
}

impl ObjectForm for RenameView {
    fn title(&self) -> String {
        format!("Rename {} {}", noun(self.view.kind), self.view.name.name)
    }

    fn show(&mut self, ui: &mut Ui, _context: &FormContext<'_>) {
        ui.horizontal(|ui| {
            let label = ui.label("New name");
            ui.add(TextEdit::singleline(&mut self.new_name))
                .labelled_by(label.id);
        });
    }

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        Ok(vec![DdlOperation::RenameView {
            view: self.view.name.clone(),
            kind: self.view.kind,
            new_name: self.new_name.trim().to_owned(),
        }])
    }

    fn changed(&self) -> CatalogPath {
        self.view.folder()
    }
}

/// Runs a materialized view's query again and stores its rows.
pub(crate) struct RefreshView {
    view: ViewAt,
    concurrently: bool,
}

impl RefreshView {
    /// On a materialized view.
    pub(crate) fn new(view: ViewAt) -> Option<Self> {
        (view.kind == ViewKind::Materialized).then_some(Self {
            view,
            concurrently: false,
        })
    }

    fn indexes(&self) -> CatalogPath {
        self.view.path.child(
            NodeKind::Folder(FolderKind::Indexes),
            FolderKind::Indexes.label(),
        )
    }
}

impl ObjectForm for RefreshView {
    fn title(&self) -> String {
        format!("Refresh materialized view {}", self.view.name.name)
    }

    fn show(&mut self, ui: &mut Ui, context: &FormContext<'_>) {
        ui.label(format!(
            "Runs the query of {} again and stores its rows in place of the old ones.",
            self.view.name.to_sql(context.target.dialect)
        ));
        // PostgreSQL refreshes concurrently only a view with a unique index, which tells it
        // which old row each new one replaces.
        let has_unique_index = context
            .catalog
            .loaded_children(&self.indexes())
            .is_some_and(|indexes| {
                indexes
                    .iter()
                    .any(|index| index.detail.as_deref() == Some("unique"))
            });
        if !has_unique_index {
            self.concurrently = false;
        }
        ui.add_enabled(
            has_unique_index,
            egui::Checkbox::new(
                &mut self.concurrently,
                "Keep the old rows readable meanwhile (CONCURRENTLY)",
            ),
        )
        .on_disabled_hover_text("Needs a unique index on the materialized view");
    }

    fn catalog_requests(&self) -> Vec<CatalogPath> {
        vec![self.indexes()]
    }

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        Ok(vec![DdlOperation::RefreshMaterializedView {
            view: self.view.name.clone(),
            concurrently: self.concurrently,
        }])
    }

    fn changed(&self) -> CatalogPath {
        self.view.path.clone()
    }
}

/// Drops the view, and with `CASCADE` (where the dialect has it) what depends on it.
pub(crate) struct DropView {
    view: ViewAt,
    cascade: bool,
}

impl DropView {
    pub(crate) fn new(view: ViewAt) -> Self {
        Self {
            view,
            cascade: false,
        }
    }
}

impl ObjectForm for DropView {
    fn title(&self) -> String {
        format!("Drop {} {}", noun(self.view.kind), self.view.name.name)
    }

    fn show(&mut self, ui: &mut Ui, context: &FormContext<'_>) {
        if context.target.dialect.capabilities().drop_cascade {
            ui.checkbox(&mut self.cascade, "Also drop what depends on it (CASCADE)");
        }
    }

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        Ok(vec![DdlOperation::DropView {
            view: self.view.name.clone(),
            kind: self.view.kind,
            cascade: self.cascade,
        }])
    }

    fn changed(&self) -> CatalogPath {
        self.view.folder()
    }

    fn destruction(&self, dialect: Dialect) -> Option<Destruction> {
        let stored = match self.view.kind {
            ViewKind::View => "The tables it reads stay as they are.",
            ViewKind::Materialized => {
                "Its stored rows and indexes go with it; the tables it reads stay as they are."
            }
        };
        let consequences = if self.cascade {
            vec!["CASCADE also drops the views that read it.".to_owned()]
        } else if dialect.capabilities().drop_cascade {
            vec![
                "If another view reads it, the database refuses: tick CASCADE to drop that too."
                    .to_owned(),
            ]
        } else {
            Vec::new()
        };
        Some(Destruction {
            name: self.view.name.name.clone(),
            summary: format!(
                "Drops the {} {}. {stored}",
                noun(self.view.kind),
                self.view.name.to_sql(dialect)
            ),
            consequences,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main() -> CatalogPath {
        CatalogPath::root().child(NodeKind::Schema, "main")
    }

    fn big_orders() -> CatalogPath {
        views_folder(&main(), ViewKind::View).child(NodeKind::View, "big_orders")
    }

    #[test]
    fn new_views_go_in_the_folder_of_their_kind() {
        let views = views_folder(&main(), ViewKind::View);
        let materialized = views_folder(&main(), ViewKind::Materialized);
        let cases = [
            (main(), ViewKind::View, Some(views.clone())),
            (views.clone(), ViewKind::View, Some(views.clone())),
            (
                views.clone(),
                ViewKind::Materialized,
                Some(materialized.clone()),
            ),
            (main(), ViewKind::Materialized, Some(materialized.clone())),
            (materialized.clone(), ViewKind::View, None),
            (big_orders(), ViewKind::View, None),
        ];
        for (path, kind, expected) in cases {
            let folder = NewView::new(path.clone(), kind).map(|form| form.changed());
            assert_eq!(folder, expected, "{path} {kind:?}");
        }
    }

    #[test]
    fn edit_view_starts_from_the_query_in_the_definition() {
        let mut form = EditView::new(ViewAt::new(big_orders()).unwrap());
        form.definition_loaded(
            &Ok("CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 100".to_owned()),
            Dialect::Sqlite,
        );
        assert_eq!(form.fields.query, "SELECT * FROM orders WHERE total > 100");
        assert_eq!(
            form.operations()
                .and_then(|operations| dbrover_sql::ddl_statements(&operations, Dialect::Sqlite)),
            Ok(vec![
                "DROP VIEW main.big_orders".to_owned(),
                "CREATE VIEW main.big_orders AS\nSELECT * FROM orders WHERE total > 100".to_owned(),
            ])
        );
        assert!(form.destruction(Dialect::Sqlite).is_some());
        assert!(form.destruction(Dialect::Postgres).is_none());
    }

    #[test]
    fn only_a_materialized_view_refreshes() {
        assert!(RefreshView::new(ViewAt::new(big_orders()).unwrap()).is_none());
        let materialized = views_folder(&main(), ViewKind::Materialized)
            .child(NodeKind::MaterializedView, "totals");
        assert!(RefreshView::new(ViewAt::new(materialized).unwrap()).is_some());
    }
}
