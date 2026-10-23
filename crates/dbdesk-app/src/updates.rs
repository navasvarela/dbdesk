//! Whether a newer dbdesk has been released: what Help › Check for updates shows, and the
//! notice in the status bar.

use dbdesk_engine::{Command, Event, Release};
use dbdesk_store::Settings;
use semver::Version;

use crate::version;

/// The GitHub repository whose releases are dbdesk's, as `owner/name`. The same repository
/// as `repository` in Cargo.toml; a test keeps them in step.
pub const RELEASES_REPOSITORY: &str = "navasvarela/dbdesk";

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

#[derive(Debug)]
pub struct Updates {
    status: UpdateStatus,
    /// Set once the check at start-up has been decided on, so a later settings reload
    /// doesn't check again.
    start_decided: bool,
    pub dialog_open: bool,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            status: UpdateStatus::NotChecked,
            start_decided: false,
            dialog_open: false,
        }
    }
}

impl Updates {
    pub fn status(&self) -> &UpdateStatus {
        &self.status
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
        if let Event::LatestRelease(result) = event {
            self.status = status_for(result, version::VERSION);
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

    #[test]
    fn the_releases_repository_is_the_one_in_cargo_toml() {
        let repository = env!("CARGO_PKG_REPOSITORY");
        assert!(
            repository.ends_with(&format!("/{RELEASES_REPOSITORY}")),
            "{repository}"
        );
    }
}
