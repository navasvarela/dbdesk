//! Downloading the newest release and installing it over the running copy of DbRover, or, on
//! Windows, keeping its installer to run once DbRover has quit.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use cargo_packager_updater::semver::Version;
use cargo_packager_updater::url::Url;
use cargo_packager_updater::{
    Config, Update, UpdateFormat, UpdaterBuilder, WindowsUpdateInstallMode,
};
use dbrover_core::describe_error;

use crate::Event;
use crate::events::EventSink;

/// How long one request may take, the download included: the updater applies one timeout to
/// both. Ten minutes fits a 30 MB package on a slow connection, and still ends a request that
/// has hung.
const TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Whether this copy of DbRover can replace itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallSupport {
    Supported,
    /// Why not, in words for the update dialog, which then offers the release page instead.
    Unsupported(String),
}

/// An update that has been downloaded and checked, and what restarting DbRover does with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledUpdate {
    pub version: String,
    pub restart: Restart,
}

/// What finishes an update once DbRover has quit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restart {
    /// The new version has replaced this copy's files (the macOS bundle, the AppImage), so
    /// starting this copy again starts it.
    Relaunch,
    /// The new version's installer, kept to run once DbRover has quit: Windows can't replace
    /// the files of a running program. The installer starts the new version when it's done.
    RunInstaller(StagedInstaller),
}

/// The installer's name in its folder. Fixed rather than versioned, so each update replaces
/// the last one's installer instead of leaving another behind.
const STAGED_INSTALLER_NAME: &str = "dbrover-setup.exe";

/// A downloaded installer whose signature has been checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedInstaller(PathBuf);

impl From<PathBuf> for StagedInstaller {
    fn from(path: PathBuf) -> Self {
        Self(path)
    }
}

impl StagedInstaller {
    /// Keeps `bytes` as the installer in `dir`, replacing any kept by an earlier update.
    fn write(dir: &Path, bytes: &[u8]) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(STAGED_INSTALLER_NAME);
        std::fs::write(&path, bytes)?;
        Ok(Self(path))
    }

    /// Runs the installer the way `cargo-packager-updater` would: in passive mode, which shows
    /// a progress bar and asks nothing, and starting DbRover when it's done.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.0);
        command.args(WindowsUpdateInstallMode::Passive.nsis_args());
        command
    }
}

/// Installs updates, so the engine can be tested without downloading or replacing anything.
pub trait UpdateInstaller: Send + Sync {
    /// Whether this copy can update itself. Cheap: only looks at how it was installed.
    fn support(&self) -> InstallSupport;

