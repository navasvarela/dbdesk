//! Query consoles: a SQL editor bound to a data source, and the results of its latest run.
//!
//! This module holds the state and its rules (what to run, how events update a run); `view`
//! and `results` draw it. See `docs/features/sql-runner.md`.

mod highlight;
mod results;
mod view;

use std::ops::Range;
use std::time::{Duration, Instant};

use dbdesk_core::{ColumnInfo, DataSourceId, DriverKind, Row, TransactionStatus};
use dbdesk_engine::{
    Command, ConsoleId, Event, RunId, RunOutcome, SessionState, StatementFailure, StatementSummary,
    TransactionEnd, TransactionMode,
};
use dbdesk_sql::{Dialect, split_statements, statement_at};
use dbdesk_store::ConsoleKey;

pub use view::{ConsoleAction, show};

pub fn dialect_for(driver: DriverKind) -> Dialect {
    match driver {
        DriverKind::Postgres => Dialect::Postgres,
        DriverKind::Sqlite => Dialect::Sqlite,
    }
}

/// One statement sent to the engine, and where it came from in the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmittedStatement {
    pub sql: String,
    /// Byte range in the console's text, to place error markers.
    pub range: Range<usize>,
}

/// Which statements a run covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunScope {
    /// The selection if there is one, otherwise the statement under the cursor.
    Current,
    /// Every statement in the console.
    All,
}

pub struct QueryConsole {
    /// The engine session's id, new every time the app starts.
    id: ConsoleId,
    /// The console's file, which outlives the app.
    key: ConsoleKey,
    title: String,
    dialect: Dialect,
    text: String,
    /// The editor's cursor or selection as byte offsets into `text`, in either order.
    selection: Range<usize>,
    /// The state of the console's own connection.
    session: SessionState,
    transactions: TransactionMode,
    transaction: TransactionStatus,
    /// How the last Commit or Rollback went, until the next run.
    transaction_ended: Option<Result<TransactionEnd, String>>,
    run: Option<RunView>,
    /// Where the last failure points in `text`; underlined until the text changes.
    error_marker: Option<Range<usize>>,
    /// A byte offset to move the editor's cursor to on the next frame, after the text was
    /// changed from outside the editor.
    pending_cursor: Option<usize>,
    file: FileState,
}

/// How the console's text relates to its file.
#[derive(Debug, Default)]
struct FileState {
    /// When the text was first changed since it was last written; `None` if it is saved.
    unsaved_since: Option<Instant>,
    /// Whether closing the tab keeps the file, so the console can be reopened. False for
    /// consoles nobody typed in, such as a fresh one or one opened by Open data.
    keep: bool,
    /// Why the last save failed, until a later one succeeds.
    save_error: Option<String>,
}

pub struct RunView {
    id: RunId,
    statements: Vec<StatementView>,
    state: RunState,
    /// The statement whose result is shown.
    selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Running,
    Finished {
        outcome: RunOutcome,
        elapsed: Duration,
    },
}

pub struct StatementView {
    sql: String,
    range: Range<usize>,
    columns: Vec<ColumnInfo>,
    rows: Vec<Row>,
    /// More rows can be fetched.
    more: bool,
    /// A fetch is in flight, so the grid doesn't ask twice.
    fetching: bool,
    result: Option<Result<StatementSummary, StatementFailure>>,
}

impl QueryConsole {
    pub fn new(
        id: ConsoleId,
        key: ConsoleKey,
        title: String,
        dialect: Dialect,
        text: String,
    ) -> Self {
        Self {
            id,
            key,
            title,
            dialect,
            text,
            selection: 0..0,
            session: SessionState::Disconnected,
            transactions: TransactionMode::Auto,
            transaction: TransactionStatus::Idle,
            transaction_ended: None,
            run: None,
            error_marker: None,
            pending_cursor: None,
            file: FileState::default(),
        }
    }

    /// A console reopened from its file, which closing it again must keep.
    pub fn reopened(mut self) -> Self {
        self.file.keep = true;
        self
    }

    pub fn id(&self) -> ConsoleId {
        self.id
    }

