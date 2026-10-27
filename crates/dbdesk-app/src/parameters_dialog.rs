//! Asks for the values of a run's `:name` parameters before it runs, starting from what the
//! console ran them with last.

use dbdesk_core::Cell;
use dbdesk_sql::ParameterValues;
use dbdesk_store::ConsoleKey;
use egui::{Align, Id, Key, Layout, Modal, TextEdit};

use crate::console::SubmittedRun;

pub enum ParametersChoice {
    Run,
    Cancel,
}

pub struct ParametersDialog {
    pub console: ConsoleKey,
    run: SubmittedRun,
    fields: Vec<Field>,
    /// Set until the first field has been given the keyboard focus.
    focus_first: bool,
}

/// One parameter's value as being typed.
struct Field {
    name: String,
    text: String,
    null: bool,
}

impl ParametersDialog {
    /// Asks for `names`, in order, starting from the values `remembered`.
    pub fn new(
        console: ConsoleKey,
        run: SubmittedRun,
        names: Vec<String>,
        remembered: &ParameterValues,
    ) -> Self {
        let fields = names
            .into_iter()
            .map(|name| {
                let (text, null) = match remembered.get(&name) {
                    Some(Some(text)) => (text.clone(), false),
                    Some(None) => (String::new(), true),
                    None => (String::new(), false),
                };
                Field { name, text, null }
            })
            .collect();
        Self {
            console,
            run,
            fields,
            focus_first: true,
        }
    }

    /// The values as typed: an empty field is an empty string, not `NULL`.
    pub fn values(&self) -> ParameterValues {
        self.fields
            .iter()
            .map(|field| {
                let value: Cell = (!field.null).then(|| field.text.clone());
                (field.name.clone(), value)
            })
            .collect()
    }

    /// The run, with the values typed.
    pub fn into_run(self) -> (ConsoleKey, SubmittedRun) {
        let values = self.values();
        let mut run = self.run;
        run.parameters = values;
        (self.console, run)
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<ParametersChoice> {
        let modal = Modal::new(Id::new("parameters_dialog")).show(ctx, |ui| {
            ui.heading("Parameter values");
            ui.label("Each value is sent as text, which the database converts as it needs.");
            ui.add_space(4.0);
            egui::Grid::new("parameter_fields")
                .num_columns(3)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    for (index, field) in self.fields.iter_mut().enumerate() {
                        let label = ui.label(format!(":{}", field.name));
                        let edit = ui
                            .add_enabled(
                                !field.null,
                                TextEdit::singleline(&mut field.text)
                                    .hint_text(if field.null { "NULL" } else { "" })
                                    .desired_width(260.0),
                            )
                            .labelled_by(label.id);
                        if index == 0 && std::mem::take(&mut self.focus_first) {
                            edit.request_focus();
                        }
                        ui.checkbox(&mut field.null, "NULL");
                        ui.end_row();
                    }
                });
            ui.separator();
            let enter = ui.input(|input| input.key_pressed(Key::Enter));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Run").clicked() || enter {
                    Some(ParametersChoice::Run)
                } else if ui.button("Cancel").clicked() {
                    Some(ParametersChoice::Cancel)
                } else {
                    None
                }
            })
            .inner
        });
        if modal.should_close() {
            return Some(ParametersChoice::Cancel);
        }
        modal.inner
    }
}

#[cfg(test)]
mod tests {
    use dbdesk_core::DataSourceId;
    use dbdesk_engine::RunPurpose;
    use dbdesk_store::ConsoleName;

    use super::*;

    #[test]
    fn starts_from_the_values_remembered_and_tells_null_from_empty() {
        let remembered: ParameterValues = [
            ("since", Some("2026-01-01".to_owned())),
            ("note", None),
            ("unused", Some("x".to_owned())),
        ]
        .into_iter()
        .collect();
        let key = ConsoleKey {
            data_source: DataSourceId::generate(),
            name: ConsoleName::numbered(1),
        };
        let run = SubmittedRun {
            statements: Vec::new(),
            purpose: RunPurpose::Run,
            parameters: ParameterValues::default(),
        };
        let names = ["since", "note", "new"].map(str::to_owned).to_vec();

        let dialog = ParametersDialog::new(key.clone(), run, names, &remembered);

        let expected: ParameterValues = [
            ("since", Some("2026-01-01".to_owned())),
            ("note", None),
            ("new", Some(String::new())),
        ]
        .into_iter()
        .collect();
        assert_eq!(dialog.values(), expected);
        let (console, run) = dialog.into_run();
        assert_eq!(console, key);
        assert_eq!(run.parameters, expected);
    }
}
