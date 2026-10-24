//! Downloading the newest release and installing it over the running copy of dbdesk.

use std::cell::Cell;
use std::sync::Arc;
use std::time::Duration;

use cargo_packager_updater::semver::Version;
use cargo_packager_updater::url::Url;
use cargo_packager_updater::{Config, UpdaterBuilder};
use dbdesk_core::describe_error;

use crate::Event;
use crate::events::EventSink;

/// How long one request may take, the download included: the updater applies one timeout to
/// both. Ten minutes fits a 30 MB package on a slow connection, and still ends a request that
/// has hung.
const TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Whether this copy of dbdesk can replace itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallSupport {
    Supported,
    /// Why not, in words for the update dialog, which then offers the release page instead.
    Unsupported(String),
}

/// Installs updates, so the engine can be tested without downloading or replacing anything.
pub trait UpdateInstaller: Send + Sync {
    /// Whether this copy can update itself. Cheap: only looks at how it was installed.
    fn support(&self) -> InstallSupport;

    /// Downloads the newest release, checks its signature, and installs it over this copy.
    /// Blocking; the engine calls it on its blocking pool. `progress` gets the bytes downloaded
    /// so far and the total when the server says. Returns the installed version.
    ///
    /// # Errors
    ///
    /// When there is nothing newer, this copy can't update itself, or the download, the
    /// signature check or the install fails. A failed install leaves the running copy as it
    /// was.
    fn install(&self, progress: &dyn Fn(u64, Option<u64>)) -> Result<String, InstallError>;
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("there is no newer release to install")]
    NoUpdate,
    #[error("{0}")]
    Unsupported(String),
    #[error("the update manifest's address isn't a URL")]
    ManifestUrl(#[source] cargo_packager_updater::url::ParseError),
    #[error("this build's version, {0}, isn't a version")]
    CurrentVersion(String),
    #[error("could not update dbdesk")]
    Updater(#[from] cargo_packager_updater::Error),
}

/// Installs the packages the release workflow signs, using cargo-packager's updater: the
/// macOS `.app.tar.gz` replaces the app bundle, and the AppImage replaces itself.
///
/// The manifest is the `latest.json` attached to each GitHub release, which lists one signed
/// package per platform (see `.github/workflows/release.yml`). A package whose signature doesn't
/// match `public_key` is refused before anything is replaced.
pub struct PackagerInstaller {
    manifest: String,
    public_key: String,
    current_version: String,
}

impl PackagerInstaller {
    pub fn new(manifest: &str, public_key: &str, current_version: &str) -> Self {
        Self {
            manifest: manifest.to_owned(),
            public_key: public_key.to_owned(),
            current_version: current_version.to_owned(),
        }
    }
}

impl UpdateInstaller for PackagerInstaller {
    fn support(&self) -> InstallSupport {
        install_support()
    }

    fn install(&self, progress: &dyn Fn(u64, Option<u64>)) -> Result<String, InstallError> {
        if let InstallSupport::Unsupported(reason) = self.support() {
            return Err(InstallError::Unsupported(reason));
        }
        let current = Version::parse(&self.current_version)
            .map_err(|_| InstallError::CurrentVersion(self.current_version.clone()))?;
        let manifest = Url::parse(&self.manifest).map_err(InstallError::ManifestUrl)?;
        let config = Config {
            endpoints: vec![manifest],
            pubkey: self.public_key.clone(),
            windows: None,
        };
        let updater = UpdaterBuilder::new(current, config)
            .timeout(TIMEOUT)
            .build()?;
        let update = updater.check()?.ok_or(InstallError::NoUpdate)?;
        let downloaded = Cell::new(0_u64);
        let bytes = update.download_extended(
            |chunk, total| {
                downloaded.set(downloaded.get() + chunk as u64);
                progress(downloaded.get(), total);
            },
            || {},
        )?;
        update.install(bytes)?;
        Ok(update.version.clone())
    }
}

/// Whether the running copy was installed in a way it can replace: the macOS app bundle, or the
/// AppImage. A `.deb` belongs to the package manager, and a build from source to its builder.
fn install_support() -> InstallSupport {
    if cfg!(target_os = "macos") {
        let in_bundle = std::env::current_exe().is_ok_and(|exe| {
            exe.ancestors()
                .any(|dir| dir.extension().is_some_and(|ext| ext == "app"))
        });
        if in_bundle {
            InstallSupport::Supported
        } else {
            InstallSupport::Unsupported(
                "This copy isn't running from dbdesk.app (it was built from source), so it \
                 can't replace itself."
                    .to_owned(),
            )
        }
    } else if cfg!(target_os = "linux") {
        if std::env::var_os("APPIMAGE").is_some() {
            InstallSupport::Supported
        } else {
            InstallSupport::Unsupported(
                "Only the AppImage updates itself. Install the new .deb from the release page, \
                 or update dbdesk the way you installed it."
                    .to_owned(),
            )
        }
    } else {
        InstallSupport::Unsupported(
            "dbdesk can't update itself on this system yet. Download the new version from the \
             release page."
                .to_owned(),
        )
    }
}

/// How often progress is reported: at each whole percent when the size is known, otherwise
/// every this many bytes. Enough for a smooth bar without an event per network read.
const UNKNOWN_SIZE_STEP: u64 = 512 * 1024;

/// Decides which progress updates are worth an event.
#[derive(Debug, Default)]
struct ProgressThrottle {
    last: Option<u64>,
}

impl ProgressThrottle {
    fn should_report(&mut self, downloaded: u64, total: Option<u64>) -> bool {
        let step = match total {
            Some(total) if total > 0 => downloaded.saturating_mul(100) / total,
            _ => downloaded / UNKNOWN_SIZE_STEP,
        };
        let report = self.last != Some(step);
        self.last = Some(step);
        report
    }
}

/// Installs on the blocking pool, reporting [`Event::UpdateProgress`] while downloading and
/// [`Event::UpdateInstalled`] at the end.
pub(crate) async fn install(installer: Arc<dyn UpdateInstaller>, events: EventSink) {
    let progress_events = events.clone();
    let task = tokio::task::spawn_blocking(move || {
        let throttle = std::sync::Mutex::new(ProgressThrottle::default());
        installer.install(&|downloaded, total| {
            let report = throttle
                .lock()
                .map(|mut throttle| throttle.should_report(downloaded, total))
                .unwrap_or(true);
            if report {
                progress_events.send(Event::UpdateProgress { downloaded, total });
            }
        })
    });
    let result = match task.await {
        Ok(Ok(version)) => Ok(version),
        Ok(Err(error)) => Err(describe_error(&error)),
        Err(error) => Err(describe_error(&error)),
    };
    match &result {
        Ok(version) => tracing::info!("installed dbdesk {version}; it starts after a restart"),
        Err(error) => tracing::error!("could not install the update: {error}"),
    }
    events.send(Event::UpdateInstalled(result));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_each_percent_or_each_half_megabyte() {
        let mut throttle = ProgressThrottle::default();
        let reported: Vec<u64> = [0, 5, 10, 15, 20, 1_000]
            .into_iter()
            .filter(|&downloaded| throttle.should_report(downloaded, Some(1_000)))
            .collect();
        assert_eq!(reported, [0, 10, 20, 1_000]);

        let mut throttle = ProgressThrottle::default();
        let reported: Vec<u64> = [0, 100, UNKNOWN_SIZE_STEP, UNKNOWN_SIZE_STEP + 1]
            .into_iter()
            .filter(|&downloaded| throttle.should_report(downloaded, None))
            .collect();
        assert_eq!(reported, [0, UNKNOWN_SIZE_STEP]);
    }
}
