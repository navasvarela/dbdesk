//! The modal dialog for creating or editing a data source, with "Test connection".

use dbdesk_core::{DataSource, DataSourceId, DriverKind, ServerInfo, SourceColor};
use dbdesk_engine::{Event, PasswordUpdate};
use egui::{
    Align, Button, Color32, Grid, Id, Layout, Modal, RichText, Sense, Stroke, TextEdit, Ui,
    WidgetInfo, WidgetType, vec2,
};

use crate::data_source_form::{
    DataSourceForm, DriverFields, FormError, PostgresFields, SqliteFields,
};
use crate::icons;
use crate::theme::Palette;
use crate::widgets;

const DIALOG_WIDTH: f32 = 420.0;
const FIELD_WIDTH: f32 = 280.0;
const SWATCH_RADIUS: f32 = 8.0;

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
        let palette = Palette::for_visuals(ui.visuals());
        ui.horizontal(|ui| {
            ui.heading(icons::driver(self.form.driver()).rich_text(palette));
            ui.heading(self.title());
        });
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
                ui.label("Colour");
                color_picker(ui, &mut self.form.color);
                ui.end_row();
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
        let palette = Palette::for_visuals(ui.visuals());
        let error_color = palette.error;
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
                widgets::status(
                    ui,
                    icons::CHECK_CIRCLE,
                    palette.success,
                    &format!("Connected to {server}."),
                );
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
                    let palette = Palette::for_visuals(ui.visuals());
                    let save_button = Button::new(RichText::new("Save").color(palette.on_primary))
                        .fill(palette.primary);
                    let save = ui.add_enabled(!saving, save_button).clicked();
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

/// "None" and one swatch per [`SourceColor`], like radio buttons.
fn color_picker(ui: &mut Ui, selected: &mut Option<SourceColor>) {
    let palette = Palette::for_visuals(ui.visuals());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let choices = std::iter::once(None).chain(SourceColor::ALL.map(Some));
        for choice in choices {
            if swatch(ui, choice, *selected == choice, palette).clicked() {
                *selected = choice;
            }
        }
    });
}

/// A filled circle in `color`, or a struck-out empty one for "no colour". The selected one
/// gets a ring.
fn swatch(
    ui: &mut Ui,
    color: Option<SourceColor>,
    selected: bool,
    palette: &Palette,
) -> egui::Response {
    let size = vec2(SWATCH_RADIUS * 2.0 + 6.0, SWATCH_RADIUS * 2.0 + 6.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let name = color.map_or("No colour", SourceColor::display_name);
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, selected, name));
    let center = rect.center();
    let painter = ui.painter();
    match color {
        Some(color) => {
            painter.circle_filled(center, SWATCH_RADIUS, palette.source(color));
        }
        None => {
            painter.circle_stroke(
                center,
                SWATCH_RADIUS - 0.5,
                Stroke::new(1.0, palette.text_weak),
            );
            let offset = vec2(SWATCH_RADIUS, -SWATCH_RADIUS) * std::f32::consts::FRAC_1_SQRT_2;
            painter.line_segment(
                [center - offset, center + offset],
                Stroke::new(1.0, palette.text_weak),
            );
        }
    }
    let ring = if selected {
        Stroke::new(2.0, palette.text)
    } else if response.hovered() {
        Stroke::new(1.0, palette.text_weak)
    } else {
        Stroke::new(0.0, Color32::TRANSPARENT)
    };
    painter.circle_stroke(center, SWATCH_RADIUS + 2.5, ring);
    response.on_hover_text(name)
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