    pub fn data_source(&self) -> DataSourceId {
        self.key.data_source
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Puts `sql` at the cursor, replacing the selection, as a statement on a line of its own,
    /// and moves the cursor past it.
    pub fn insert_statement(&mut self, sql: &str, now: Instant) {
        let (start, end) = ordered(&self.selection);
        let sql = sql.trim_end();
        let mut inserted = String::new();
        if !self.text[..start].is_empty() && !self.text[..start].ends_with('\n') {
            inserted.push('\n');
        }
        inserted.push_str(sql);
        if !sql.ends_with(';') {
            inserted.push(';');
        }
        inserted.push('\n');
        self.text.replace_range(start..end, &inserted);
        let cursor = start + inserted.len();
        self.selection = cursor..cursor;
        self.pending_cursor = Some(cursor);
        self.edited(now);
    }

    /// Records that the user changed the text at `now`.
    pub fn edited(&mut self, now: Instant) {
        self.error_marker = None;
        self.file.keep = true;
        self.file.unsaved_since.get_or_insert(now);
    }

    /// The save to send if the text has been unsaved for at least `delay` (or at all, for a
    /// zero delay), marking it saved. Otherwise how long until it would be due.
    pub fn take_save(
        &mut self,
        now: Instant,
        delay: Duration,
    ) -> Result<Command, Option<Duration>> {
        let since = self.file.unsaved_since.ok_or(None)?;
        let waited = now.saturating_duration_since(since);
        if waited < delay {
            return Err(Some(delay - waited));
        }
        self.file.unsaved_since = None;
        self.file.save_error = None;
        Ok(Command::SaveConsole {
            key: self.key.clone(),
            text: self.text.clone(),
        })
    }

    /// What to do with the file when the tab closes: write the last edits, or remove a file
    /// nobody typed anything into.
    pub fn file_on_close(&mut self) -> Option<Command> {
        if !self.file.keep || self.text.trim().is_empty() {
            return Some(Command::DeleteConsoleFile(self.key.clone()));
        }
        self.take_save(Instant::now(), Duration::ZERO).ok()
    }

    /// Whether the file is kept once the tab closes.
    pub fn keeps_file(&self) -> bool {
        self.file.keep && !self.text.trim().is_empty()
    }

    pub fn save_error(&self) -> Option<&str> {
        self.file.save_error.as_deref()
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    pub fn transaction_mode(&self) -> TransactionMode {
        self.transactions
    }

    /// Whether the console's connection has uncommitted work that closing it would roll
    /// back.
    pub fn has_open_transaction(&self) -> bool {
        self.transaction != TransactionStatus::Idle
    }

    pub fn is_running(&self) -> bool {
        self.run
            .as_ref()
            .is_some_and(|run| run.state == RunState::Running)
    }

    /// What `scope` would run right now. Empty if there is nothing but whitespace and
    /// comments.
    pub fn statements_to_run(&self, scope: RunScope) -> Vec<SubmittedStatement> {
        let (start, end) = ordered(&self.selection);
        let submit = |offset: usize, ranges: Vec<Range<usize>>, source: &str| {
            ranges
                .into_iter()
                .map(|range| SubmittedStatement {
                    sql: source[range.clone()].to_owned(),
                    range: range.start + offset..range.end + offset,
                })
                .collect::<Vec<_>>()
        };
        match scope {
            RunScope::Current if start != end => {
                let selected = &self.text[start..end];
                submit(start, split_statements(selected, self.dialect), selected)
            }
            RunScope::Current => {
                let ranges = split_statements(&self.text, self.dialect);
                let current = statement_at(&ranges, end).map(|index| ranges[index].clone());
                submit(0, current.into_iter().collect(), &self.text)
            }
            RunScope::All => submit(0, split_statements(&self.text, self.dialect), &self.text),
        }
    }

    pub fn begin_run(&mut self, run: RunId, statements: &[SubmittedStatement]) {
        self.error_marker = None;
        self.transaction_ended = None;
        self.run = Some(RunView {
            id: run,
            statements: statements
                .iter()
                .map(|statement| StatementView {
                    sql: statement.sql.clone(),
                    range: statement.range.clone(),
                    columns: Vec::new(),
                    rows: Vec::new(),
                    more: false,
                    fetching: false,
                    result: None,
                })
                .collect(),
            state: RunState::Running,
            selected: 0,
        });
    }

    /// The run to cancel: one still running, or one whose rows are still open.
    pub fn cancellable_run(&self) -> Option<RunId> {
        let run = self.run.as_ref()?;
        let open_rows = run.statements.iter().any(|statement| statement.more);
        (run.state == RunState::Running || open_rows).then_some(run.id)
    }

    /// Marks the next page as requested. `None` if there is nothing more, or a fetch is
    /// already in flight.
    pub fn request_more(&mut self) -> Option<RunId> {
        let run = self.run.as_mut()?;
        let statement = run
            .statements
            .iter_mut()
            .find(|statement| statement.more && !statement.fetching)?;
        statement.fetching = true;
        Some(run.id)
    }

    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::ConsoleSessionChanged { console, state } if *console == self.id => {
                self.session_changed(state);
            }
            Event::TransactionChanged { console, status } if *console == self.id => {
                self.transaction = *status;
            }
            Event::TransactionEnded { console, result } if *console == self.id => {
                self.transaction_ended = Some(result.clone());
            }
            Event::ConsoleSaveFailed { key, error } if *key == self.key => {
                // Shown until the next save is sent; that one writes the whole text again.
                self.file.save_error = Some(error.clone());
            }
            Event::StatementStarted {
                run,
                statement,
                columns,
            } => {
                if let Some(view) = self.current_run_mut(*run)
                    && let Some(statement_view) = view.statements.get_mut(*statement)
                {
                    statement_view.columns = columns.clone();
                    view.selected = *statement;
                }
            }
            Event::RowsFetched {
                run,
                statement,
                rows,
                more,
            } => {
                if let Some(view) = self.current_run_mut(*run)
                    && let Some(statement_view) = view.statements.get_mut(*statement)
                {
                    statement_view.rows.extend(rows.iter().cloned());
                    statement_view.more = *more;
                    statement_view.fetching = false;
                }
            }
            Event::StatementFinished {
                run,
                statement,
                result,
            } => self.statement_finished(*run, *statement, result),
            Event::RunFinished {
                run,
                outcome,
                elapsed,
            } => {
                if let Some(view) = self.current_run_mut(*run) {
                    view.state = RunState::Finished {
                        outcome: *outcome,
                        elapsed: *elapsed,
                    };
                }
            }
            _ => {}
        }
    }

