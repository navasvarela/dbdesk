use dbdesk_engine::InstallSupport;
use egui::{Align, Id, Layout, Modal, OpenUrl, ProgressBar, RichText};

use crate::theme::Palette;
use crate::updates::{InstallState, UpdateStatus, Updates};
use crate::version;

/// What the user asked of the updates dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    Close,
    Install,
    Restart,
}

/// Help › Check for updates, and the status bar's notice: whether a newer release is out,
/// installing it when this copy can replace itself, and its release page.
pub fn show(ctx: &egui::Context, updates: &Updates) -> Option<UpdateAction> {
    let modal = Modal::new(Id::new("check_for_updates")).show(ctx, |ui| {
        let palette = &Palette::for_visuals(ui.visuals());
        ui.set_min_width(360.0);
        ui.heading("Updates");
        ui.add_space(ui.spacing().item_spacing.y);
        let mut page = None;
        match updates.status() {
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
                page = Some(url);
                describe_install(ui, palette, version, updates);
            }
            UpdateStatus::Failed(error) => {
                ui.label("Couldn't check for updates.");
                ui.colored_label(palette.error, error);
            }
        }
        ui.separator();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut action = None;
            if ui.button("Close").clicked() {
                action = Some(UpdateAction::Close);
            }
            if let Some(url) = page
                && ui.button("Open release page").clicked()
            {
                ui.ctx().open_url(OpenUrl::new_tab(url));
            }
            if let Some(button) = primary_button(updates)
                && ui.button(button.label()).clicked()
            {
                action = Some(button.action());
            }
            action
        })
        .inner
    });
    if modal.should_close() {
        Some(UpdateAction::Close)
    } else {
        modal.inner
    }
}

/// What the dialog says about an available update, depending on how installing it is going.
fn describe_install(
    ui: &mut egui::Ui,
    palette: &Palette,
    available: &semver::Version,
    updates: &Updates,
) {
    match updates.install_state() {
        InstallState::Idle => {
            ui.label(format!(
                "dbdesk {available} is available (you have {}).",
                version::VERSION
            ));
            if let InstallSupport::Unsupported(reason) = updates.support() {
                ui.label(RichText::new(reason).color(palette.text_weak));
            }
        }
        state @ InstallState::Downloading { downloaded, .. } => {
            ui.label(format!("Downloading dbdesk {available}…"));
            let bar = match state.fraction() {
                Some(fraction) => ProgressBar::new(fraction).show_percentage(),
                None => ProgressBar::new(0.0)
                    .animate(true)
                    .text(format!("{:.1} MB", *downloaded as f64 / 1_000_000.0)),
            };
            ui.add(bar);
        }
        InstallState::Installed { version } => {
            ui.label(format!("dbdesk {version} is installed."));
            ui.label(RichText::new("Restart dbdesk to start using it.").color(palette.text_weak));
        }
        InstallState::Failed(error) => {
            ui.label("Couldn't install the update.");
            ui.colored_label(palette.error, error);
        }
    }
}

/// The dialog's main button, when there is a next step to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrimaryButton {
    Install,
    TryAgain,
    Restart,
}

impl PrimaryButton {
    fn label(self) -> &'static str {
        match self {
            Self::Install => "Install and restart",
            Self::TryAgain => "Try again",
            Self::Restart => "Restart now",
        }
    }

    fn action(self) -> UpdateAction {
        match self {
            Self::Install | Self::TryAgain => UpdateAction::Install,
            Self::Restart => UpdateAction::Restart,
        }
    }
}

fn primary_button(updates: &Updates) -> Option<PrimaryButton> {
    let installable = matches!(updates.status(), UpdateStatus::Available { .. })
        && *updates.support() == InstallSupport::Supported;
    match updates.install_state() {
        InstallState::Idle if installable => Some(PrimaryButton::Install),
        InstallState::Failed(_) if installable => Some(PrimaryButton::TryAgain),
        InstallState::Installed { .. } => Some(PrimaryButton::Restart),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use dbdesk_engine::{Event, Release};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn with_update(support: InstallSupport) -> Updates {
        let mut updates = Updates::new(support);
        updates.apply(&Event::LatestRelease(Ok(Release {
            tag: "v9.0.0".to_owned(),
            page: "https://example.com/v9.0.0".to_owned(),
        })));
        updates
    }

    #[test]
    fn offers_to_install_where_it_can_and_the_release_page_everywhere() {
        let updates = with_update(InstallSupport::Supported);
        let mut action = None;
        let mut harness = Harness::new_ui(|ui| action = action.or(show(ui.ctx(), &updates)));
        harness.run();
        harness.get_by_label(&format!(
            "dbdesk 9.0.0 is available (you have {}).",
            version::VERSION
        ));
        harness.get_by_label("Open release page");
        harness.get_by_label("Install and restart").click();
        harness.run();
        drop(harness);
        assert_eq!(action, Some(UpdateAction::Install));

        let updates = with_update(InstallSupport::Unsupported("Only the AppImage".to_owned()));
        let mut harness = Harness::new_ui(|ui| {
            show(ui.ctx(), &updates);
        });
        harness.run();
        harness.get_by_label("Only the AppImage");
        assert!(harness.query_by_label("Install and restart").is_none());
        harness.get_by_label("Open release page");
    }

    #[test]
    fn shows_progress_then_offers_a_restart() {
        let mut updates = with_update(InstallSupport::Supported);
        let _ = updates.install();
        updates.apply(&Event::UpdateProgress {
            downloaded: 50,
            total: Some(100),
        });
        let mut harness = Harness::new_ui(|ui| {
            show(ui.ctx(), &updates);
        });
        harness.run_steps(2);
        harness.get_by_label("Downloading dbdesk 9.0.0…");
        assert!(harness.query_by_label("Install and restart").is_none());
        drop(harness);

        updates.apply(&Event::UpdateInstalled(Ok("9.0.0".to_owned())));
        let mut action = None;
        let mut harness = Harness::new_ui(|ui| action = action.or(show(ui.ctx(), &updates)));
        harness.run();
        harness.get_by_label("dbdesk 9.0.0 is installed.");
        harness.get_by_label("Restart now").click();
        harness.run();
        drop(harness);
        assert_eq!(action, Some(UpdateAction::Restart));
    }
}
