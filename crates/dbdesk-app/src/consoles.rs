//! The open query consoles, their files, and the rules for starting runs in them. Returns the
//! engine commands to send rather than sending them, so it can be tested without an engine.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use dbdesk_core::{CatalogPath, DataSource, DataSourceId};
use dbdesk_engine::{Command, ConsoleId, Event, RunId};
use dbdesk_sql::FormatOptions;
use dbdesk_store::{ConsoleKey, ConsoleName, ExportFormat};

use crate::console::{
    self, ConsoleAction, FormatScope, QueryConsole, RecentCompletions, RunScope, SubmittedStatement,
};
use crate::hazard_dialog::{self, HazardChoice, HazardConfirmation, Review};
use crate::rollback_dialog::Closing;

/// How long a console's text stays unsaved after an edit. Long enough that typing doesn't
/// write a file per keystroke, short enough that a crash loses at most a second of typing.
pub const SAVE_DELAY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct Consoles {
    open: BTreeMap<ConsoleKey, QueryConsole>,
    /// Completions accepted in any console, which rank first in all of them.
    recent_completions: RecentCompletions,
    saved: SavedConsoles,
    next_console: u64,
    next_run: u64,
    hazard: Option<HazardConfirmation>,
}

/// Console files that aren't open in a tab, with their text, so they can be reopened.
#[derive(Debug, Default)]
pub struct SavedConsoles {
    files: BTreeMap<ConsoleKey, String>,
}

impl SavedConsoles {
    /// The names of `data_source`'s closed consoles, in order.
    pub fn names(&self, data_source: DataSourceId) -> impl Iterator<Item = &ConsoleName> {
        self.files
            .keys()
            .filter(move |key| key.data_source == data_source)
            .map(|key| &key.name)
    }
}

impl Consoles {
    /// The open consoles to draw, the saved ones the explorer offers, and the recent
    /// completions, borrowed at once.
    pub fn open_and_saved_mut(
        &mut self,
    ) -> (
        &mut BTreeMap<ConsoleKey, QueryConsole>,
        &SavedConsoles,
        &RecentCompletions,
    ) {
        (&mut self.open, &self.saved, &self.recent_completions)
    }

    #[cfg(test)]
    pub fn saved(&self) -> &SavedConsoles {
        &self.saved
    }

    #[cfg(test)]
    pub fn open_consoles(&self) -> impl Iterator<Item = &QueryConsole> {
        self.open.values()
    }

    /// Opens a new console on `data_source`, holding `text`, in a file of its own.
    pub fn open(&mut self, data_source: &DataSource, text: String) -> ConsoleKey {
        let key = ConsoleKey {
            data_source: data_source.id,
            name: self.free_name(data_source.id),
        };
        let console = self.new_console(data_source, key.clone(), text);
        self.open.insert(key.clone(), console);
        key
    }

    /// Reopens a saved console. `None` if there is no such file.
    pub fn reopen(&mut self, data_source: &DataSource, key: &ConsoleKey) -> Option<ConsoleKey> {
        let text = self.saved.files.remove(key)?;
        let console = self.new_console(data_source, key.clone(), text).reopened();
        self.open.insert(key.clone(), console);
        Some(key.clone())
    }

    /// The lowest numbered name no console of the data source uses, open or saved.
    fn free_name(&self, data_source: DataSourceId) -> ConsoleName {
        let in_use = |name: &ConsoleName| {
            let key = ConsoleKey {
                data_source,
                name: name.clone(),
            };
            self.open.contains_key(&key) || self.saved.files.contains_key(&key)
        };
        (1..)
            .map(ConsoleName::numbered)
            .find(|name| !in_use(name))
            .unwrap_or_else(|| ConsoleName::numbered(0))
    }

    fn new_console(
        &mut self,
        data_source: &DataSource,
        key: ConsoleKey,
        text: String,
    ) -> QueryConsole {
        self.next_console += 1;
        let title = title(&data_source.name, &key.name);
        let dialect = console::dialect_for(data_source.config.driver());
        QueryConsole::new(ConsoleId(self.next_console), key, title, dialect, text)
    }

