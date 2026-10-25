//! The modal dialog for creating or editing a data source, with "Test connection".

use std::path::Path;

use dbdesk_core::{
    DataSource, DataSourceId, DriverKind, ServerInfo, SourceColor, expand, expand_path,
    has_placeholder,
};
use dbdesk_engine::{DatabaseFile, Event, PasswordUpdate, ProcessEnvironment};
use egui::{
    Align, Button, Color32, Grid, Id, Layout, Modal, RichText, Sense, Stroke, TextEdit, Ui,
    WidgetInfo, WidgetType, vec2,
};

use crate::data_source_form::{
    DataSourceForm, DriverFields, DuckDbFields, DuckDbLocation, FormError, ServerFields, TlsChoice,
};
use crate::file_picker::{self, FileEngine};
use crate::icons;
use crate::theme::Palette;
use crate::widgets;

const DIALOG_WIDTH: f32 = 420.0;
const FIELD_WIDTH: f32 = 280.0;
const SWATCH_RADIUS: f32 = 8.0;

pub enum DialogAction {
    Test(DataSource, PasswordUpdate),
    Save(DataSource, PasswordUpdate),
    /// Create an empty database file at the path chosen in the Save dialog.
    CreateDatabaseFile(DatabaseFile),
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
    Creating,
    Created(Result<(), String>),
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
            Event::DatabaseFileCreated { file, result } if self.is_creating(file) => {
                self.status = Status::Created(result.clone());
                false
            }
            _ => false,
        }
    }

    /// Whether `file` is the database this dialog's File field names.
    fn is_creating(&self, file: &DatabaseFile) -> bool {
        let path = match (&self.form.fields, file) {
            (DriverFields::Sqlite(fields), DatabaseFile::Sqlite(_)) => &fields.path,
            (
                DriverFields::DuckDb(DuckDbFields {
                    location: DuckDbLocation::File,
                    path,
                    ..
                }),
                DatabaseFile::DuckDb(_),
            ) => path,
            _ => return false,
        };
        Path::new(path.trim()) == file.path()
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
            let logo_size = ui.text_style_height(&egui::TextStyle::Heading);
            ui.add(icons::driver_logo_sized(
                self.form.driver(),
                palette,
                logo_size,
            ));
            ui.heading(self.title());
        });
        ui.add_space(ui.spacing().item_spacing.y);
        let new_database = self.fields(ui);
        ui.add_space(ui.spacing().item_spacing.y);
        self.status_line(ui);
        ui.separator();
        let action = self.buttons(ui);
        match new_database {
            Some(file) => {
                self.status = Status::Creating;
                Some(DialogAction::CreateDatabaseFile(file))
            }
            None => action,
        }
    }

    fn title(&self) -> String {
        let driver = self.form.driver().display_name();
        if self.is_new {
            format!("New {driver} data source")
        } else {
            format!("Edit {driver} data source")
        }
    }

    /// Draws the form. Returns a database file if the user chose to create one.
    fn fields(&mut self, ui: &mut Ui) -> Option<DatabaseFile> {
        let password_hint = if self.is_new { "none" } else { "unchanged" };
        Grid::new("data_source_fields")
            .num_columns(2)
            .show(ui, |ui| {
                text_row(ui, "Name", &mut self.form.name, "optional");
                ui.label("Colour");
                color_picker(ui, &mut self.form.color);
                ui.end_row();
                match &mut self.form.fields {
                    DriverFields::Postgres(fields) => {
                        server_rows(ui, &mut fields.server, password_hint);
                        None
                    }
                    DriverFields::Mysql(fields) => {
                        server_rows(ui, &mut fields.server, password_hint);
                        None
                    }
                    DriverFields::SqlServer(fields) => {
                        server_rows(ui, &mut fields.server, password_hint);
                        None
                    }
                    DriverFields::Sqlite(fields) => file_rows(
                        ui,
                        FileEngine::Sqlite,
                        &mut fields.path,
                        &mut fields.read_only,
                    ),
                    DriverFields::DuckDb(fields) => duckdb_rows(ui, fields),
                }
            })
            .inner
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
            Status::Creating => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Creating the database file…");
                });
            }
            Status::Created(Ok(())) => {
                widgets::status(
                    ui,
                    icons::CHECK_CIRCLE,
                    palette.success,
                    "Created an empty database. Save to keep this data source.",
                );
            }
            Status::Created(Err(error)) => {
                ui.colored_label(error_color, format!("Could not create the file: {error}"));
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

/// The rows every network engine shares: where the server is, whom to connect as, and how to
/// encrypt. An engine's own rows go before or after them.
fn server_rows(ui: &mut Ui, fields: &mut ServerFields, password_hint: &str) {
    let defaults = fields.defaults;
    let default_port = defaults.port.to_string();
    setting_row(ui, "Host", &mut fields.host, defaults.host, Resolve::Text);
    setting_row(ui, "Port", &mut fields.port, &default_port, Resolve::Text);
    setting_row(
        ui,
        "Database",
        &mut fields.database,
        defaults.database,
        Resolve::Text,
    );
    setting_row(ui, "User", &mut fields.user, defaults.user, Resolve::Text);
    let label = ui.label("Password");
    ui.add(
        TextEdit::singleline(&mut fields.password)
            .password(true)
            .hint_text(password_hint)
            .desired_width(FIELD_WIDTH),
    )
    .labelled_by(label.id);
    ui.end_row();
    tls_rows(ui, fields);
}

/// The TLS mode and, for `verify-full`, the file of authorities to trust.
fn tls_rows(ui: &mut Ui, fields: &mut ServerFields) {
    let label = ui.label("TLS");
    egui::ComboBox::from_id_salt("data_source_tls")
        .selected_text(fields.tls.label())
        .show_ui(ui, |ui| {
            for choice in TlsChoice::ALL {
                ui.selectable_value(&mut fields.tls, choice, choice.label())
                    .on_hover_text(choice.description());
            }
        })
        .response
        .labelled_by(label.id)
        .on_hover_text(fields.tls.description());
    ui.end_row();
    if fields.tls != TlsChoice::VerifyFull {
        return;
    }
    setting_row(
        ui,
        "CA file",
        &mut fields.ca_file,
        "public authorities",
        Resolve::Path,
    );
    ui.label("");
    let palette = Palette::for_visuals(ui.visuals());
    if file_button(ui, icons::FOLDER_OPEN, palette.warning, "Browse…")
        .on_hover_text("Choose a PEM file of authorities to trust")
        .clicked()
        && let Some(path) = file_picker::pick_ca_file(&fields.ca_file)
    {
        fields.ca_file = path.display().to_string();
    }
    ui.end_row();
}

/// The file field, then Browse… (open an existing database) and New… (create one). Returns the
/// database to create, if New… was used; the engine creates it.
fn file_rows(
    ui: &mut Ui,
    engine: FileEngine,
    path: &mut String,
    read_only: &mut bool,
) -> Option<DatabaseFile> {
    let hint = match engine {
        FileEngine::Sqlite => "/path/to/database.db",
        FileEngine::DuckDb => "/path/to/database.duckdb",
    };
    setting_row(ui, "File", path, hint, Resolve::Path);
    ui.label("");
    let new_database = ui
        .horizontal(|ui| {
            let palette = Palette::for_visuals(ui.visuals());
            if file_button(ui, icons::FOLDER_OPEN, palette.warning, "Browse…")
                .on_hover_text("Open an existing database file")
                .clicked()
                && let Some(chosen) = file_picker::pick_database_file(engine, path)
            {
                *path = chosen.display().to_string();
            }
            let new_path = file_button(ui, icons::FILE_PLUS, palette.success, "New…")
                .on_hover_text("Create a new, empty database file")
                .clicked()
                .then(|| file_picker::pick_new_database_file(engine, path))
                .flatten()?;
            *path = new_path.display().to_string();
            // A database you just created is one you mean to write to.
            *read_only = false;
            Some(engine.file(new_path))
        })
        .inner;
    ui.end_row();
    ui.label("");
    ui.checkbox(read_only, "Read-only");
    ui.end_row();
    new_database
}

/// A file, as for SQLite, or a database in memory, for querying CSV, Parquet and JSON files.
fn duckdb_rows(ui: &mut Ui, fields: &mut DuckDbFields) -> Option<DatabaseFile> {
    ui.label("Database");
    ui.horizontal(|ui| {
        ui.radio_value(&mut fields.location, DuckDbLocation::File, "File");
        ui.radio_value(&mut fields.location, DuckDbLocation::InMemory, "In memory");
    });
    ui.end_row();
    match fields.location {
        DuckDbLocation::File => file_rows(
            ui,
            FileEngine::DuckDb,
            &mut fields.path,
            &mut fields.read_only,
        ),
        DuckDbLocation::InMemory => {
            ui.label("");
            ui.weak(
                "Starts empty each time it connects. Query files with read_csv(), \
                 read_parquet() and read_json(), or by naming them: FROM 'orders.csv'.",
            );
            ui.end_row();
            None
        }
    }
}

/// A button with a coloured icon before its text, named by the text alone.
fn file_button(ui: &mut Ui, glyph: &str, icon_color: Color32, text: &str) -> egui::Response {
    let label = widgets::icon_text(ui, glyph, icon_color, text);
    widgets::name(ui.button(label), WidgetType::Button, text, true)
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

/// How a setting's placeholders are resolved when connecting.
#[derive(Clone, Copy)]
enum Resolve {
    Text,
    /// Also turns a leading `~` into the home folder.
    Path,
}

/// A setting's row, then, when it holds a `${NAME}` placeholder, a row saying what that
/// currently resolves to, or why it doesn't: the same rules connecting uses.
fn setting_row(ui: &mut Ui, label: &str, value: &mut String, hint: &str, resolve: Resolve) {
    text_row(ui, label, value, hint);
    let text = value.trim();
    if !has_placeholder(text) {
        return;
    }
    let resolved = match resolve {
        Resolve::Text => expand(text, &ProcessEnvironment),
        Resolve::Path => {
            expand_path(Path::new(text), &ProcessEnvironment).map(|path| path.display().to_string())
        }
    };
    let palette = Palette::for_visuals(ui.visuals());
    ui.label("");
    match resolved {
        Ok(resolved) => ui.label(
            RichText::new(format!("= {resolved}"))
                .small()
                .color(palette.text_weak),
        ),
        Err(error) => ui.label(
            RichText::new(error.to_string())
                .small()
                .color(palette.warning),
        ),
    }
    .on_hover_text("Placeholders are resolved from the environment dbdesk was started in");
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
