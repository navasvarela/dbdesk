//! The open query consoles and the rules for starting runs in them. Returns the engine
//! commands to send rather than sending them, so it can be tested without an engine.

use std::collections::{BTreeMap, HashMap};

use dbdesk_core::{CatalogPath, DataSourceId};
use dbdesk_engine::{Command, ConsoleId, Event, RunId};

use crate::console::{self, ConsoleAction, QueryConsole, RunScope, SubmittedStatement};
use crate::hazard_dialog::{self, HazardChoice, HazardConfirmation, Review};
use crate::state::DataSourceEntry;

#[derive(Default)]
pub struct Consoles {
    open: BTreeMap<ConsoleId, QueryConsole>,
    /// Consoles opened per data source so far, to number their titles.
    opened_per_source: HashMap<DataSourceId, usize>,
    next_console: u64,
    next_run: u64,
    hazard: Option<HazardConfirmation>,
}

impl Consoles {
    pub fn open_mut(&mut self) -> &mut BTreeMap<ConsoleId, QueryConsole> {
        &mut self.open
    }

    /// Opens a console on `entry`'s data source, holding `text`.
    pub fn open(&mut self, entry: &DataSourceEntry, text: String) -> ConsoleId {
        self.next_console += 1;
        let id = ConsoleId(self.next_console);
        let data_source = &entry.data_source;
        let count = self.opened_per_source.entry(data_source.id).or_default();
        *count += 1;
        let title = match *count {
            1 => data_source.name.clone(),
            n => format!("{} ({n})", data_source.name),
        };
        let dialect = console::dialect_for(data_source.config.driver());
        self.open.insert(
            id,
            QueryConsole::new(id, data_source.id, title, dialect, text),
        );
        id
    }

    /// Opens a console that selects everything from the table or view at `path`, and runs it.
    pub fn open_data(
        &mut self,
        entry: &DataSourceEntry,
        path: &CatalogPath,
    ) -> Option<(ConsoleId, Command)> {
        let name = path.qualified_name()?;
        let id = self.open(entry, format!("SELECT * FROM {name};\n"));
        let statements = self.open.get(&id)?.statements_to_run(RunScope::All);
        let command = self.start_run(id, statements)?;
        Some((id, command))
    }

    pub fn close(&mut self, id: ConsoleId) -> Command {
        self.open.remove(&id);
        Command::CloseConsole(id)
    }

    pub fn apply(&mut self, event: &Event) {
        for console in self.open.values_mut() {
            console.apply(event);
        }
    }

    pub fn handle(&mut self, id: ConsoleId, action: ConsoleAction) -> Option<Command> {
        match action {
            ConsoleAction::Run(statements) => self.submit(id, statements),
            ConsoleAction::FetchMore(run) => Some(Command::FetchMore { console: id, run }),
            ConsoleAction::Cancel(run) => Some(Command::Cancel { console: id, run }),
        }
    }

    /// Runs the statements, unless one would change every row of a table: then the
    /// confirmation is shown first and nothing runs until the user answers.
    fn submit(&mut self, id: ConsoleId, statements: Vec<SubmittedStatement>) -> Option<Command> {
        let dialect = self.open.get(&id)?.dialect();
        match hazard_dialog::review(id, statements, dialect) {
            Review::Safe(statements) => self.start_run(id, statements),
            Review::NeedsConfirmation(confirmation) => {
                self.hazard = Some(confirmation);
                None
            }
        }
    }

    fn start_run(&mut self, id: ConsoleId, statements: Vec<SubmittedStatement>) -> Option<Command> {
        let console = self.open.get_mut(&id)?;
        self.next_run += 1;
        let run = RunId(self.next_run);
        console.begin_run(run, &statements);
        Some(Command::Execute {
            console: id,
            data_source: console.data_source(),
            run,
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
                self.start_run(confirmation.console, confirmation.statements)
            }
            HazardChoice::Cancel => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use dbdesk_core::{ConnectionConfig, DataSource, SqliteConfig};
    use dbdesk_engine::SessionState;

    use super::*;
    use crate::catalog_tree::CatalogTree;

    fn entry(name: &str) -> DataSourceEntry {
        DataSourceEntry {
            data_source: DataSource {
                id: DataSourceId::generate(),
                name: name.to_owned(),
                config: ConnectionConfig::Sqlite(SqliteConfig {
                    path: "/data/app.db".into(),
                    read_only: false,
                }),
            },
            session: SessionState::Disconnected,
            catalog: CatalogTree::default(),
        }
    }

    fn run(statements: &[&str]) -> ConsoleAction {
        ConsoleAction::Run(
            statements
                .iter()
                .map(|sql| SubmittedStatement {
                    sql: (*sql).to_owned(),
                    range: 0..sql.len(),
                })
                .collect(),
        )
    }

    #[test]
    fn numbers_titles_and_runs_safe_statements_at_once() {
        let mut consoles = Consoles::default();
        let source = entry("shop");
        let first = consoles.open(&source, String::new());
        let second = consoles.open(&source, String::new());

        assert_eq!(consoles.open[&first].title(), "shop");
        assert_eq!(consoles.open[&second].title(), "shop (2)");
        assert!(matches!(
            consoles.handle(first, run(&["SELECT 1"])),
            Some(Command::Execute { statements, .. }) if statements == ["SELECT 1"]
        ));
    }

    #[test]
    fn holds_back_unfiltered_deletes_until_confirmed() {
        let mut consoles = Consoles::default();
        let id = consoles.open(&entry("shop"), String::new());

        assert!(
            consoles
                .handle(id, run(&["SELECT 1", "DELETE FROM orders"]))
                .is_none()
        );

        assert!(consoles.hazard.is_some());
        assert!(!consoles.open[&id].is_running());
    }

    #[test]
    fn opens_table_data_with_a_select() {
        let mut consoles = Consoles::default();
        let path = CatalogPath::root()
            .child(dbdesk_core::NodeKind::Schema, "main")
            .child(dbdesk_core::NodeKind::Table, "Order Items");

        let (_, command) = consoles.open_data(&entry("shop"), &path).unwrap();

        assert!(matches!(
            command,
            Command::Execute { statements, .. } if statements == ["SELECT * FROM main.\"Order Items\""]
        ));
    }
}