    /// Opens a console that selects everything from the table or view at `path`, and runs it.
    pub fn open_data(
        &mut self,
        data_source: &DataSource,
        path: &CatalogPath,
    ) -> Option<(ConsoleKey, Command)> {
        let name = path.qualified_name()?;
        let key = self.open(data_source, format!("SELECT * FROM {name};\n"));
        let statements = self.open.get(&key)?.statements_to_run(RunScope::All);
        let command = self.start_run(&key, statements)?;
        Some((key, command))
    }

    /// Closes the console's tab and connection. Its file is kept, with the latest text, if
    /// the user typed in it, and removed otherwise.
    pub fn close(&mut self, key: &ConsoleKey) -> Vec<Command> {
        let Some(mut console) = self.open.remove(key) else {
            return Vec::new();
        };
        let mut commands = vec![Command::CloseConsole(console.id())];
        commands.extend(console.file_on_close());
        if console.keeps_file() {
            self.saved
                .files
                .insert(key.clone(), console.text().to_owned());
        }
        commands
    }

    /// Saves for the consoles whose edits are due, and when to look again.
    pub fn due_saves(&mut self, now: Instant) -> (Vec<Command>, Option<Duration>) {
        let mut next_due: Option<Duration> = None;
        let mut saves = Vec::new();
        for console in self.open.values_mut() {
            match console.take_save(now, SAVE_DELAY) {
                Ok(save) => saves.push(save),
                Err(Some(wait)) => next_due = Some(next_due.map_or(wait, |next| next.min(wait))),
                Err(None) => {}
            }
        }
        (saves, next_due)
    }

    /// Saves for every console with unsaved edits, for when the app quits.
    pub fn unsaved(&mut self) -> Vec<Command> {
        self.open
            .values_mut()
            .filter_map(|console| console.take_save(Instant::now(), Duration::ZERO).ok())
            .collect()
    }

    /// The titles of the consoles with an open transaction that `closing` would roll back.
    pub fn rolled_back_by(&self, closing: &Closing) -> Vec<String> {
        self.open
            .iter()
            .filter(|(key, console)| closing.affects(key) && console.has_open_transaction())
            .map(|(_, console)| console.title().to_owned())
            .collect()
    }

    /// Writes the rows the console's grid shows to `path`.
    pub fn export(&self, key: &ConsoleKey, format: ExportFormat, path: PathBuf) -> Option<Command> {
        self.open.get(key)?.export(format, path)
    }

    /// Formats SQL in the console, if it is still open.
    pub fn format(&mut self, key: &ConsoleKey, scope: FormatScope, options: &FormatOptions) {
        if let Some(console) = self.open.get_mut(key) {
            console.format(scope, options, Instant::now());
        }
    }

    /// Puts a statement at the console's cursor, if the console is still open.
    pub fn insert_statement(&mut self, key: &ConsoleKey, sql: &str) {
        if let Some(console) = self.open.get_mut(key) {
            console.insert_statement(sql, Instant::now());
        }
    }

    pub fn apply(&mut self, event: &Event) {
        if let Event::ConsolesLoaded(files) = event {
            self.saved.files.extend(
                files
                    .iter()
                    .filter(|file| !self.open.contains_key(&file.key))
                    .map(|file| (file.key.clone(), file.text.clone())),
            );
        }
        for console in self.open.values_mut() {
            console.apply(event);
        }
    }

