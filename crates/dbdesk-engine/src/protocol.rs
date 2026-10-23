use std::time::Duration;

use dbdesk_core::{
    CatalogNode, CatalogPath, ColumnInfo, DataSource, DataSourceId, Password, Row, ServerInfo,
};

/// Identifies a query console. Chosen by the UI, unique for the app's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConsoleId(pub u64);

/// Identifies one execution in a console, so events from an older run can be told apart.
/// Chosen by the UI, unique for the app's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RunId(pub u64);

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
    /// Runs statements in order on the console's own connection, connecting it to
    /// `data_source` first if needed, and stops at the first failure. Each statement's first
    /// page of rows is reported; the last statement's remaining rows stay available through
    /// [`Command::FetchMore`] until the next run.
    Execute {
        console: ConsoleId,
        data_source: DataSourceId,
        run: RunId,
        statements: Vec<String>,
    },
    /// Reads the next page of the run's last statement.
    FetchMore {
        console: ConsoleId,
        run: RunId,
    },
    /// Stops the run's current statement, or discards its unread rows.
    Cancel {
        console: ConsoleId,
        run: RunId,
    },
    /// Disconnects the console's connection.
    CloseConsole(ConsoleId),
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
    ConsoleSessionChanged {
        console: ConsoleId,
        state: SessionState,
    },
    /// A statement that returns rows has started; its rows follow as [`Event::RowsFetched`].
    StatementStarted {
        run: RunId,
        statement: usize,
        columns: Vec<ColumnInfo>,
    },
    RowsFetched {
        run: RunId,
        statement: usize,
        rows: Vec<Row>,
        /// Whether [`Command::FetchMore`] can read more.
        more: bool,
    },
    StatementFinished {
        run: RunId,
        statement: usize,
        result: Result<StatementSummary, StatementFailure>,
    },
    /// Every statement has run, or one failed. Rows of the last statement may still be
    /// fetched afterwards.
    RunFinished {
        run: RunId,
        outcome: RunOutcome,
        elapsed: Duration,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementSummary {
    /// Rows changed (or returned), as the database reports it.
    pub rows_changed: Option<u64>,
    /// Time until the statement completed, or until its first page for row results.
    pub elapsed: Duration,
    /// Whether rows were left unread.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementFailure {
    pub message: String,
    /// Byte offset into the statement's text, when the database reports one.
    pub position: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Completed,
    Failed,
    Cancelled,
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
