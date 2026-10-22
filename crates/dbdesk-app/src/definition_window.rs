use dbdesk_core::{CatalogPath, DataSourceId};
use dbdesk_engine::Event;
use egui::{ScrollArea, TextEdit, Window};

const DEFAULT_SIZE: [f32; 2] = [560.0, 380.0];

/// Shows the source of one catalog object (a view, function, index, …) in a floating window.
///
/// Read-only for now; once query consoles exist (M3) this could open the source in one.
pub struct DefinitionWindow {
    id: DataSourceId,
    path: CatalogPath,
    title: String,
    content: Content,
}

enum Content {
    Loading,
    Loaded(String),
    Failed(String),
}

impl DefinitionWindow {
    pub fn new(id: DataSourceId, path: CatalogPath, title: String) -> Self {
        Self {
            id,
            path,
            title,
            content: Content::Loading,
        }
    }

    pub fn apply(&mut self, event: &Event) {
        if let Event::DefinitionLoaded { id, path, result } = event
            && *id == self.id
            && *path == self.path
        {
            self.content = match result {
                Ok(text) => Content::Loaded(text.clone()),
                Err(error) => Content::Failed(error.clone()),
            };
        }
    }

    /// Draws the window. Returns `false` once the user has closed it.
    pub fn show(&self, ctx: &egui::Context) -> bool {
        let mut open = true;
        Window::new(format!("Definition of {}", self.title))
            .id(egui::Id::new("definition_window"))
            .open(&mut open)
            .default_size(DEFAULT_SIZE)
            .show(ctx, |ui| match &self.content {
                Content::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading…");
                    });
                }
                Content::Failed(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                Content::Loaded(text) => {
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                    ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                        // A `&str` buffer makes the editor read-only but still selectable.
                        ui.add(
                            TextEdit::multiline(&mut text.as_str())
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    });
                }
            });
        open
    }
}