    pub fn handle(&mut self, key: &ConsoleKey, action: ConsoleAction) -> Option<Command> {
        let id = self.open.get(key)?.id();
        match action {
            ConsoleAction::Run(statements) => self.submit(key, statements),
            ConsoleAction::FetchMore(run) => Some(Command::FetchMore { console: id, run }),
            ConsoleAction::Cancel(run) => Some(Command::Cancel { console: id, run }),
            ConsoleAction::EndTransaction(end) => {
                Some(Command::EndTransaction { console: id, end })
            }
            ConsoleAction::DescribeTable(table) => Some(Command::DescribeTable {
                data_source: self.open.get(key)?.data_source(),
                table,
            }),
            ConsoleAction::SubmitEdits => self.open.get_mut(key)?.submit_edits(),
            ConsoleAction::Validate(statements) => Some(Command::ValidateStatements {
                console: id,
                statements,
            }),
            ConsoleAction::CompletionAccepted(candidate) => {
                self.recent_completions.record(&candidate);
                None
            }
            // The app opens a window or a file dialog, or updates the catalog, for these.
            ConsoleAction::ShowHistory
            | ConsoleAction::Export(_)
            | ConsoleAction::LoadCatalog(_)
            | ConsoleAction::Format(_) => None,
        }
    }

    /// Runs the statements, unless one would change every row of a table: then the
    /// confirmation is shown first and nothing runs until the user answers.
    fn submit(&mut self, key: &ConsoleKey, statements: Vec<SubmittedStatement>) -> Option<Command> {
        let dialect = self.open.get(key)?.dialect();
        match hazard_dialog::review(key.clone(), statements, dialect) {
            Review::Safe(statements) => self.start_run(key, statements),
            Review::NeedsConfirmation(confirmation) => {
                self.hazard = Some(confirmation);
                None
            }
        }
    }

    fn start_run(
        &mut self,
        key: &ConsoleKey,
        statements: Vec<SubmittedStatement>,
    ) -> Option<Command> {
        let console = self.open.get_mut(key)?;
        self.next_run += 1;
        let run = RunId(self.next_run);
        console.begin_run(run, &statements);
        Some(Command::Execute {
            console: console.id(),
            data_source: console.data_source(),
            run,
            transactions: console.transaction_mode(),
            statements: statements
                .into_iter()
                .map(|statement| statement.sql)
                .collect(),
        })
    }

    pub fn show_hazard_confirmation(&mut self, ctx: &egui::Context) -> Option<Command> {
        let choice = self.hazard.as_ref()?.show(ctx)?;
        let confirmation = self.hazard.take()?;
        match choice {
            HazardChoice::RunAnyway => {
                self.start_run(&confirmation.console, confirmation.statements)
            }
            HazardChoice::Cancel => None,
        }
    }
}

/// A console's tab title: the data source's name, numbered from the second console on, as
/// `shop`, `shop (2)`; or with the file's name if the user named it.
fn title(data_source: &str, name: &ConsoleName) -> String {
    match name.number() {
        Some(1) => data_source.to_owned(),
        Some(n) => format!("{data_source} ({n})"),
        None => format!("{data_source}: {name}"),
    }
}

#[cfg(test)]
mod tests {
    use dbdesk_core::{ConnectionConfig, SqliteConfig};
    use dbdesk_store::SavedConsole;

    use super::*;

    fn data_source(name: &str) -> DataSource {
        DataSource {
            id: DataSourceId::generate(),
            name: name.to_owned(),
            color: None,
            config: ConnectionConfig::Sqlite(SqliteConfig {
                path: "/data/app.db".into(),
                read_only: false,
            }),
        }
    }

    fn run(statements: &[&str]) -> ConsoleAction {
        ConsoleAction::Run(
            statements
                .iter()
                .map(|sql| SubmittedStatement {
                    sql: (*sql).to_owned(),
                    range: 0..sql.len(),
                    prefix_len: 0,
                })
                .collect(),
        )
    }

    fn named(data_source: &DataSource, name: &str) -> ConsoleKey {
        ConsoleKey {
            data_source: data_source.id,
            name: ConsoleName::try_from(name.to_owned()).unwrap(),
        }
    }

    #[test]
    fn numbers_titles_and_runs_safe_statements_at_once() {
        let mut consoles = Consoles::default();
        let source = data_source("shop");
        let first = consoles.open(&source, String::new());
        let second = consoles.open(&source, String::new());

        assert_eq!(consoles.open[&first].title(), "shop");
        assert_eq!(consoles.open[&second].title(), "shop (2)");
        assert_eq!(second.name.as_str(), "console_2");
        assert!(matches!(
            consoles.handle(&first, run(&["SELECT 1"])),
            Some(Command::Execute { statements, .. }) if statements == ["SELECT 1"]
        ));
    }

