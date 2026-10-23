use dbdesk_core::{DataSource, DataSourceId};
use dbdesk_engine::{Event, SessionState};

use crate::catalog_tree::CatalogTree;

/// What the UI knows about the world, built only from engine events.
#[derive(Debug, Default)]
pub struct AppState {
    pub engine_status: EngineStatus,
    pub data_sources: DataSources,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum EngineStatus {
    #[default]
    Starting,
    Running,
    Stopped,
}

impl EngineStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum DataSources {
    #[default]
    Loading,
    Unavailable(String),
    /// Sorted by name, case-insensitively.
    Loaded(Vec<DataSourceEntry>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSourceEntry {
    pub data_source: DataSource,
    pub session: SessionState,
    pub catalog: CatalogTree,
}

impl DataSourceEntry {
    fn new(data_source: DataSource) -> Self {
        Self {
            data_source,
            session: SessionState::Disconnected,
            catalog: CatalogTree::default(),
        }
    }

    fn set_session(&mut self, state: &SessionState) {
        // Everything loaded came from the connection that just went away, and the next
        // connection may see a different catalog.
        if matches!(state, SessionState::Disconnected | SessionState::Failed(_)) {
            self.catalog.reset();
        }
        self.session = state.clone();
    }
}

impl AppState {
    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::Started => self.engine_status = EngineStatus::Running,
            Event::DataSourcesLoaded(data_sources) => {
                let mut entries: Vec<_> = data_sources
                    .iter()
                    .cloned()
                    .map(DataSourceEntry::new)
                    .collect();
                sort_by_name(&mut entries);
                self.data_sources = DataSources::Loaded(entries);
            }
            Event::DataSourcesUnavailable { error } => {
                self.data_sources = DataSources::Unavailable(error.clone());
            }
            Event::DataSourceSaved(data_source) => self.data_sources.upsert(data_source),
            Event::DataSourceDeleted(id) => self.data_sources.remove(*id),
            Event::SessionChanged { id, state } => {
                if let Some(entry) = self.data_sources.entry_mut(*id) {
                    entry.set_session(state);
                }
            }
            Event::ChildrenLoaded { id, path, result } => {
                if let Some(catalog) = self.catalog_mut(*id) {
                    catalog.loaded(path, result);
                }
            }
            // The dialogs, the definition window and the consoles track these themselves.
            Event::DataSourceSaveFailed { .. }
            | Event::ConsolesLoaded(_)
            | Event::KeymapLoaded { .. }
            | Event::HistoryFound { .. }
            | Event::ExportFinished { .. }
            | Event::TableDescribed { .. }
            | Event::EditsApplied { .. }
            | Event::TransactionChanged { .. }
            | Event::TransactionEnded { .. }
            | Event::ConsoleSaveFailed { .. }
            | Event::ConnectionTested { .. }
            | Event::DefinitionLoaded { .. }
            | Event::ConsoleSessionChanged { .. }
            | Event::StatementStarted { .. }
            | Event::RowsFetched { .. }
            | Event::StatementFinished { .. }
            | Event::RunFinished { .. } => {}
        }
    }

    pub fn data_source(&self, id: DataSourceId) -> Option<&DataSourceEntry> {
        self.data_sources
            .entries()
            .iter()
            .find(|e| e.data_source.id == id)
    }

    pub fn catalog_mut(&mut self, id: DataSourceId) -> Option<&mut CatalogTree> {
        self.data_sources
            .entry_mut(id)
            .map(|entry| &mut entry.catalog)
    }

    pub fn connected_count(&self) -> usize {
        self.data_sources
            .entries()
            .iter()
            .filter(|entry| matches!(entry.session, SessionState::Connected(_)))
            .count()
    }
}

impl DataSources {
    pub fn entries(&self) -> &[DataSourceEntry] {
        match self {
            Self::Loaded(entries) => entries,
            Self::Loading | Self::Unavailable(_) => &[],
        }
    }

    fn entry_mut(&mut self, id: DataSourceId) -> Option<&mut DataSourceEntry> {
        match self {
            Self::Loaded(entries) => entries.iter_mut().find(|e| e.data_source.id == id),
            Self::Loading | Self::Unavailable(_) => None,
        }
    }

