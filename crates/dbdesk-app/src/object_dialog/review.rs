//! The review panel: the SQL a helper will run, highlighted and read-only, with Execute, Open
//! in console and Copy, and the database's error when it fails.

use dbdesk_core::SafetyLevel;
use dbdesk_engine::DdlFailure;
use dbdesk_sql::DdlError;
use egui::{Align, Id, Layout, Modal, ScrollArea, TextBuffer, TextEdit, Ui};

use super::Target;
use crate::console::highlight::{self, Marks};
use crate::theme::Palette;
use crate::{icons, widgets};

/// The SQL's share of the window: enough for a table of a dozen columns before it scrolls.
const SQL_HEIGHT: f32 = 220.0;

pub(super) enum ReviewAction {
    Execute,
    OpenInConsole,
    Cancel,
}

#[derive(Default)]
pub(super) struct Review {
    state: State,
    /// Execute was clicked on a careful data source, which asks first.
    confirming: bool,
}

#[derive(Default)]
enum State {
    #[default]
    Ready,
    /// Sent; waiting for the engine's answer.
    Running,
    Failed {
        message: String,
        /// Statements that committed before the failure, where DDL isn't transactional.
        applied: usize,
    },
}

/// The statements as one script, each ended with `;`, as the panel shows them and a console
/// receives them.
pub(super) fn script(statements: &[String]) -> String {
    statements
        .iter()
        .map(|statement| format!("{statement};\n"))
        .collect::<Vec<_>>()
        .join("\n")
}

impl Review {
    pub(super) fn is_running(&self) -> bool {
        matches!(self.state, State::Running)
    }

    pub(super) fn failed(&mut self, failure: &DdlFailure) {
        if self.is_running() {
            self.state = State::Failed {
                message: failure.message.clone(),
                applied: failure.applied,
            };
        }
    }

    /// The statements never reached the database, e.g. because it couldn't be connected to.
    pub(super) fn not_sent(&mut self, message: &str) {
        if self.is_running() {
            self.state = State::Failed {
                message: message.to_owned(),
                applied: 0,
            };
        }
    }

    /// `confirmed` is whether the destruction, if any, has been confirmed.
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        target: &Target,
        statements: &Result<Vec<String>, DdlError>,
        confirmed: bool,
    ) -> Option<ReviewAction> {
        let palette = Palette::for_visuals(ui.visuals());
        ui.strong("SQL");
        let script = match statements {
            Ok(statements) => {
                sql(ui, target, &script(statements));
                notes(ui, target, statements.len());
                Some(script(statements))
            }
            Err(error) => {
                ui.colored_label(palette.warning, error.to_string());
                None
            }
        };
        if let State::Failed { message, applied } = &self.state {
            ui.colored_label(palette.error, message);
            if *applied > 0 {
                ui.colored_label(palette.warning, applied_note(*applied));
            }
        }
        ui.separator();
        let action = self.buttons(ui, target, script.as_deref(), confirmed);
        let action = match action {
            Some(ReviewAction::Execute) if target.safety == SafetyLevel::Careful => {
                self.confirming = true;
                None
            }
            action => action,
        };
        let action = action.or_else(|| self.careful_confirmation(ui.ctx(), target));
        if matches!(action, Some(ReviewAction::Execute)) {
            self.state = State::Running;
        }
        action
    }

    fn buttons(
        &self,
        ui: &mut Ui,
        target: &Target,
        script: Option<&str>,
        confirmed: bool,
    ) -> Option<ReviewAction> {
        let running = self.is_running();
        let writable = target.safety != SafetyLevel::ReadOnly;
        let ready = script.is_some() && !running;
        // Inside a row, so the buttons take one line's height rather than all that is left.
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let execute = ui
                    .add_enabled(
                        ready && confirmed && writable,
                        egui::Button::new(widgets::icon_text(
                            ui,
                            icons::PLAY,
                            egui::Color32::PLACEHOLDER,
                            "Execute",
                        )),
                    )
                    .on_disabled_hover_text(execute_disabled_reason(target, confirmed, running));
                let execute = widgets::name(execute, egui::WidgetType::Button, "Execute", ready);
                if running {
                    ui.spinner();
                }
                let console = ui.add_enabled(ready, egui::Button::new("Open in console"));
                let copy = ui.add_enabled(script.is_some(), egui::Button::new("Copy"));
                let cancel = ui.add_enabled(!running, egui::Button::new("Cancel"));
                if copy.clicked()
                    && let Some(script) = script
                {
                    ui.ctx().copy_text(script.to_owned());
                }
                if execute.clicked() {
                    Some(ReviewAction::Execute)
                } else if console.clicked() {
                    Some(ReviewAction::OpenInConsole)
                } else if cancel.clicked() {
                    Some(ReviewAction::Cancel)
                } else {
                    None
                }
            })
            .inner
        })
        .inner
    }

    /// On a careful data source, Execute asks once more, naming the data source.
    fn careful_confirmation(
        &mut self,
        ctx: &egui::Context,
        target: &Target,
    ) -> Option<ReviewAction> {
        if !self.confirming {
            return None;
        }
        let modal = Modal::new(Id::new("object_dialog_careful")).show(ctx, |ui| {
            ui.heading(format!("Run on \u{201c}{}\u{201d}?", target.name));
            ui.label(format!(
                "\u{201c}{}\u{201d} is careful, and this changes the schema.",
                target.name
            ));
            ui.separator();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Run").clicked() {
                    Some(true)
                } else if ui.button("Cancel").clicked() {
                    Some(false)
                } else {
                    None
                }
            })
            .inner
        });
        let choice = if modal.should_close() {
            Some(false)
        } else {
            modal.inner
        };
        match choice {
            Some(run) => {
                self.confirming = false;
                run.then_some(ReviewAction::Execute)
            }
            None => None,
        }
    }
}

