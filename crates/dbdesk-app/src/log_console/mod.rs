//! The log console panel and the `tracing` pipeline that feeds it.
//! See `docs/features/log-console.md`.

mod entry;
mod filter;
mod pipeline;

use std::collections::VecDeque;

use egui::text::LayoutJob;
use egui::{Color32, ScrollArea, TextEdit, TextFormat, TextStyle, Visuals};
use tracing::Level;

pub use entry::LogEntry;
use filter::LogFilter;
#[cfg(test)]
pub use pipeline::LogConsoleLayer;
pub use pipeline::{LogFeed, pipeline};

/// Entries kept in memory before the oldest are dropped. Large enough for a long working
/// session, small enough that filtering every entry each frame stays well under a millisecond.
const MAX_ENTRIES: usize = 10_000;

const LEVELS: [Level; 5] = [
    Level::ERROR,
    Level::WARN,
    Level::INFO,
    Level::DEBUG,
    Level::TRACE,
];

const TIME_FORMAT: &str = "%H:%M:%S%.3f";
const SEARCH_BOX_WIDTH: f32 = 200.0;

pub struct LogConsole {
    feed: LogFeed,
    entries: VecDeque<LogEntry>,
    filter: LogFilter,
}

impl LogConsole {
    pub fn new(feed: LogFeed, ctx: &egui::Context) -> Self {
        feed.waker.install(ctx);
        Self {
            feed,
            entries: VecDeque::new(),
            filter: LogFilter::default(),
        }
    }

    /// Moves newly logged entries into the console. Returns whether any arrived.
    pub fn drain(&mut self) -> bool {
        let mut received = false;
        while let Ok(entry) = self.feed.entries.try_recv() {
            if self.entries.len() == MAX_ENTRIES {
                self.entries.pop_front();
            }
            self.entries.push_back(entry);
            received = true;
        }
        received
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.toolbar(ui);
        ui.separator();
        self.entry_list(ui);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        crate::widgets::toolbar(ui, |ui| {
            egui::ComboBox::from_id_salt("log_console_verbosity")
                .selected_text(level_name(self.filter.verbosity))
                .show_ui(ui, |ui| {
                    for level in LEVELS {
                        ui.selectable_value(&mut self.filter.verbosity, level, level_name(level));
                    }
                });
            ui.add_space(6.0);
            ui.label(
                crate::widgets::glyph(crate::icons::MAGNIFYING_GLASS)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add(
                TextEdit::singleline(&mut self.filter.search)
                    .hint_text("Search")
                    .desired_width(SEARCH_BOX_WIDTH),
            );
            if crate::widgets::IconButton::new(crate::icons::TRASH, "Clear")
                .show(ui)
                .clicked()
            {
                self.entries.clear();
            }
        });
    }

    fn entry_list(&self, ui: &mut egui::Ui) {
        let visible: Vec<&LogEntry> = self
            .entries
            .iter()
            .filter(|entry| self.filter.matches(entry))
            .collect();
        let row_height = ui.text_style_height(&TextStyle::Monospace);

        ScrollArea::both()
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show_rows(ui, row_height, visible.len(), |ui, rows| {
                for entry in &visible[rows] {
                    ui.label(entry_line(entry, ui.style()));
                }
            });
    }
}

/// Lays an entry out as one line, so every row has exactly the monospace text height that
/// `show_rows` assumes when it decides which rows are visible.
fn entry_line(entry: &LogEntry, style: &egui::Style) -> LayoutJob {
    let font_id = TextStyle::Monospace.resolve(style);
    let visuals = &style.visuals;
    let weak = visuals.weak_text_color();
    let format = |color: Color32| TextFormat::simple(font_id.clone(), color);
    let gap = font_id.size;

    let mut job = LayoutJob::default();
    job.wrap.max_rows = 1;
    job.append(
        &entry.time.strftime(TIME_FORMAT).to_string(),
        0.0,
        format(weak),
    );
    job.append(
        &format!("{:<5}", entry.level),
        gap,
        format(level_color(entry.level, visuals)),
    );
    job.append(&entry.target, gap, format(weak));
    job.append(&entry.message, gap, format(visuals.text_color()));
    if !entry.fields.is_empty() {
        job.append(&entry.fields, gap, format(weak));
    }
    job
}

fn level_color(level: Level, visuals: &Visuals) -> Color32 {
    match level {
        Level::ERROR => visuals.error_fg_color,
        Level::WARN => visuals.warn_fg_color,
        Level::INFO => visuals.text_color(),
        _ => visuals.weak_text_color(),
    }
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::ERROR => "Error",
        Level::WARN => "Warn",
        Level::INFO => "Info",
        Level::DEBUG => "Debug",
        _ => "Trace",
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;

    #[test]
    fn shows_logged_entries_and_hides_filtered_ones() {
        let (layer, feed) = pipeline();
        let mut console = LogConsole::new(feed, &egui::Context::default());
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
            tracing::info!("engine started");
            tracing::debug!("loaded 12 nodes");
        });
        assert!(console.drain());

        let harness = crate::test_support::ui_harness(|ui| console.ui(ui));

        harness.get_by_label_contains("engine started");
        assert!(harness.query_by_label_contains("loaded 12 nodes").is_none());
    }

    #[test]
    fn drops_the_oldest_entries_beyond_the_limit() {
        let (layer, feed) = pipeline();
        let mut console = LogConsole::new(feed, &egui::Context::default());
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
            for index in 0..=MAX_ENTRIES {
                tracing::info!("entry {index}");
            }
        });

        console.drain();

        assert_eq!(console.entries.len(), MAX_ENTRIES);
        assert_eq!(console.entries[0].message, "entry 1");
    }
}
