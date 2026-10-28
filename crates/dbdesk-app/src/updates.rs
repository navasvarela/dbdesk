//! Whether a newer dbdesk has been released, and installing it: what Help › Check for updates
//! shows, and the notice in the status bar.

use dbdesk_engine::{Command, Event, InstallSupport, InstalledUpdate, Release, Restart};
use dbdesk_store::Settings;
use semver::Version;

use crate::version;

/// The GitHub repository whose releases are dbdesk's, as `owner/name`. The same repository
/// as `repository` in Cargo.toml; a test keeps them in step.
pub const RELEASES_REPOSITORY: &str = "navasvarela/dbdesk";

/// The update manifest the installer reads: the `latest.json` the release workflow attaches to
/// every release. GitHub redirects `latest/download` to the newest published release, so
/// drafts are never offered.
pub const UPDATE_MANIFEST_URL: &str =
    "https://github.com/navasvarela/dbdesk/releases/latest/download/latest.json";

/// The public half of the key the release workflow signs update packages with. The private
/// half is the `CARGO_PACKAGER_SIGN_PRIVATE_KEY` repository secret. A package whose signature
/// doesn't match is refused before anything is replaced. Changing it means every copy older
/// than the change must be updated by hand once (see docs/releasing.md).
pub const UPDATE_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDMwNUI0ODk5QkUxNEEyRjQKUldUMG9oUyttVWhiTUtlVmM0aVBkRSt3VVpzQWxOTTRKQ1RkK3FGcTZBSEduTUx3TjUzemdBVEcK";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    NotChecked,
    Checking,
    UpToDate {
        latest: Version,
    },
    Available {
        version: Version,
        /// The release's page, with its notes and downloads.
        page: String,
    },
    Failed(String),
}

/// Where installing an available update has got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallState {
    Idle,
    Downloading {
        downloaded: u64,
        total: Option<u64>,
    },
    /// Installed over this copy, or its installer kept for when dbdesk quits.
    Installed(InstalledUpdate),
    Failed(String),
}

