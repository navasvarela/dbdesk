//! The query history of one data source, searchable and filtered by where statements came
//! from, opened from a console to put a past statement back into it. Statements an MCP
//! client ran are marked.

use dbdesk_core::DataSourceId;
use dbdesk_engine::{Command, Event};
use dbdesk_store::{ConsoleKey, HistoryEntry, HistoryOutcome, HistoryQuery, HistorySource};
use egui::{ComboBox, Label, RichText, ScrollArea, TextEdit, Ui, Window};
use jiff::tz::TimeZone;

use crate::theme::Palette;

const DEFAULT_SIZE: [f32; 2] = [640.0, 420.0];
const TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

pub struct HistoryWindow {
    /// The console that statements are inserted into.
    console: ConsoleKey,
    data_source_name: String,
    search: String,
    /// Only statements from there; `None` for all.
    source: Option<HistorySource>,
    /// The answer to the latest search; `None` while it is pending.
    results: Option<Result<Vec<HistoryEntry>, String>>,
}

/// What the user did in the window this frame.
pub enum HistoryAction {
    /// The search changed or needs refreshing; run this one.
    Search(HistoryQuery),
    /// Put this statement into the window's console.
    Insert {
        console: ConsoleKey,
        sql: String,
    },
    Close,
}

impl HistoryWindow {
    /// Opens the window for `console`, with the search for every recent statement to send.
    pub fn open(console: ConsoleKey, data_source_name: String) -> (Self, Command) {
        let window = Self {
            console,
            data_source_name,
            search: String::new(),
            source: None,
            results: None,
        };
        let search = window.search_command();
        (window, search)
    }

    pub fn data_source(&self) -> DataSourceId {
        self.console.data_source
    }

    fn query(&self) -> HistoryQuery {
        HistoryQuery {
            data_source: self.data_source(),
            text: self.search.clone(),
            source: self.source,
        }
    }

    /// The search for the current text and source, to rerun when new statements may have
    /// been recorded.
    pub fn search_command(&self) -> Command {
        Command::SearchHistory(self.query())
    }

    pub fn apply(&mut self, event: &Event) {
        if let Event::HistoryFound { query, result } = event
            && *query == self.query()
        {
            self.results = Some(result.clone());
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<HistoryAction> {
        let mut open = true;
        let action = Window::new(format!("History of {}", self.data_source_name))
            .id(egui::Id::new("history_window"))
            .open(&mut open)
            .default_size(DEFAULT_SIZE)
            .show(ctx, |ui| self.contents(ui))
            .and_then(|response| response.inner.flatten());
        if open {
            action
        } else {
            Some(HistoryAction::Close)
        }
    }

    fn contents(&mut self, ui: &mut Ui) -> Option<HistoryAction> {
        let searched = ui
            .horizontal(|ui| {
                let filtered = source_filter(ui, &mut self.source);
                let typed = ui
                    .add(
                        TextEdit::singleline(&mut self.search)
                            .hint_text("Search statements")
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
                filtered || typed
            })
            .inner;
        if searched {
            self.results = None;
            return Some(HistoryAction::Search(self.query()));
        }
        ui.weak("Double-click a statement, or click Insert, to put it into the console.");
        ui.separator();
        match &self.results {
            None => {
                ui.spinner();
                None
            }
            Some(Err(error)) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
                None
            }
            Some(Ok(entries)) if entries.is_empty() => {
                ui.weak("Nothing found.");
                None
            }
            Some(Ok(entries)) => self.entries(ui, entries),
        }
    }

    fn entries(&self, ui: &mut Ui, entries: &[HistoryEntry]) -> Option<HistoryAction> {
        let row_height = ui.spacing().interact_size.y;
        let time_zone = TimeZone::system();
        let mut chosen = None;
        ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            row_height,
            entries.len(),
            |ui, rows| {
                for entry in &entries[rows] {
                    if entry_row(ui, entry, &time_zone) {
                        chosen = Some(HistoryAction::Insert {
                            console: self.console.clone(),
                            sql: entry.sql.clone(),
                        });
                    }
                }
            },
        );
        chosen
    }
}

/// The choices of where statements came from, each with its label.
const SOURCES: [(Option<HistorySource>, &str); 3] = [
    (None, "All sources"),
    (Some(HistorySource::App), "dbdesk"),
    (Some(HistorySource::Mcp), "MCP clients"),
];

/// Returns whether the choice changed.
fn source_filter(ui: &mut Ui, source: &mut Option<HistorySource>) -> bool {
    let selected = SOURCES
        .iter()
        .find(|(choice, _)| choice == source)
        .map_or("All sources", |(_, label)| label);
    let mut changed = false;
    ComboBox::from_id_salt("history_source")
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (choice, label) in SOURCES {
                changed |= ui.selectable_value(source, choice, label).changed();
            }
        })
        .response
        .on_hover_text("Where the statements were run from");
    changed
}

/// Draws one entry. Returns whether the user chose to insert it.
fn entry_row(ui: &mut Ui, entry: &HistoryEntry, time_zone: &TimeZone) -> bool {
    ui.horizontal(|ui| {
        let insert = ui.small_button("Insert").clicked();
        let time = entry.executed_at.to_zoned(time_zone.clone());
        ui.weak(time.strftime(TIME_FORMAT).to_string());
        outcome_label(ui, entry);
        if entry.source == HistorySource::Mcp {
            let palette = Palette::for_visuals(ui.visuals());
            ui.label(
                RichText::new("MCP")
                    .small()
                    .strong()
                    .color(palette.secondary),
            )
            .on_hover_text("Run by an MCP client, through dbdesk mcp");
        }
        let first_line = entry.sql.lines().next().unwrap_or_default();
        let statement = ui
            .add(
                Label::new(RichText::new(first_line).monospace())
                    .truncate()
                    .sense(egui::Sense::click()),
            )
            .on_hover_text(&entry.sql);
        insert || statement.double_clicked()
    })
    .inner
}

fn outcome_label(ui: &mut Ui, entry: &HistoryEntry) {
    let palette = Palette::for_visuals(ui.visuals());
    let (text, color) = match entry.outcome {
        HistoryOutcome::Ok => ("ok", palette.success),
        HistoryOutcome::Error => ("error", palette.error),
        HistoryOutcome::Cancelled => ("cancelled", palette.warning),
    };
    let details = match (entry.duration, entry.rows) {
        (Some(duration), Some(rows)) => format!("{} ms · {rows} rows", duration.as_millis()),
        (Some(duration), None) => format!("{} ms", duration.as_millis()),
        (None, Some(rows)) => format!("{rows} rows"),
        (None, None) => String::new(),
    };
    ui.colored_label(color, text).on_hover_text(details);
}