    /// The latest run, if it is `run`: events for older runs are stale and ignored.
    fn current_run_mut(&mut self, run: RunId) -> Option<&mut RunView> {
        self.run.as_mut().filter(|view| view.id == run)
    }

    fn statement_finished(
        &mut self,
        run: RunId,
        statement: usize,
        result: &Result<StatementSummary, StatementFailure>,
    ) {
        let Some(view) = self.run.as_mut().filter(|view| view.id == run) else {
            return;
        };
        let Some(statement_view) = view.statements.get_mut(statement) else {
            return;
        };
        statement_view.result = Some(result.clone());
        statement_view.more = false;
        statement_view.fetching = false;
        if let Err(StatementFailure {
            position: Some(position),
            ..
        }) = result
        {
            let offset = statement_view.range.start + position;
            self.error_marker = Some(highlight::error_marker(&self.text, self.dialect, offset));
        }
        if result.is_err() {
            view.selected = statement;
        }
    }

    /// A connection that fails or closes ends whatever the console was doing with it.
    fn session_changed(&mut self, state: &SessionState) {
        self.session = state.clone();
        if matches!(state, SessionState::Disconnected | SessionState::Failed(_)) {
            // The database rolls back whatever a closed connection left uncommitted.
            self.transaction = TransactionStatus::Idle;
        }
        let failure = match state {
            SessionState::Failed(error) => error.clone(),
            SessionState::Disconnected => "the connection closed".to_owned(),
            SessionState::Connecting | SessionState::Connected(_) => return,
        };
        let Some(view) = self.run.as_mut() else {
            return;
        };
        for statement in &mut view.statements {
            statement.more = false;
            statement.fetching = false;
        }
        if view.state == RunState::Running {
            if let Some(pending) = view
                .statements
                .iter_mut()
                .find(|statement| statement.result.is_none())
            {
                pending.result = Some(Err(StatementFailure {
                    message: failure,
                    position: None,
                }));
            }
            view.state = RunState::Finished {
                outcome: RunOutcome::Failed,
                elapsed: Duration::ZERO,
            };
        }
    }
}

/// A selection's ends in text order.
fn ordered(range: &Range<usize>) -> (usize, usize) {
    (range.start.min(range.end), range.start.max(range.end))
}

#[cfg(test)]
mod tests;
