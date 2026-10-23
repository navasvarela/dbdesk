use egui::{Align, Id, Layout, Modal, OpenUrl, RichText};

use crate::theme::Palette;
use crate::updates::UpdateStatus;
use crate::version;

/// Help › Check for updates, and the status bar's notice: whether a newer release is out,
/// and a way to its page. Returns `true` once the user closes it.
pub fn show(ctx: &egui::Context, status: &UpdateStatus) -> bool {
    let modal = Modal::new(Id::new("check_for_updates")).show(ctx, |ui| {
        let palette = Palette::for_visuals(ui.visuals());
        ui.set_min_width(320.0);
        ui.heading("Updates");
        ui.add_space(ui.spacing().item_spacing.y);
        let mut page = None;
        match status {
            UpdateStatus::NotChecked | UpdateStatus::Checking => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Checking for a newer release…");
                });
            }
            UpdateStatus::UpToDate { latest } => {
                ui.label(format!("You're up to date ({}).", version::VERSION));
                ui.label(
                    RichText::new(format!("The newest release is {latest}."))
                        .color(palette.text_weak),
                );
            }
            UpdateStatus::Available { version, page: url } => {
                ui.label(format!(
                    "dbdesk {version} is available (you have {}).",
                    version::VERSION
                ));
                page = Some(url);
            }
            UpdateStatus::Failed(error) => {
                ui.label("Couldn't check for updates.");
                ui.colored_label(palette.error, error);
            }
        }
        ui.separator();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(url) = page
                && ui.button("Open release page").clicked()
            {
                ui.ctx().open_url(OpenUrl::new_tab(url));
            }
            ui.button("Close").clicked()
        })
        .inner
    });
    modal.should_close() || modal.inner
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use semver::Version;

    use super::*;

    #[test]
    fn offers_the_release_page_when_an_update_is_out_and_closes() {
        let status = UpdateStatus::Available {
            version: Version::new(9, 0, 0),
            page: "https://example.com/v9.0.0".to_owned(),
        };
        let mut closed = false;
        let mut harness = Harness::new_ui(|ui| closed |= show(ui.ctx(), &status));
        harness.run();

        harness.get_by_label(&format!(
            "dbdesk 9.0.0 is available (you have {}).",
            version::VERSION
        ));
        harness.get_by_label("Open release page");
        harness.get_by_label("Close").click();
        harness.run();
        drop(harness);

        assert!(closed);
    }
}
