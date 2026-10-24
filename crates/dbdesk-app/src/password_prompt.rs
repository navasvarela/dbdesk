//! Asks for a data source's password when connecting finds none: nothing is saved, or the
//! keychain can't be read. Opened by [`Event::PasswordNeeded`].

use dbdesk_core::{DataSource, DataSourceId, Password};
use dbdesk_engine::{Event, PasswordKept, PasswordNeeded};
use egui::{Align, Button, Id, Layout, Modal, RichText, TextEdit};

use crate::theme::Palette;

const FIELD_WIDTH: f32 = 260.0;

pub enum PromptAction {
    /// Give the engine the password, then connect again once it has it.
    Provide {
        password: Password,
        save: bool,
    },
    Cancel,
}

pub struct PasswordPrompt {
    data_source: DataSourceId,
    name: String,
    /// Where the connection goes, so the user knows which password is meant.
    target: String,
    reason: PasswordNeeded,
    password: String,
    /// Ticked to start with: a password typed once should not be asked for again.
    save: bool,
    submitted: bool,
    focus_requested: bool,
}

impl PasswordPrompt {
    pub fn new(data_source: &DataSource, reason: PasswordNeeded) -> Self {
        Self {
            data_source: data_source.id,
            name: data_source.name.clone(),
            target: data_source.config.to_string(),
            reason,
            password: String::new(),
            save: true,
            submitted: false,
            focus_requested: false,
        }
    }

    pub fn data_source(&self) -> DataSourceId {
        self.data_source
    }

    /// Whether `event` answers this prompt's password, meaning the prompt is done. A password
    /// the keychain refused is still kept for the session, so that isn't a reason to stay.
    pub fn apply(&self, event: &Event) -> Option<PasswordKept> {
        match event {
            Event::PasswordProvided { data_source, kept } if *data_source == self.data_source => {
                Some(kept.clone())
            }
            _ => None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<PromptAction> {
        let modal = Modal::new(Id::new("password_prompt")).show(ctx, |ui| self.contents(ui));
        if modal.should_close() {
            return Some(PromptAction::Cancel);
        }
        modal.inner
    }

    fn contents(&mut self, ui: &mut egui::Ui) -> Option<PromptAction> {
        let palette = Palette::for_visuals(ui.visuals());
        ui.heading(format!("Connect to {}", self.name));
        ui.label(RichText::new(&self.target).color(palette.text_weak));
        ui.label(format!("Enter the password: {}.", self.reason));
        ui.add_space(ui.spacing().item_spacing.y);
        let entered = ui
            .horizontal(|ui| {
                let label = ui.label("Password");
                let field = ui
                    .add(
                        TextEdit::singleline(&mut self.password)
                            .password(true)
                            .desired_width(FIELD_WIDTH),
                    )
                    .labelled_by(label.id);
                if !std::mem::replace(&mut self.focus_requested, true) {
                    field.request_focus();
                }
                field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
            })
            .inner;
        ui.checkbox(&mut self.save, "Save to keychain")
            .on_hover_text("Otherwise the password is kept only until dbdesk quits");
        ui.separator();
        let (connect, cancel) = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                let connect = Button::new(RichText::new("Connect").color(palette.on_primary))
                    .fill(palette.primary);
                let connect = ui.add_enabled(!self.submitted, connect).clicked();
                (connect, ui.button("Cancel").clicked())
            })
            .inner;
        if cancel {
            return Some(PromptAction::Cancel);
        }
        if (connect || entered) && !self.submitted {
            self.submitted = true;
            return Some(PromptAction::Provide {
                password: Password::new(self.password.clone()),
                save: self.save,
            });
        }
        None
    }
}
