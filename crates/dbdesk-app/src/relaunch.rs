//! Starting dbdesk again once it has quit, so an installed update takes over.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

/// Process-wide, because the request is made inside the app (when the window closes) but must
/// be acted on in `main`, after eframe has returned and the engine has stopped. Starting the
/// new copy any earlier would have two copies writing the same files.
static REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

/// Starts the installed copy, if a restart was requested. Failing to start it is logged, not
/// fatal: the update is installed either way and starts at the next launch.
pub fn start_if_requested() {
    if !REQUESTED.load(Ordering::SeqCst) {
        return;
    }
    let target = match std::env::current_exe() {
        Ok(exe) => target(&exe, std::env::var_os("APPIMAGE").map(PathBuf::from)),
        Err(error) => {
            tracing::error!("could not find dbdesk to restart it: {error}");
            return;
        }
    };
    match target.command().spawn() {
        Ok(_) => tracing::info!("restarting into the installed update"),
        Err(error) => tracing::error!("could not restart dbdesk: {error}"),
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
                "/Applications/dbdesk.app/Contents/MacOS/dbdesk",
                None,
                Target::Bundle("/Applications/dbdesk.app".into()),
            ),
            (
                "/tmp/.mount_dbdeskXYZ/usr/bin/dbdesk",
                Some("/home/ada/Apps/dbdesk.AppImage"),
                Target::AppImage("/home/ada/Apps/dbdesk.AppImage".into()),
            ),
            (
                "/home/ada/dbdesk/target/release/dbdesk",
                None,
                Target::Executable("/home/ada/dbdesk/target/release/dbdesk".into()),
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
