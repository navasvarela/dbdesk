use std::path::PathBuf;

use directories::{BaseDirs, ProjectDirs};

use crate::{CustomFiles, TrustedCommandsFile};

/// The application name used for platform directories. Also the eframe app id, so the
/// window/layout state eframe persists lands in the same folder.
pub const APP_NAME: &str = "dbdesk";

/// dbdesk's directories on this machine (`~/Library/Application Support/dbdesk` on macOS).
#[derive(Debug, Clone)]
pub struct AppPaths {
    dirs: ProjectDirs,
    home: PathBuf,
}

impl AppPaths {
    /// Resolves the platform directories, or `None` if the OS reports no home directory.
    pub fn discover() -> Option<Self> {
        Some(Self {
            dirs: ProjectDirs::from("", "", APP_NAME)?,
            home: BaseDirs::new()?.home_dir().to_owned(),
        })
    }

    /// OpenSSH's file of trusted host keys, which SSH tunnels share with `ssh`. Not
    /// guaranteed to exist.
    pub fn ssh_known_hosts_file(&self) -> PathBuf {
        self.home.join(".ssh").join("known_hosts")
    }

    /// Rolling log files. Not guaranteed to exist yet.
    pub fn logs_dir(&self) -> PathBuf {
        self.dirs.data_dir().join("logs")
    }

    /// Saved data sources. Not guaranteed to exist yet.
    pub fn data_sources_file(&self) -> PathBuf {
        self.dirs.config_dir().join("data_sources.toml")
    }

    /// Projects and their MCP permissions. Not guaranteed to exist yet.
    pub fn projects_file(&self) -> PathBuf {
        self.dirs.config_dir().join("projects.toml")
    }

    /// Query console files, one folder per data source. Not guaranteed to exist yet.
    pub fn consoles_dir(&self) -> PathBuf {
        self.dirs.config_dir().join("consoles")
    }

    /// Key bindings, if the user wrote any. Not guaranteed to exist.
    pub fn keymap_file(&self) -> PathBuf {
        self.dirs.config_dir().join("keymap.toml")
    }

    /// Preferences, such as how SQL is formatted. Not guaranteed to exist.
    pub fn settings_file(&self) -> PathBuf {
        self.dirs.config_dir().join("settings.toml")
    }

    /// `actions.toml`, `snippets.toml` and `theme.toml`, if the user wrote them, beside the
    /// keymap.
    pub fn custom_files(&self) -> CustomFiles {
        CustomFiles::new(self.dirs.config_dir())
    }

    /// The command actions the user chose to run. In the data folder rather than beside
    /// `actions.toml`, so a config folder copied from elsewhere brings no trust with it.
    pub fn trusted_commands_file(&self) -> TrustedCommandsFile {
        TrustedCommandsFile::new(self.dirs.data_dir().join("trusted_commands.toml"))
    }

    /// The query history database. Not guaranteed to exist yet.
    pub fn history_file(&self) -> PathBuf {
        self.dirs.config_dir().join("history.sqlite")
    }
}
