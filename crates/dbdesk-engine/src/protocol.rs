use std::path::PathBuf;
use std::time::Duration;

use dbdesk_core::{
    CatalogNode, CatalogPath, ColumnInfo, DataSource, DataSourceId, Password, ProjectId, Rejection,
    RelationName, Row, ServerInfo, TableDescription, TransactionStatus,
};
use dbdesk_store::{
    ConsoleKey, HistoryEntry, KeymapBindings, Project, Projects, SavedConsole, Settings,
};

use crate::{Export, Release};

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
    /// Adds a project, invisible to MCP, with a slug made from its name. The UI picks the id,
    /// so it can move a data source into the new project right after. Answered by
    /// [`Event::ProjectSaved`] or [`Event::ProjectSaveFailed`].
    CreateProject {
        id: ProjectId,
        name: String,
    },
    /// Changes a project's name, keeping its slug. Answered by [`Event::ProjectSaved`] or
    /// [`Event::ProjectSaveFailed`].
    RenameProject {
        id: ProjectId,
        name: String,
    },
    /// Moves the project's data sources to no project, then forgets the project. Never
    /// deletes a data source. Answered by an [`Event::DataSourceSaved`] per data source moved,
    /// then [`Event::ProjectDeleted`], or by [`Event::ProjectSaveFailed`].
    DeleteProject(ProjectId),
    /// Puts a data source in a project, or in no project. Answered by
    /// [`Event::DataSourceSaved`] or [`Event::DataSourceSaveFailed`].
    MoveToProject {
        data_source: DataSourceId,
        project: Option<ProjectId>,
    },
    /// Connects with these settings without saving them, then disconnects.
    /// Answered by [`Event::ConnectionTested`].
    TestConnection {
        data_source: DataSource,
        password: PasswordUpdate,
    },
    Connect(DataSourceId),
    /// Disconnects the explorer's connection and every console's connection to the data
    /// source, rolling back their open transactions.
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
    /// Creates a new, empty SQLite database file, refusing to overwrite an existing one.
    /// Answered by [`Event::SqliteDatabaseCreated`].
    CreateSqliteDatabase(PathBuf),
    /// Runs statements in order on the console's own connection, connecting it to
    /// `data_source` first if needed, and stops at the first failure. Each statement's first
    /// page of rows is reported; the last statement's remaining rows stay available through
    /// [`Command::FetchMore`] until the next run.
    Execute {
        console: ConsoleId,
        data_source: DataSourceId,
        run: RunId,
        statements: Vec<String>,
        transactions: TransactionMode,
    },
    /// Commits or rolls back the console's open transaction. Answered by
    /// [`Event::TransactionEnded`].
    EndTransaction {
        console: ConsoleId,
        end: TransactionEnd,
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
    /// Disconnects the console's connection, which rolls back its open transaction.
    CloseConsole(ConsoleId),
    /// Writes a console's text to its file. Saves are applied in the order sent. A failure is
    /// reported as [`Event::ConsoleSaveFailed`].
    SaveConsole {
        key: ConsoleKey,
        text: String,
    },
    /// Removes a console's file, if it exists.
    DeleteConsoleFile(ConsoleKey),
    /// Resolves and describes a table on the explorer's connection, connecting first if
    /// needed, so a console's open result isn't disturbed. Answered by
    /// [`Event::TableDescribed`].
    DescribeTable {
        data_source: DataSourceId,
        table: RelationName,
    },
    /// Runs generated `UPDATE`s on the console's connection, all or none: each must change
    /// exactly one row. Inside an open transaction they stay part of it (behind a savepoint);
    /// otherwise they commit together. Answered by [`Event::EditsApplied`].
    ApplyEdits {
        console: ConsoleId,
        statements: Vec<String>,
    },
    /// Asks the console's own connection whether the database would accept each statement,
    /// without running any (see `Connection::validate`). Never connects: a console that
    /// isn't connected has every statement skipped. Answered by
    /// [`Event::StatementsValidated`].
    ValidateStatements {
        console: ConsoleId,
        statements: Vec<String>,
    },
    /// Reads `keymap.toml` again. Answered by [`Event::KeymapLoaded`].
    ReloadKeymap,
    /// Writes a new `keymap.toml` holding `text`, unless one exists, then reads it.
    /// Answered by [`Event::KeymapLoaded`].
    CreateKeymap {
        text: String,
    },
    /// Reads `settings.toml` again. Answered by [`Event::SettingsLoaded`].
    ReloadSettings,
    /// Replaces `settings.toml` with these settings, then reads it. Answered by
    /// [`Event::SettingsLoaded`].
    SaveSettings(Settings),
    /// Writes rows a console shows to a file. Answered by [`Event::ExportFinished`].
    ExportRows {
        console: ConsoleId,
        export: Export,
    },
    /// Looks up the data source's past statements containing `text`, newest first.
    /// Answered by [`Event::HistoryFound`].
    SearchHistory {
        data_source: DataSourceId,
        text: String,
    },
    /// Asks the release feed for the newest published release. Answered by
    /// [`Event::LatestRelease`].
    CheckForUpdates,
}

