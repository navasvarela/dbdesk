//! Where dbdesk keeps its files. See `docs/persistence.md` for the full layout.

use std::path::PathBuf;

use directories::ProjectDirs;

/// The application name used for platform directories. Also the eframe app id, so the
/// window/layout state eframe persists lands in the same folder.
pub const APP_NAME: &str = "dbdesk";

/// dbdesk's directories on this machine (`~/Library/Application Support/dbdesk` on macOS).
#[derive(Debug, Clone)]
pub struct AppPaths {
    dirs: ProjectDirs,
}

impl AppPaths {
    /// Resolves the platform directories, or `None` if the OS reports no home directory.
    pub fn discover() -> Option<Self> {
        ProjectDirs::from("", "", APP_NAME).map(|dirs| Self { dirs })
    }

    /// Rolling log files. Not guaranteed to exist yet.
    pub fn logs_dir(&self) -> PathBuf {
        self.dirs.data_dir().join("logs")
    }
}
