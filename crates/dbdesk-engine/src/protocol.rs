use dbdesk_core::{CatalogNode, CatalogPath, DataSource, DataSourceId, Password, ServerInfo};

/// A request from the UI to the engine.
///
/// Shutting the [`EngineHandle`](crate::EngineHandle) down closes the command channel, which is
/// how the UI asks the engine to stop.
#[derive(Debug)]
pub enum Command {
    /// Creates or updates (matched by id) a data source, storing its password if one is set.
    /// Answered by [`Event::DataSourceSaved`] or [`Event::DataSourceSaveFailed`].
    SaveDataSource {
        data_source: DataSource,
        password: PasswordUpdate,
    },
    /// Disconnects, forgets the data source and removes its saved password.
    DeleteDataSource(DataSourceId),
    /// Connects with these settings without saving them, then disconnects.
    /// Answered by [`Event::ConnectionTested`].
    TestConnection {
        data_source: DataSource,
        password: PasswordUpdate,
    },
    Connect(DataSourceId),
    Disconnect(DataSourceId),
    /// Lists the children of a catalog node, connecting first if needed.
    /// Answered by [`Event::ChildrenLoaded`].
    LoadChildren {
        id: DataSourceId,
        path: CatalogPath,
    },
    /// Answered by [`Event::DefinitionLoaded`].
    LoadDefinition {
        id: DataSourceId,
        path: CatalogPath,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordUpdate {
    /// Use whatever password is saved, if any.
    Keep,
    Set(Password),
}

/// Something the engine reports to the UI.
///
/// Errors arrive as display strings: the UI only shows them, and the full error has already
/// been logged by the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The engine's command loop is running and accepting commands.
    Started,
    DataSourcesLoaded(Vec<DataSource>),
    /// `data_sources.toml` exists but can't be read. Saving is refused until it is fixed, so
    /// a broken file is never overwritten with an empty list.
    DataSourcesUnavailable {
        error: String,
    },
    DataSourceSaved(DataSource),
    DataSourceSaveFailed {
        id: DataSourceId,
        error: String,
    },
    DataSourceDeleted(DataSourceId),
    SessionChanged {
        id: DataSourceId,
        state: SessionState,
    },
    ConnectionTested {
        id: DataSourceId,
        result: Result<ServerInfo, String>,
    },
    ChildrenLoaded {
        id: DataSourceId,
        path: CatalogPath,
        result: Result<Vec<CatalogNode>, String>,
    },
    DefinitionLoaded {
        id: DataSourceId,
        path: CatalogPath,
        result: Result<String, String>,
    },
}

/// The state of a data source's connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SessionState {
    #[default]
    Disconnected,
    Connecting,
    Connected(ServerInfo),
    Failed(String),
}