    /// Downloads the newest release, checks its signature, and installs it over this copy, or
    /// keeps its installer to run once DbRover has quit. Blocking; the engine calls it on its
    /// blocking pool. `progress` gets the bytes downloaded so far and the total when the server
    /// says.
    ///
    /// # Errors
    ///
    /// When there is nothing newer, this copy can't update itself, or the download, the
    /// signature check or the install fails. A failed install leaves the running copy as it
    /// was.
    fn install(&self, progress: &dyn Fn(u64, Option<u64>))
    -> Result<InstalledUpdate, InstallError>;
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
    #[error("could not update DbRover")]
    Updater(#[from] cargo_packager_updater::Error),
    #[error("could not keep the downloaded installer")]
    KeepInstaller(#[source] std::io::Error),
}

/// Installs the packages the release workflow signs, using cargo-packager's updater: the
/// macOS `.app.tar.gz` replaces the app bundle, the AppImage replaces itself, and the Windows
/// installer is kept to run once DbRover has quit.
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

    fn install(
        &self,
        progress: &dyn Fn(u64, Option<u64>),
    ) -> Result<InstalledUpdate, InstallError> {
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
        let restart = put_in_place(&update, bytes)?;
        Ok(InstalledUpdate {
            version: update.version,
            restart,
        })
    }
}

/// Where the Windows installer is kept until DbRover quits.
fn staging_dir() -> PathBuf {
    std::env::temp_dir().join("dbrover-update")
}

/// Replaces this copy with the downloaded, verified package, or keeps it to run later.
fn put_in_place(update: &Update, bytes: Vec<u8>) -> Result<Restart, InstallError> {
    match update.format {
        UpdateFormat::App | UpdateFormat::AppImage => {
            update.install(bytes)?;
            Ok(Restart::Relaunch)
        }
        // Not `update.install`: for an installer it starts it and calls `std::process::exit`
        // at once, from this blocking thread, which would skip the app's checks for open
        // transactions and unsaved consoles on the way out.
        UpdateFormat::Nsis => StagedInstaller::write(&staging_dir(), &bytes)
            .map(Restart::RunInstaller)
            .map_err(InstallError::KeepInstaller),
        UpdateFormat::Wix => Err(InstallError::Unsupported(
            "The update is an .msi package, which DbRover doesn't install. Download the \
             installer from the release page."
                .to_owned(),
        )),
    }
}

/// Whether the running copy was installed in a way it can replace: the macOS app bundle, the
/// AppImage, or a copy put there by the Windows installer. A `.deb` belongs to the package
/// manager, and a portable copy or a build from source to whoever put it there.
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
                "This copy isn't running from DbRover.app (it was built from source), so it \
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
                 or update DbRover the way you installed it."
                    .to_owned(),
            )
        }
    } else if cfg!(target_os = "windows") {
        if std::env::current_exe().is_ok_and(|exe| installed_by_installer(&exe)) {
            InstallSupport::Supported
        } else {
            InstallSupport::Unsupported(
                "This copy wasn't installed with DbRover's installer (it's a portable copy, or \
                 was built from source), so it can't replace itself. Download the installer \
                 from the release page."
                    .to_owned(),
            )
        }
    } else {
        InstallSupport::Unsupported(
            "DbRover can't update itself on this system yet. Download the new version from the \
             release page."
                .to_owned(),
        )
    }
}

/// The uninstaller the NSIS installer writes beside `dbrover.exe` (cargo-packager's template
/// names it so). Its presence tells an installed copy from a portable one, which an installer
/// run would not update: the installer installs over the previous installation it finds in the
/// registry, not over the running copy.
const WINDOWS_UNINSTALLER_NAME: &str = "uninstall.exe";

fn installed_by_installer(exe: &Path) -> bool {
    exe.with_file_name(WINDOWS_UNINSTALLER_NAME).is_file()
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
        Ok(Ok(update)) => Ok(update),
        Ok(Err(error)) => Err(describe_error(&error)),
        Err(error) => Err(describe_error(&error)),
    };
    match &result {
        Ok(update) => tracing::info!(
            "DbRover {} is ready; restarting finishes the update ({:?})",
            update.version,
            update.restart
        ),
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

    #[test]
    fn keeps_one_installer_replacing_the_last_update_s() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("dbrover-update");
        let first = StagedInstaller::write(&staging, b"0.13.0").unwrap();
        let second = StagedInstaller::write(&staging, b"0.14.0").unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read(&second.0).unwrap(), b"0.14.0");
        assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 1);
    }

    #[test]
    fn runs_the_installer_passively_then_starts_dbrover() {
        let installer = StagedInstaller::from(PathBuf::from("dbrover-setup.exe"));
        let command = installer.command();
        assert_eq!(command.get_program(), "dbrover-setup.exe");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["/P", "/R"]);
    }

    #[test]
    fn only_a_copy_beside_its_uninstaller_was_installed() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("dbrover.exe");
        std::fs::write(&exe, b"").unwrap();
        assert!(!installed_by_installer(&exe), "a portable copy");
        std::fs::write(dir.path().join(WINDOWS_UNINSTALLER_NAME), b"").unwrap();
        assert!(installed_by_installer(&exe));
    }
}
