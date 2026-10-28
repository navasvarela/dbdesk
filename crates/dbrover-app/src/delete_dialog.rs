use dbrover_core::{DataSource, DataSourceId};
use egui::{Align, Id, Layout, Modal};

pub enum DeleteChoice {
    Confirmed(DataSourceId),
    Cancelled,
}

/// Asks before deleting a data source, since that also removes its saved password.
pub struct DeleteConfirmation {
    id: DataSourceId,
    name: String,
}

impl DeleteConfirmation {
    pub fn new(data_source: &DataSource) -> Self {
        Self {
            id: data_source.id,
            name: data_source.name.clone(),
        }
    }

    pub fn show(&self, ctx: &egui::Context) -> Option<DeleteChoice> {
        let modal = Modal::new(Id::new("delete_data_source")).show(ctx, |ui| {
            ui.heading("Delete data source");
            ui.label(format!(
                "Delete \"{}\"? Its saved password is removed from the keychain too.",
                self.name
            ));
            ui.separator();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    Some(DeleteChoice::Confirmed(self.id))
                } else if ui.button("Cancel").clicked() {
                    Some(DeleteChoice::Cancelled)
                } else {
                    None
                }
            })
            .inner
        });
        if modal.should_close() {
            return Some(DeleteChoice::Cancelled);
        }
        modal.inner
    }
}
