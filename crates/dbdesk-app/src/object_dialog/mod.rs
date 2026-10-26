//! The object helpers' dialogs: a form saying what to create or change, then the SQL it will
//! run, shown before anything runs (the review panel). Nothing reaches the database until
//! Execute is clicked; Open in console and Copy hand the SQL over instead.
//!
//! Each helper is an [`ObjectForm`]; this module is the frame they share: the window, the
//! confirmation a destructive change needs, the review panel, and waiting for the engine.

mod confirm;
mod review;
mod table;

use dbdesk_core::{CatalogPath, DataSource, DataSourceId, NodeKind, SafetyLevel, SourceColor};
use dbdesk_engine::{Event, SessionState};
use dbdesk_sql::{DdlError, DdlOperation, Dialect, ObjectName, ddl_statements};
use egui::{Id, Ui, Window};

pub use confirm::Destruction;
use table::{DropTable, TableAt, TruncateTable};

use crate::explorer::Helper;
use review::{Review, ReviewAction};

/// The window's width when it opens; its height follows what the form shows.
const DEFAULT_WIDTH: f32 = 620.0;

/// The data source a dialog changes, and what about it decides how careful the dialog is.
#[derive(Debug, Clone)]
pub struct Target {
    pub data_source: DataSourceId,
    pub name: String,
    pub dialect: Dialect,
    pub safety: SafetyLevel,
    /// Coloured red, the colour that marks production: a destructive change asks for the
    /// object's name to be typed.
    pub red: bool,
}

impl Target {
    pub fn new(data_source: &DataSource) -> Self {
        Self {
            data_source: data_source.id,
            name: data_source.name.clone(),
            dialect: Dialect::from(data_source.config.driver()),
            safety: data_source.safety,
            red: data_source.color == Some(SourceColor::Red),
        }
    }
}

/// One helper's fields: what the user asks for, turned into operations for the builder.
pub trait ObjectForm {
    /// The window's title, e.g. "New table" or "Drop table orders".
    fn title(&self) -> String;

    /// Draws the fields.
    fn show(&mut self, ui: &mut Ui, target: &Target);

    /// What the form asks for, or why it can't be written as SQL yet.
    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError>;

    /// The catalog node whose children change once the operations are applied, refreshed
    /// afterwards.
    fn changed(&self) -> CatalogPath;

    /// For a change that destroys something, what it destroys, spelt out before Execute.
    fn destruction(&self, _dialect: Dialect) -> Option<Destruction> {
        None
    }
}

/// What the app does for an open dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectDialogAction {
    /// Runs the statements with `Command::ApplyDdl`; the dialog waits for the answer.
    Execute {
        data_source: DataSourceId,
        statements: Vec<String>,
    },
    /// Opens a new console on the data source holding `text`, and closes the dialog.
    OpenInConsole {
        data_source: DataSourceId,
        text: String,
    },
    Close,
}

/// The form for a helper the explorer asked for. `None` if its node isn't the kind the
/// helper works on, which the explorer's menus never offer.
pub fn form(helper: Helper) -> Option<Box<dyn ObjectForm>> {
    Some(match helper {
        Helper::TruncateTable(path) => Box::new(TruncateTable::new(TableAt::new(path)?)),
        Helper::DropTable(path) => Box::new(DropTable::new(TableAt::new(path)?)),
    })
}

pub struct ObjectDialog {
    target: Target,
    form: Box<dyn ObjectForm>,
    review: Review,
    confirmation: confirm::Confirmation,
}

impl ObjectDialog {
    pub fn new(target: Target, form: Box<dyn ObjectForm>) -> Self {
        Self {
            target,
            form,
            review: Review::default(),
            confirmation: confirm::Confirmation::default(),
        }
    }

    pub fn data_source(&self) -> DataSourceId {
        self.target.data_source
    }

    /// Follows the engine's answer to Execute. Returns the node to refresh once the change
    /// has been applied, which means the dialog is done.
    pub fn apply(&mut self, event: &Event) -> Option<CatalogPath> {
        let data_source = self.target.data_source;
        match event {
            Event::DdlApplied {
                data_source: answered,
                result,
            } if *answered == data_source => match result {
                Ok(()) => return Some(self.form.changed()),
                Err(failure) => self.review.failed(failure),
            },
            // A request sent while connecting is never answered if the connection fails.
            Event::SessionChanged { id, state } if *id == data_source => match state {
                SessionState::Failed(error) | SessionState::Lost(error) => {
                    self.review.not_sent(&format!("Could not connect: {error}"));
                }
                SessionState::Disconnected => self.review.not_sent("Disconnected."),
                SessionState::Connecting | SessionState::Connected(_) => {}
            },
            _ => {}
        }
        None
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<ObjectDialogAction> {
        let mut open = true;
        let mut action = None;
        Window::new(self.form.title())
            .id(Id::new("object_dialog"))
            .open(&mut open)
            .collapsible(false)
            .default_width(DEFAULT_WIDTH)
            .show(ctx, |ui| action = self.contents(ui));
        if !open && !self.review.is_running() {
            return Some(ObjectDialogAction::Close);
        }
        action
    }

    fn contents(&mut self, ui: &mut Ui) -> Option<ObjectDialogAction> {
        self.form.show(ui, &self.target);
        let destruction = self.form.destruction(self.target.dialect);
        let confirmed = match &destruction {
            Some(destruction) => self.confirmation.show(ui, destruction, &self.target),
            None => true,
        };
        ui.separator();
        let statements = self
            .form
            .operations()
            .and_then(|operations| ddl_statements(&operations, self.target.dialect));
        let action = self.review.show(ui, &self.target, &statements, confirmed)?;
        let statements = statements.ok()?;
        Some(match action {
            ReviewAction::Execute => ObjectDialogAction::Execute {
                data_source: self.target.data_source,
                statements,
            },
            ReviewAction::OpenInConsole => ObjectDialogAction::OpenInConsole {
                data_source: self.target.data_source,
                text: review::script(&statements),
            },
            ReviewAction::Cancel => ObjectDialogAction::Close,
        })
    }
}

/// The schema object at `path`, named as the builder names it: in the path's schema, which
/// every engine's tree has above its tables (MySQL's and ClickHouse's databases are schemas
/// there).
fn object_name(path: &CatalogPath) -> Option<ObjectName> {
    let object = path.last()?;
    if matches!(
        object.kind,
        NodeKind::Database | NodeKind::Schema | NodeKind::Folder(_)
    ) {
        return None;
    }
    Some(ObjectName::new(
        path.name_of(NodeKind::Schema),
        &object.name,
    ))
}

#[cfg(test)]
mod tests;