impl InstallState {
    /// How far the download has got, from 0 to 1, when its size is known.
    pub fn fraction(&self) -> Option<f32> {
        match *self {
            Self::Downloading {
                downloaded,
                total: Some(total),
            } if total > 0 => Some((downloaded as f64 / total as f64).min(1.0) as f32),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct Updates {
    status: UpdateStatus,
    /// Set once the check at start-up has been decided on, so a later settings reload
    /// doesn't check again.
    start_decided: bool,
    /// Whether this copy can replace itself; decided once at start-up from how it was
    /// installed.
    support: InstallSupport,
    install: InstallState,
    /// Set when the user chose to restart into an installed update, until the app quits or
    /// the quit is cancelled.
    restart_after_quit: bool,
    pub dialog_open: bool,
}

impl Default for Updates {
    fn default() -> Self {
        Self::new(InstallSupport::Unsupported(String::new()))
    }
}

impl Updates {
    pub fn new(support: InstallSupport) -> Self {
        Self {
            status: UpdateStatus::NotChecked,
            start_decided: false,
            support,
            install: InstallState::Idle,
            restart_after_quit: false,
            dialog_open: false,
        }
    }

    pub fn status(&self) -> &UpdateStatus {
        &self.status
    }

    pub fn support(&self) -> &InstallSupport {
        &self.support
    }

    pub fn install_state(&self) -> &InstallState {
        &self.install
    }

    /// The status bar's link to the dialog, while there is something to say.
    pub fn notice(&self) -> Option<String> {
        let UpdateStatus::Available { version, .. } = &self.status else {
            return None;
        };
        Some(match &self.install {
            InstallState::Idle | InstallState::Failed(_) => {
                format!("dbdesk {version} is available")
            }
            state @ InstallState::Downloading { .. } => match state.fraction() {
                Some(fraction) => format!("Downloading dbdesk {version}… {:.0}%", fraction * 100.0),
                None => format!("Downloading dbdesk {version}…"),
            },
            InstallState::Installed(_) => "Restart to finish updating".to_owned(),
        })
    }

    /// Install and restart: downloads and installs the available update, once. `None` when
    /// there is nothing to install, this copy can't replace itself, or it is already under way.
    pub fn install(&mut self) -> Option<Command> {
        let can_start = matches!(self.status, UpdateStatus::Available { .. })
            && self.support == InstallSupport::Supported
            && matches!(self.install, InstallState::Idle | InstallState::Failed(_));
        if !can_start {
            return None;
        }
        self.install = InstallState::Downloading {
            downloaded: 0,
            total: None,
        };
        Some(Command::InstallUpdate)
    }

    /// Restart now: true when an installed update is waiting for the app to start again. The
    /// app then quits through its usual checks for open transactions.
    pub fn request_restart(&mut self) -> bool {
        self.restart_after_quit = matches!(self.install, InstallState::Installed(_));
        self.restart_after_quit
    }

    /// The quit was cancelled, so the restart is too: quitting later shouldn't start dbdesk
    /// again unasked.
    pub fn cancel_restart(&mut self) {
        self.restart_after_quit = false;
    }

    /// What to do once the app has quit, when the user chose to restart into an update.
    pub fn restart_after_quit(&self) -> Option<&Restart> {
        match &self.install {
            InstallState::Installed(update) if self.restart_after_quit => Some(&update.restart),
            _ => None,
        }
    }

    /// The check at start-up, once, when the settings allow it. Its outcome only shows if
    /// there is something new: a failed automatic check is logged by the engine and
    /// otherwise left alone.
    pub fn start(&mut self, settings: &Settings) -> Option<Command> {
        if std::mem::replace(&mut self.start_decided, true) || !settings.updates.check_on_start {
            return None;
        }
        self.status = UpdateStatus::Checking;
        Some(Command::CheckForUpdates)
    }

    /// Help › Check for updates: checks again and opens the dialog to show the outcome.
    pub fn check_now(&mut self) -> Command {
        self.status = UpdateStatus::Checking;
        self.dialog_open = true;
        Command::CheckForUpdates
    }

    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::LatestRelease(result) => self.status = status_for(result, version::VERSION),
            Event::UpdateProgress { downloaded, total } => {
                self.install = InstallState::Downloading {
                    downloaded: *downloaded,
                    total: *total,
                };
            }
            Event::UpdateInstalled(Ok(update)) => {
                self.install = InstallState::Installed(update.clone());
            }
            Event::UpdateInstalled(Err(error)) => {
                self.install = InstallState::Failed(error.clone())
            }
            _ => {}
        }
    }
}

/// What the newest release means for the running `current` version.
fn status_for(latest: &Result<Release, String>, current: &str) -> UpdateStatus {
    let release = match latest {
        Ok(release) => release,
        Err(error) => return UpdateStatus::Failed(error.clone()),
    };
    let Ok(current) = Version::parse(current) else {
        return UpdateStatus::Failed(format!("this build's version, {current}, isn't a version"));
    };
    // Tags are `v0.2.0`; the release workflow refuses any other shape (docs/releasing.md).
    match Version::parse(release.tag.trim_start_matches('v')) {
        Ok(version) if version > current => UpdateStatus::Available {
            version,
            page: release.page.clone(),
        },
        Ok(latest) => UpdateStatus::UpToDate { latest },
        Err(_) => UpdateStatus::Failed(format!(
            "the newest release's tag, {}, isn't a version",
            release.tag
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> Result<Release, String> {
        Ok(Release {
            tag: tag.to_owned(),
            page: format!("https://example.com/{tag}"),
        })
    }

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn a_higher_version_is_an_update_and_anything_else_is_not() {
        let cases = [
            (
                release("v0.2.0"),
                "0.1.0",
                UpdateStatus::Available {
                    version: version("0.2.0"),
                    page: "https://example.com/v0.2.0".to_owned(),
                },
            ),
            (
                release("0.1.1"),
                "0.1.0",
                UpdateStatus::Available {
                    version: version("0.1.1"),
                    page: "https://example.com/0.1.1".to_owned(),
                },
            ),
            // Numeric, not textual: 0.10 is after 0.9.
            (
                release("v0.10.0"),
                "0.9.0",
                UpdateStatus::Available {
                    version: version("0.10.0"),
                    page: "https://example.com/v0.10.0".to_owned(),
                },
            ),
            (
                release("v0.1.0"),
                "0.1.0",
                UpdateStatus::UpToDate {
                    latest: version("0.1.0"),
                },
            ),
            // A build from main, newer than the last release.
            (
                release("v0.1.0"),
                "0.2.0",
                UpdateStatus::UpToDate {
                    latest: version("0.1.0"),
                },
            ),
            (
                release("nightly"),
                "0.1.0",
                UpdateStatus::Failed(
                    "the newest release's tag, nightly, isn't a version".to_owned(),
                ),
            ),
            (
                Err("could not reach GitHub".to_owned()),
                "0.1.0",
                UpdateStatus::Failed("could not reach GitHub".to_owned()),
            ),
        ];
        for (latest, current, expected) in cases {
            assert_eq!(
                status_for(&latest, current),
                expected,
                "{latest:?} vs {current}"
            );
        }
    }

    #[test]
    fn checks_once_at_start_unless_switched_off() {
        let mut updates = Updates::default();
        let settings = Settings::default();
        assert!(matches!(
            updates.start(&settings),
            Some(Command::CheckForUpdates)
        ));
        assert_eq!(updates.status(), &UpdateStatus::Checking);
        assert!(
            updates.start(&settings).is_none(),
            "a settings reload checked again"
        );

        let mut off = Settings::default();
        off.updates.check_on_start = false;
        let mut updates = Updates::default();
        assert!(updates.start(&off).is_none());
        assert_eq!(updates.status(), &UpdateStatus::NotChecked);
        assert!(updates.start(&Settings::default()).is_none());
    }

    #[test]
    fn checking_now_opens_the_dialog_and_the_answer_replaces_the_status() {
        let mut updates = Updates::default();
        assert!(matches!(updates.check_now(), Command::CheckForUpdates));
        assert!(updates.dialog_open);
        assert_eq!(updates.status(), &UpdateStatus::Checking);

        updates.apply(&Event::LatestRelease(Err("offline".to_owned())));
        assert_eq!(
            updates.status(),
            &UpdateStatus::Failed("offline".to_owned())
        );
    }

    fn available() -> Updates {
        let mut updates = Updates::new(InstallSupport::Supported);
        updates.apply(&Event::LatestRelease(release("v99.0.0")));
        updates
    }

    #[test]
    fn installs_only_an_available_update_on_a_copy_that_can_replace_itself() {
        let mut updates = available();
        assert!(matches!(updates.install(), Some(Command::InstallUpdate)));
        assert!(updates.install().is_none(), "installed twice at once");

        let mut up_to_date = Updates::new(InstallSupport::Supported);
        up_to_date.apply(&Event::LatestRelease(release(&format!(
            "v{}",
            version::VERSION
        ))));
        assert!(up_to_date.install().is_none());

        let mut from_a_deb = Updates::new(InstallSupport::Unsupported("a .deb".to_owned()));
        from_a_deb.apply(&Event::LatestRelease(release("v99.0.0")));
        assert!(from_a_deb.install().is_none());
    }

    #[test]
    fn follows_the_download_then_restarts_only_into_an_installed_update() {
        let mut updates = available();
        assert!(
            !updates.request_restart(),
            "restarted with nothing installed"
        );
        let _ = updates.install();

        updates.apply(&Event::UpdateProgress {
            downloaded: 25,
            total: Some(100),
        });
        assert_eq!(updates.install_state().fraction(), Some(0.25));

        updates.apply(&Event::UpdateInstalled(
            Err("signature mismatch".to_owned()),
        ));
        assert_eq!(
            updates.install_state(),
            &InstallState::Failed("signature mismatch".to_owned())
        );
        assert!(!updates.request_restart());
        assert!(
            updates.install().is_some(),
            "a failed install can't be retried"
        );

        updates.apply(&Event::UpdateInstalled(Ok(InstalledUpdate {
            version: "99.0.0".to_owned(),
            restart: Restart::Relaunch,
        })));
        assert_eq!(updates.restart_after_quit(), None, "restarted unasked");
        assert!(updates.request_restart());
        assert_eq!(updates.restart_after_quit(), Some(&Restart::Relaunch));
        updates.cancel_restart();
        assert_eq!(updates.restart_after_quit(), None);
    }

    #[test]
    fn the_releases_repository_is_the_one_in_cargo_toml() {
        let repository = env!("CARGO_PKG_REPOSITORY");
        assert!(
            repository.ends_with(&format!("/{RELEASES_REPOSITORY}")),
            "{repository}"
        );
    }
}
