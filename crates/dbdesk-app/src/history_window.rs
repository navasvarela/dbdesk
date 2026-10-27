//! The query history of one data source, searchable and filtered by where statements came
//! from, opened from a console to put a past statement back into it. Each statement is marked
//! with where it came from: a console, the grid, an object helper, an import, an export, or an
//! MCP client.

use dbdesk_core::DataSourceId;
use dbdesk_engine::{Command, Event};
use dbdesk_store::{
    ConsoleKey, HistoryEntry, HistoryOutcome, HistoryQuery, HistorySource, HistorySourceKind,
};
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
    kind: Option<HistorySourceKind>,
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
            kind: None,
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
            kind: self.kind,
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
                let filtered = kind_filter(ui, &mut self.kind);
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

const ALL_SOURCES: &str = "All sources";

/// What the filter and the badges call each kind of source.
fn kind_label(kind: HistorySourceKind) -> &'static str {
    match kind {
        HistorySourceKind::Console => "Consoles",
        HistorySourceKind::GridEdit => "Grid edits",
        HistorySourceKind::ObjectHelper => "Object helpers",
        HistorySourceKind::Import => "Imports",
        HistorySourceKind::Export => "Exports",
        HistorySourceKind::Mcp => "MCP clients",
        HistorySourceKind::Dbdesk => "Earlier dbdesk",
    }
}

/// Returns whether the choice changed.
fn kind_filter(ui: &mut Ui, kind: &mut Option<HistorySourceKind>) -> bool {
    let selected = kind.map_or(ALL_SOURCES, kind_label);
    let mut changed = false;
    ComboBox::from_id_salt("history_source")
        .selected_text(selected)
        .show_ui(ui, |ui| {
            changed |= ui.selectable_value(kind, None, ALL_SOURCES).changed();
            for choice in HistorySourceKind::ALL {
                changed |= ui
                    .selectable_value(kind, Some(choice), kind_label(choice))
                    .changed();
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
        source_badge(ui, &entry.source);
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

/// A short tag naming where the statement came from, with the details on hover. Statements
/// from MCP clients stand out, since nobody at the keyboard ran them.
fn source_badge(ui: &mut Ui, source: &HistorySource) {
    let palette = Palette::for_visuals(ui.visuals());
    let (text, color, hover) = match source {
        HistorySource::Console => (
            "Console",
            palette.text_weak,
            "Run in a query console".to_owned(),
        ),
        HistorySource::GridEdit => (
            "Grid",
            palette.text_weak,
            "Submitted from the grid".to_owned(),
        ),
        HistorySource::ObjectHelper => (
            "Object",
            palette.text_weak,
            "Run by an object helper in the explorer".to_owned(),
        ),
        HistorySource::Import { file } => (
            "Import",
            palette.text_weak,
            format!("Imported from {}", file.display()),
        ),
        HistorySource::Export => (
            "Export",
            palette.text_weak,
            "Read by Export \u{203a} All rows".to_owned(),
        ),
        HistorySource::Mcp { client } => (
            "MCP",
            palette.secondary,
            match client {
                Some(client) => format!("Run by the MCP client {client}, through dbdesk mcp"),
                None => "Run by an MCP client, through dbdesk mcp".to_owned(),
            },
        ),
        HistorySource::Dbdesk => (
            "dbdesk",
            palette.text_weak,
            "Run by a dbdesk that didn't record where".to_owned(),
        ),
    };
    ui.label(RichText::new(text).small().strong().color(color))
        .on_hover_text(hover);
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
