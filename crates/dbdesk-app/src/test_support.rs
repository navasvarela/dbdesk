//! Helpers for tests that drive the whole app: a real engine, a data source file in a
//! temporary directory, and passwords kept in memory instead of the OS keychain.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dbdesk_core::{DataSourceId, Password};
use dbdesk_engine::{EngineConfig, Release, ReleaseError, ReleaseFeed};
use dbdesk_store::{
    ConsoleFiles, DataSourceFile, HistoryFile, KeymapFile, SecretError, SecretStore, SettingsFile,
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
pub struct MemorySecrets(Mutex<HashMap<DataSourceId, Password>>);

impl MemorySecrets {
    pub fn with_password(id: DataSourceId, password: Password) -> Self {
        Self(Mutex::new(HashMap::from([(id, password)])))
    }
}

impl SecretStore for MemorySecrets {
    fn password(&self, id: DataSourceId) -> Result<Option<Password>, SecretError> {
        Ok(self.0.lock().unwrap().get(&id).cloned())
    }

    fn set_password(&self, id: DataSourceId, password: &Password) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(id, password.clone());
        Ok(())
    }

    fn delete_password(&self, id: DataSourceId) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(&id);
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

/// Starts the app with its data source file in `dir`.
pub fn start_app(
    builder: HarnessBuilder<DbDeskApp>,
    dir: &Path,
    log_feed: LogFeed,
    secrets: MemorySecrets,
) -> Harness<'static, DbDeskApp> {
    let config = EngineConfig {
        data_sources: DataSourceFile::new(dir.join("data_sources.toml")),
        secrets: Arc::new(secrets),
        consoles: ConsoleFiles::new(dir.join("consoles")),
        history: HistoryFile::new(dir.join("history.sqlite")),
        keymap: KeymapFile::new(dir.join("keymap.toml")),
        settings: SettingsFile::new(dir.join("settings.toml")),
        releases: Arc::new(FixedReleases::current()),
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
