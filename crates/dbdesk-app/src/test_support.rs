//! Helpers for tests that drive the whole app: a real engine, a data source file in a
//! temporary directory, and passwords kept in memory instead of the OS keychain.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dbdesk_core::{DataSourceId, Password};
use dbdesk_engine::{
    EngineConfig, InstallError, InstallSupport, InstalledUpdate, ProcessRunner, Release,
    ReleaseError, ReleaseFeed, Restart, UpdateInstaller,
};
use dbdesk_store::{
    ConsoleFiles, CustomFiles, DataSourceFile, HistoryFile, KeymapFile, ProjectFile, Secret,
    SecretError, SecretStore, SettingsFile, TrustedCommandsFile,
};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, HarnessBuilder};

use crate::app::DbDeskApp;
use crate::log_console::LogFeed;

/// How long to wait for engine events before failing. Generous, because the first connection
/// to a database can be slow on a busy CI machine.
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Passwords in memory, so tests never touch the real keychain.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<(DataSourceId, Secret), Password>>);

impl MemorySecrets {
    pub fn with_password(id: DataSourceId, password: Password) -> Self {
        Self(Mutex::new(HashMap::from([(
            (id, Secret::Database),
            password,
        )])))
    }

    pub fn saved(&self, id: DataSourceId, secret: Secret) -> Option<Password> {
        self.0.lock().unwrap().get(&(id, secret)).cloned()
    }
}

impl SecretStore for MemorySecrets {
    fn get(&self, id: DataSourceId, secret: Secret) -> Result<Option<Password>, SecretError> {
        Ok(self.saved(id, secret))
    }

    fn set(&self, id: DataSourceId, secret: Secret, value: &Password) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert((id, secret), value.clone());
        Ok(())
    }

    fn delete(&self, id: DataSourceId, secret: Secret) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(&(id, secret));
        Ok(())
    }
}

/// A release feed that never touches the network: it reports this release, or none.
pub struct FixedReleases(pub Option<Release>);

impl FixedReleases {
    /// The running version as the newest release, so the app finds itself up to date.
    pub fn current() -> Self {
        Self::tagged(&format!("v{}", crate::version::VERSION))
    }

    pub fn tagged(tag: &str) -> Self {
        Self(Some(Release {
            tag: tag.to_owned(),
            page: format!("https://example.com/releases/{tag}"),
        }))
    }
}

impl ReleaseFeed for FixedReleases {
    fn latest(&self) -> Result<Release, ReleaseError> {
        self.0.clone().ok_or(ReleaseError::NoReleases)
    }
}

/// An installer that never downloads or replaces anything: it can install, reports a
/// download in two steps, and says it installed `version`.
pub struct FakeInstaller {
    pub version: &'static str,
}

impl Default for FakeInstaller {
    fn default() -> Self {
        Self { version: "99.0.0" }
    }
}

impl UpdateInstaller for FakeInstaller {
    fn support(&self) -> InstallSupport {
        InstallSupport::Supported
    }

    fn install(
        &self,
        progress: &dyn Fn(u64, Option<u64>),
    ) -> Result<InstalledUpdate, InstallError> {
        progress(0, Some(100));
        progress(100, Some(100));
        Ok(InstalledUpdate {
            version: self.version.to_owned(),
            restart: Restart::Relaunch,
        })
    }
}

/// Starts the app with its data source file in `dir`.
pub fn start_app(
    builder: HarnessBuilder<DbDeskApp>,
    dir: &Path,
    log_feed: LogFeed,
    secrets: MemorySecrets,
) -> Harness<'static, DbDeskApp> {
    let config = EngineConfig {
        data_sources: DataSourceFile::new(dir.join("data_sources.toml")),
        projects: ProjectFile::new(dir.join("projects.toml")),
        secrets: Arc::new(secrets),
        consoles: ConsoleFiles::new(dir.join("consoles")),
        history: HistoryFile::new(dir.join("history.sqlite")),
        keymap: KeymapFile::new(dir.join("keymap.toml")),
        settings: SettingsFile::new(dir.join("settings.toml")),
        custom_files: CustomFiles::new(dir),
        trusted_commands: TrustedCommandsFile::new(dir.join("trusted_commands.toml")),
        programs: Arc::new(ProcessRunner),
        releases: Arc::new(FixedReleases::current()),
        updater: Arc::new(FakeInstaller::default()),
        known_hosts: dir.join("known_hosts"),
        idle_ping_after: dbdesk_engine::IDLE_PING_AFTER,
    };
    builder.build_eframe(|cc| DbDeskApp::new(cc, log_feed, config).unwrap())
}

/// Steps the app until at least one widget labelled with something containing `text`
/// appears; engine events arrive asynchronously, so a fixed number of steps would be flaky.
pub fn step_until_visible(harness: &mut Harness<'_, DbDeskApp>, text: &str) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while harness.query_all_by_label_contains(text).next().is_none() {
        assert!(Instant::now() < deadline, "{text:?} never appeared");
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
}

/// `Harness::new_ui` with dbdesk's fonts, which widgets that draw icons need. egui applies new
/// fonts from the next frame on, so the first frame only installs them.
pub fn ui_harness<'a>(mut app: impl FnMut(&mut egui::Ui) + 'a) -> Harness<'a> {
    let mut fonts_installed = false;
    let mut harness = Harness::new_ui(move |ui| {
        if fonts_installed {
            app(ui);
        } else {
            crate::fonts::install(ui.ctx());
            fonts_installed = true;
        }
    });
    harness.run();
    harness
}