    /// Updates the saved data source, keeping its session state, or adds it.
    fn upsert(&mut self, data_source: &DataSource) {
        let Self::Loaded(entries) = self else {
            return;
        };
        match entries
            .iter_mut()
            .find(|e| e.data_source.id == data_source.id)
        {
            Some(entry) => entry.data_source = data_source.clone(),
            None => entries.push(DataSourceEntry::new(data_source.clone())),
        }
        sort_by_name(entries);
    }

    fn remove(&mut self, id: DataSourceId) {
        if let Self::Loaded(entries) = self {
            entries.retain(|entry| entry.data_source.id != id);
        }
    }
}

fn sort_by_name(entries: &mut [DataSourceEntry]) {
    entries.sort_by_key(|entry| entry.data_source.name.to_lowercase());
}

#[cfg(test)]
mod tests {
    use dbdesk_core::{ConnectionConfig, ServerInfo, SqliteConfig};

    use super::*;

    fn data_source(name: &str) -> DataSource {
        DataSource {
            id: DataSourceId::generate(),
            name: name.to_owned(),
            config: ConnectionConfig::Sqlite(SqliteConfig {
                path: format!("/data/{name}.db").into(),
                read_only: false,
            }),
        }
    }

    fn names(state: &AppState) -> Vec<&str> {
        state
            .data_sources
            .entries()
            .iter()
            .map(|entry| entry.data_source.name.as_str())
            .collect()
    }

    #[test]
    fn engine_is_running_once_it_reports_started() {
        let mut state = AppState::default();
        assert_eq!(state.engine_status, EngineStatus::Starting);

        state.apply(&Event::Started);

        assert_eq!(state.engine_status, EngineStatus::Running);
    }

    #[test]
    fn keeps_data_sources_sorted_through_saves_and_deletes() {
        let mut state = AppState::default();
        let beta = data_source("beta");
        state.apply(&Event::DataSourcesLoaded(vec![
            beta.clone(),
            data_source("Alpha"),
        ]));
        assert_eq!(names(&state), ["Alpha", "beta"]);

        state.apply(&Event::DataSourceSaved(data_source("gamma")));
        state.apply(&Event::DataSourceDeleted(beta.id));

        assert_eq!(names(&state), ["Alpha", "gamma"]);
    }

    #[test]
    fn renaming_keeps_the_session() {
        let mut state = AppState::default();
        let mut renamed = data_source("old");
        state.apply(&Event::DataSourcesLoaded(vec![renamed.clone()]));
        let connected = SessionState::Connected(ServerInfo {
            product: "SQLite".to_owned(),
            version: "3.50.0".to_owned(),
        });
        state.apply(&Event::SessionChanged {
            id: renamed.id,
            state: connected.clone(),
        });

        renamed.name = "new".to_owned();
        state.apply(&Event::DataSourceSaved(renamed.clone()));

        let entry = state.data_source(renamed.id).unwrap();
        assert_eq!(entry.data_source.name, "new");
        assert_eq!(entry.session, connected);
        assert_eq!(state.connected_count(), 1);
    }

    #[test]
    fn losing_the_connection_forgets_the_loaded_catalog() {
        let mut state = AppState::default();
        let source = data_source("app");
        state.apply(&Event::DataSourcesLoaded(vec![source.clone()]));
        let root = dbdesk_core::CatalogPath::root();
        let catalog = state.catalog_mut(source.id).unwrap();
        assert!(catalog.expand(&root));

        state.apply(&Event::SessionChanged {
            id: source.id,
            state: SessionState::Connecting,
        });
        assert!(
            state
                .data_source(source.id)
                .unwrap()
                .catalog
                .is_expanded(&root)
        );
        state.apply(&Event::SessionChanged {
            id: source.id,
            state: SessionState::Failed("timed out".to_owned()),
        });

        assert!(
            !state
                .data_source(source.id)
                .unwrap()
                .catalog
                .is_expanded(&root)
        );
    }
}