fn execute_disabled_reason(target: &Target, confirmed: bool, running: bool) -> String {
    if target.safety == SafetyLevel::ReadOnly {
        format!(
            "\u{201c}{}\u{201d} is read-only: open the SQL in a console or copy it instead",
            target.name
        )
    } else if running {
        "Running…".to_owned()
    } else if !confirmed {
        "Confirm above first".to_owned()
    } else {
        "Fill in the form first".to_owned()
    }
}

/// The SQL, highlighted as the console highlights it, selectable but not editable.
fn sql(ui: &mut Ui, target: &Target, text: &str) {
    let dialect = target.dialect;
    let marks = Marks::default();
    let mut layouter = |ui: &Ui, text: &dyn TextBuffer, wrap_width: f32| {
        let mut job = highlight::layout_job(text.as_str(), dialect, &marks, ui.style());
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    ScrollArea::vertical()
        .id_salt("object_dialog_sql")
        .max_height(SQL_HEIGHT)
        .show(ui, |ui| {
            // A `&str` buffer makes the editor read-only but still selectable.
            ui.add(
                TextEdit::multiline(&mut &*text)
                    .id_salt("object_dialog_sql_text")
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .layouter(&mut layouter),
            );
        });
}

/// What the user should know before Execute: that a read-only data source refuses it, and
/// where DDL isn't transactional, that a failure part way keeps what ran before it.
fn notes(ui: &mut Ui, target: &Target, statements: usize) {
    let palette = Palette::for_visuals(ui.visuals());
    if target.safety == SafetyLevel::ReadOnly {
        ui.horizontal_wrapped(|ui| {
            ui.label(widgets::glyph(icons::LOCK_SIMPLE).color(palette.warning));
            ui.label(format!(
                "\u{201c}{}\u{201d} is read-only, so this can't run here.",
                target.name
            ));
        });
    }
    if statements > 1 && !target.dialect.capabilities().transactional_ddl {
        ui.weak(format!(
            "{} commits each statement as it runs: if one fails, the ones before it stay \
             applied.",
            target.dialect.display_name()
        ));
    }
}

fn applied_note(applied: usize) -> String {
    match applied {
        1 => "The first statement ran and stays applied.".to_owned(),
        n => format!("The first {n} statements ran and stay applied."),
    }
}