/// How a console's statements are committed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TransactionMode {
    /// Each statement commits on its own, unless the user opens a transaction with `BEGIN`.
    #[default]
    Auto,
    /// A transaction is started before the first statement that changes anything, and stays
    /// open until committed or rolled back.
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionEnd {
    Commit,
    Rollback,
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
    /// What `projects.toml` holds, read at start-up, right before the data sources. No
    /// projects if there is no file.
    ProjectsLoaded(Projects),
    /// `projects.toml` exists but can't be read. Changing projects is refused until it is
    /// fixed, so a broken file is never overwritten.
    ProjectsUnavailable {
        error: String,
    },
    /// A project was created or renamed.
    ProjectSaved(Project),
    ProjectSaveFailed {
        id: ProjectId,
        error: String,
    },
    /// The project is gone; its data sources were moved to no project first.
    ProjectDeleted(ProjectId),
    /// Every saved console file, read once at start-up, right after the data sources.
    ConsolesLoaded(Vec<SavedConsole>),
    /// The bindings `keymap.toml` sets, read at start-up (before the data sources) and on
    /// request. Empty if there is no file.
    KeymapLoaded {
        path: PathBuf,
        result: Result<KeymapBindings, String>,
    },
    /// What `settings.toml` holds, read at start-up (after the keymap, before the data
    /// sources), after a save and on request. The defaults if there is no file.
    SettingsLoaded {
        path: PathBuf,
        result: Result<Settings, String>,
    },
    /// The answer to [`Command::CheckForUpdates`]: the newest published release, or why it
    /// couldn't be found out.
    LatestRelease(Result<Release, String>),
    ConsoleSaveFailed {
        key: ConsoleKey,
        error: String,
    },
    /// The answer to [`Command::DescribeTable`]: `None` if there is no such plain table.
    TableDescribed {
        data_source: DataSourceId,
        table: RelationName,
        result: Result<Option<TableDescription>, String>,
    },
    /// The answer to [`Command::ApplyEdits`].
    EditsApplied {
        console: ConsoleId,
        result: Result<(), EditFailure>,
    },
    /// The answer to [`Command::ValidateStatements`]: each statement, as sent, and what the
    /// database said.
    StatementsValidated {
        console: ConsoleId,
        results: Vec<(String, StatementCheck)>,
    },
    /// The answer to [`Command::ExportRows`]: how many rows were written, or why none were.
    ExportFinished {
        console: ConsoleId,
        path: PathBuf,
        result: Result<usize, String>,
    },
    /// The answer to [`Command::SearchHistory`], echoing what was searched for so the UI can
    /// drop answers to searches it has moved on from.
    HistoryFound {
        data_source: DataSourceId,
        text: String,
        result: Result<Vec<HistoryEntry>, String>,
    },
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
    SqliteDatabaseCreated {
        path: PathBuf,
        result: Result<(), String>,
    },
    ConsoleSessionChanged {
        console: ConsoleId,
        state: SessionState,
    },
    /// The console's connection entered or left a transaction, by any statement or by
    /// [`Command::EndTransaction`]. A new connection starts outside one, and closing one ends
    /// its transaction, so neither is reported.
    TransactionChanged {
        console: ConsoleId,
        status: TransactionStatus,
    },
    /// The answer to [`Command::EndTransaction`]: what actually happened, which is a rollback
    /// when committing a transaction that had failed.
    TransactionEnded {
        console: ConsoleId,
        result: Result<TransactionEnd, String>,
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

/// What the database said about a statement it was asked to prepare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatementCheck {
    Accepted,
    Rejected(Rejection),
    /// Not asked: the console isn't connected, holds unread rows or an open transaction, the
    /// statement isn't a data query or change, or the database couldn't be asked.
    Skipped,
}

/// Why edits weren't applied. Nothing was: the changes were rolled back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditFailure {
    /// The index of the statement that failed, if one did (rather than starting or ending
    /// the transaction around them).
    pub statement: Option<usize>,
    pub message: String,
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
