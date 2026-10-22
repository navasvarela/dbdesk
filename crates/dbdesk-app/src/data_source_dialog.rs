//! The modal dialog for creating or editing a data source, with "Test connection".

use dbdesk_core::{DataSource, DataSourceId, DriverKind, ServerInfo};
use dbdesk_engine::{Event, PasswordUpdate};
use egui::{Align, Button, Grid, Id, Layout, Modal, TextEdit, Ui};

use crate::data_source_form::{
    DataSourceForm, DriverFields, FormError, PostgresFields, SqliteFields,
};
use crate::theme::SUCCESS_COLOR;

const DIALOG_WIDTH: f32 = 420.0;
const FIELD_WIDTH: f32 = 280.0;

pub enum DialogAction {
    Test(DataSource, PasswordUpdate),
    Save(DataSource, PasswordUpdate),
    Close,
}

pub struct DataSourceDialog {
    id: DataSourceId,
    is_new: bool,
    form: DataSourceForm,
    status: Status,
}

enum Status {
    Editing,
    Invalid(FormError),
    Testing,
    Tested(Result<ServerInfo, String>),
    Saving,
    SaveFailed(String),
}

impl DataSourceDialog {
    pub fn create(driver: DriverKind) -> Self {
        Self {
            id: DataSourceId::generate(),
            is_new: true,
            form: DataSourceForm::new(driver),
            status: Status::Editing,
        }
    }

    pub fn edit(data_source: &DataSource) -> Self {
        Self {
            id: data_source.id,
            is_new: false,
            form: DataSourceForm::from_data_source(data_source),
            status: Status::Editing,
        }
    }

    /// Updates the dialog from an engine event. Returns `true` once its data source has been
    /// saved, meaning the dialog is done.
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::ConnectionTested { id, result } if *id == self.id => {
                self.status = Status::Tested(result.clone());
                false
            }
            Event::DataSourceSaveFailed { id, error } if *id == self.id => {
                self.status = Status::SaveFailed(error.clone());
                false
            }
            Event::DataSourceSaved(data_source) => data_source.id == self.id,
            _ => false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<DialogAction> {
        let modal = Modal::new(Id::new("data_source_dialog")).show(ctx, |ui| self.contents(ui));
        if modal.should_close() {
            return Some(DialogAction::Close);
        }
        modal.inner
    }

    fn contents(&mut self, ui: &mut Ui) -> Option<DialogAction> {
        ui.set_width(DIALOG_WIDTH);
        ui.heading(self.title());
        ui.add_space(ui.spacing().item_spacing.y);
        self.fields(ui);
        ui.add_space(ui.spacing().item_spacing.y);
        self.status_line(ui);
        ui.separator();
        self.buttons(ui)
    }

    fn title(&self) -> String {
        let driver = self.form.driver().display_name();
        if self.is_new {
            format!("New {driver} data source")
        } else {
            format!("Edit {driver} data source")
        }
    }

    fn fields(&mut self, ui: &mut Ui) {
        let uses_password = self.form.driver().uses_password();
        let password_hint = if self.is_new { "none" } else { "unchanged" };
        Grid::new("data_source_fields")
            .num_columns(2)
            .show(ui, |ui| {
                text_row(ui, "Name", &mut self.form.name, "optional");
                match &mut self.form.fields {
                    DriverFields::Postgres(fields) => postgres_rows(ui, fields),
                    DriverFields::Sqlite(fields) => sqlite_rows(ui, fields),
                }
                if uses_password {
                    let label = ui.label("Password");
                    ui.add(
                        TextEdit::singleline(&mut self.form.password)
                            .password(true)
                            .hint_text(password_hint)
                            .desired_width(FIELD_WIDTH),
                    )
                    .labelled_by(label.id);
                    ui.end_row();
                }
            });
    }

    fn status_line(&self, ui: &mut Ui) {
        let error_color = ui.visuals().error_fg_color;
        match &self.status {
            Status::Editing => {}
            Status::Invalid(error) => {
                ui.colored_label(error_color, error.to_string());
            }
            Status::Testing => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Testing connection…");
                });
            }
            Status::Tested(Ok(server)) => {
                ui.colored_label(SUCCESS_COLOR, format!("Connected to {server}."));
            }
            Status::Tested(Err(error)) => {
                ui.colored_label(error_color, error);
            }
            Status::Saving => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Saving…");
                });
            }
            Status::SaveFailed(error) => {
                ui.colored_label(error_color, format!("Could not save: {error}"));
            }
        }
    }

    fn buttons(&mut self, ui: &mut Ui) -> Option<DialogAction> {
        let testing = matches!(self.status, Status::Testing);
        let saving = matches!(self.status, Status::Saving);
        let (test, save, cancel) = ui
            .horizontal(|ui| {
                let test = ui
                    .add_enabled(!testing && !saving, Button::new("Test connection"))
                    .clicked();
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let save = ui.add_enabled(!saving, Button::new("Save")).clicked();
                    let cancel = ui.button("Cancel").clicked();
                    (test, save, cancel)
                })
                .inner
            })
            .inner;

        if cancel {
            Some(DialogAction::Close)
        } else if test {
            self.submit(Status::Testing)
                .map(|(data_source, password)| DialogAction::Test(data_source, password))
        } else if save {
            self.submit(Status::Saving)
                .map(|(data_source, password)| DialogAction::Save(data_source, password))
        } else {
            None
        }
    }

    /// Validates the form and, if it is valid, moves to `next` and returns what to send.
    fn submit(&mut self, next: Status) -> Option<(DataSource, PasswordUpdate)> {
        match self.form.to_data_source(self.id) {
            Ok(data_source) => {
                self.status = next;
                Some((data_source, self.form.password_update()))
            }
            Err(error) => {
                self.status = Status::Invalid(error);
                None
            }
        }
    }
}

fn postgres_rows(ui: &mut Ui, fields: &mut PostgresFields) {
    text_row(ui, "Host", &mut fields.host, "localhost");
    text_row(ui, "Port", &mut fields.port, "5432");
    text_row(ui, "Database", &mut fields.database, "postgres");
    text_row(ui, "User", &mut fields.user, "postgres");
}

fn sqlite_rows(ui: &mut Ui, fields: &mut SqliteFields) {
    text_row(ui, "File", &mut fields.path, "/path/to/database.db");
    ui.label("");
    ui.checkbox(&mut fields.read_only, "Read-only");
    ui.end_row();
}

/// A label and a text field, linked so screen readers (and UI tests) find the field by label.
fn text_row(ui: &mut Ui, label: &str, value: &mut String, hint: &str) {
    let label = ui.label(label);
    ui.add(
        TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(FIELD_WIDTH),
    )
    .labelled_by(label.id);
    ui.end_row();
}