    #[test]
    fn holds_back_unfiltered_deletes_until_confirmed() {
        let mut consoles = Consoles::default();
        let key = consoles.open(&data_source("shop"), String::new());

        assert!(
            consoles
                .handle(&key, run(&["SELECT 1", "DELETE FROM orders"]))
                .is_none()
        );

        assert!(consoles.hazard.is_some());
        assert!(!consoles.open[&key].is_running());
    }

    #[test]
    fn opens_table_data_with_a_select() {
        let mut consoles = Consoles::default();
        let path = CatalogPath::root()
            .child(dbdesk_core::NodeKind::Schema, "main")
            .child(dbdesk_core::NodeKind::Table, "Order Items");

        let (_, command) = consoles.open_data(&data_source("shop"), &path).unwrap();

        assert!(matches!(
            command,
            Command::Execute { statements, .. } if statements == ["SELECT * FROM main.\"Order Items\""]
        ));
    }

    #[test]
    fn saves_edits_once_they_settle() {
        let mut consoles = Consoles::default();
        let key = consoles.open(&data_source("shop"), "select 1".to_owned());
        let start = Instant::now();
        assert_eq!(consoles.due_saves(start).0.len(), 0, "nothing was edited");

        consoles.open.get_mut(&key).unwrap().edited(start);
        let (saves, next) = consoles.due_saves(start + SAVE_DELAY / 2);
        assert!(saves.is_empty());
        assert_eq!(next, Some(SAVE_DELAY / 2));

        let (saves, next) = consoles.due_saves(start + SAVE_DELAY);
        assert!(matches!(
            saves.as_slice(),
            [Command::SaveConsole { key: saved, text }] if *saved == key && text == "select 1"
        ));
        assert_eq!(next, None);
        assert!(consoles.unsaved().is_empty());
    }

    #[test]
    fn keeps_the_files_of_consoles_the_user_typed_in() {
        let mut consoles = Consoles::default();
        let source = data_source("shop");
        let untouched = consoles.open(&source, "SELECT * FROM t;".to_owned());
        let typed_in = consoles.open(&source, "select 2".to_owned());
        consoles
            .open
            .get_mut(&typed_in)
            .unwrap()
            .edited(Instant::now());

        let closed = consoles.close(&untouched);
        assert!(matches!(
            closed.as_slice(),
            [Command::CloseConsole(_), Command::DeleteConsoleFile(key)] if *key == untouched
        ));
        let closed = consoles.close(&typed_in);
        assert!(matches!(
            closed.as_slice(),
            [Command::CloseConsole(_), Command::SaveConsole { key, .. }] if *key == typed_in
        ));

        assert_eq!(
            consoles.saved().names(source.id).collect::<Vec<_>>(),
            [&typed_in.name]
        );
        assert_eq!(consoles.reopen(&source, &typed_in), Some(typed_in.clone()));
        assert_eq!(consoles.open[&typed_in].text(), "select 2");
        assert_eq!(consoles.saved().names(source.id).count(), 0);
    }

    #[test]
    fn new_consoles_skip_the_names_of_saved_files() {
        let mut consoles = Consoles::default();
        let source = data_source("shop");
        consoles.apply(&Event::ConsolesLoaded(vec![SavedConsole {
            key: named(&source, "console"),
            text: "select 1".to_owned(),
        }]));

        let key = consoles.open(&source, String::new());

        assert_eq!(key, named(&source, "console_2"));
    }

    #[test]
    fn titles_name_the_data_source_and_the_console() {
        let cases = [
            ("console", "shop"),
            ("console_3", "shop (3)"),
            ("report", "shop: report"),
        ];
        for (name, expected) in cases {
            let name = ConsoleName::try_from(name.to_owned()).unwrap();
            assert_eq!(title("shop", &name), expected);
        }
    }
}
