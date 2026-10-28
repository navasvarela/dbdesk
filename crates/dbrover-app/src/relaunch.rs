//! Starting DbRover again once it has quit, so an installed update takes over, or running the
//! Windows installer that does both.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use dbrover_engine::Restart;

/// Process-wide, because the request is made inside the app (when the window closes) but must
/// be acted on in `main`, after eframe has returned and the engine has stopped. Starting the
/// new copy, or the installer that replaces this one's files, any earlier would have two
/// programs writing the same files.
static REQUESTED: OnceLock<Restart> = OnceLock::new();

pub fn request(restart: Restart) {
    // The app asks once, as its window closes; a second request could only repeat the first.
    if REQUESTED.set(restart).is_err() {
        tracing::warn!("a restart was already requested");
    }
}

/// Starts the installed copy, or the kept installer, if a restart was requested. Failing to is
/// logged, not fatal: an installed update starts at the next launch either way, and a kept
/// installer is downloaded again by the next update.
pub fn start_if_requested() {
    let Some(restart) = REQUESTED.get() else {
        return;
    };
    let mut command = match restart {
        Restart::Relaunch => match std::env::current_exe() {
            Ok(exe) => target(&exe, std::env::var_os("APPIMAGE").map(PathBuf::from)).command(),
            Err(error) => {
                tracing::error!("could not find DbRover to restart it: {error}");
                return;
            }
        },
        Restart::RunInstaller(installer) => installer.command(),
    };
    match command.spawn() {
        Ok(_) => tracing::info!("restarting into the update ({restart:?})"),
        Err(error) => tracing::error!("could not restart DbRover: {error}"),
    }
}

/// What to start for the running copy's installed form.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    /// A macOS app bundle, started through LaunchServices so it gets its own Dock icon and
    /// is not a child of this process.
    Bundle(PathBuf),
    /// The AppImage file itself, whose path the AppImage runtime puts in `APPIMAGE`: the
    /// running executable is inside a temporary mount that disappears on exit.
    AppImage(PathBuf),
    Executable(PathBuf),
}

impl Target {
    fn command(&self) -> Command {
        match self {
            Self::Bundle(bundle) => {
                let mut command = Command::new("open");
                command.arg("-n").arg(bundle);
                command
            }
            Self::AppImage(path) | Self::Executable(path) => Command::new(path),
        }
    }
}

fn target(exe: &Path, appimage: Option<PathBuf>) -> Target {
    if let Some(appimage) = appimage {
        return Target::AppImage(appimage);
    }
    exe.ancestors()
        .find(|dir| dir.extension().is_some_and(|ext| ext == "app"))
        .map_or_else(
            || Target::Executable(exe.to_owned()),
            |bundle| Target::Bundle(bundle.to_owned()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarts_the_bundle_the_appimage_or_the_executable() {
        let cases = [
            (
                "/Applications/DbRover.app/Contents/MacOS/dbrover",
                None,
                Target::Bundle("/Applications/DbRover.app".into()),
            ),
            (
                "/tmp/.mount_dbroverXYZ/usr/bin/dbrover",
                Some("/home/ada/Apps/dbrover.AppImage"),
                Target::AppImage("/home/ada/Apps/dbrover.AppImage".into()),
            ),
            (
                "/home/ada/dbrover/target/release/dbrover",
                None,
                Target::Executable("/home/ada/dbrover/target/release/dbrover".into()),
            ),
        ];
        for (exe, appimage, expected) in cases {
            assert_eq!(
                target(Path::new(exe), appimage.map(PathBuf::from)),
                expected,
                "{exe}"
            );
        }
    }
}
