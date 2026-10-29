//! Each format's options in the export dialog.

use dbrover_export::{
    ExportFormat, ExportOptions, FieldSeparator, HtmlDocument, LineEnding, MAX_ROWS_PER_INSERT,
    ParquetCompression, Quoting, RowsPerInsert, SheetName,
};
use egui::Ui;

/// The separators the dialog names; any other single character is typed in.
const SEPARATORS: [(FieldSeparator, &str); 4] = [
    (FieldSeparator::COMMA, "Comma"),
    (FieldSeparator::SEMICOLON, "Semicolon"),
    (FieldSeparator::TAB, "Tab"),
    (FieldSeparator::PIPE, "Pipe"),
];

const NULL_HINT: &str = "(an empty field)";

/// Options typed as text, which may not be valid yet: the typed options keep the last valid
/// value, and [`OptionTexts::problem`] says what is wrong with the text.
pub struct OptionTexts {
    /// Whether the separator is typed rather than one of [`SEPARATORS`].
    other_separator: bool,
    separator: String,
    sheet_name: String,
}

impl OptionTexts {
    pub fn new(options: &ExportOptions) -> Self {
        let separator = options.csv.separator;
        Self {
            other_separator: !SEPARATORS.iter().any(|(named, _)| *named == separator),
            separator: separator.get().to_string(),
            sheet_name: options.xlsx.sheet_name.as_str().to_owned(),
        }
    }

    /// What is wrong with what was typed for `format`'s options.
    pub fn problem(&self, format: ExportFormat) -> Result<(), String> {
        match format {
            ExportFormat::Csv if self.other_separator => parse_separator(&self.separator)
                .map(drop)
                .map_err(|problem| problem.to_owned()),
            ExportFormat::Xlsx => SheetName::try_from(self.sheet_name.clone())
                .map(drop)
                .map_err(|problem| problem.to_string()),
            _ => Ok(()),
        }
    }
}

fn parse_separator(text: &str) -> Result<FieldSeparator, &'static str> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(separator), None) => FieldSeparator::try_from(separator)
            .map_err(|_| "A quote or a line break can't separate fields"),
        _ => Err("Type one character to separate fields"),
    }
}

/// The options of `format`, laid out as a form.
pub fn options_ui(
    ui: &mut Ui,
    format: ExportFormat,
    options: &mut ExportOptions,
    texts: &mut OptionTexts,
) {
    egui::Grid::new("export_options")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| match format {
            ExportFormat::Csv => csv(ui, options, texts),
            ExportFormat::Xlsx => xlsx(ui, options, texts),
            ExportFormat::Tsv => {
                header(ui, &mut options.tsv.header);
                null_text(ui, &mut options.tsv.null_text);
            }
            ExportFormat::Markdown => null_text(ui, &mut options.markdown.null_text),
            ExportFormat::SqlInsert => {
                rows_per_statement(ui, &mut options.insert.rows_per_statement)
            }
            ExportFormat::Html => html(ui, options),
            ExportFormat::Parquet => parquet(ui, &mut options.parquet.compression),
            ExportFormat::Json | ExportFormat::JsonLines | ExportFormat::SqlUpdate => {
                ui.weak("This format has no options.");
                ui.end_row();
            }
        });
}

fn csv(ui: &mut Ui, options: &mut ExportOptions, texts: &mut OptionTexts) {
    let csv = &mut options.csv;
    ui.label("Separator");
    ui.horizontal(|ui| {
        let selected = match SEPARATORS.iter().find(|(named, _)| *named == csv.separator) {
            Some((_, name)) if !texts.other_separator => *name,
            _ => "Other",
        };
        egui::ComboBox::from_id_salt("csv_separator")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (separator, name) in SEPARATORS {
                    if ui.selectable_label(selected == name, name).clicked() {
                        csv.separator = separator;
                        texts.other_separator = false;
                    }
                }
                if ui.selectable_label(selected == "Other", "Other").clicked() {
                    texts.other_separator = true;
                }
            });
        if texts.other_separator {
            let field = ui.add(
                egui::TextEdit::singleline(&mut texts.separator)
                    .desired_width(32.0)
                    .char_limit(1),
            );
            if field.changed()
                && let Ok(separator) = parse_separator(&texts.separator)
            {
                csv.separator = separator;
            }
        }
    });
    ui.end_row();
    ui.label("Quotes");
    ui.horizontal(|ui| {
        ui.radio_value(&mut csv.quoting, Quoting::WhereNeeded, "Where needed");
        ui.radio_value(&mut csv.quoting, Quoting::Always, "Around every value");
    });
    ui.end_row();
    header(ui, &mut csv.header);
    null_text(ui, &mut csv.null_text);
    ui.label("Line ending");
    ui.horizontal(|ui| {
        ui.radio_value(&mut csv.line_ending, LineEnding::Crlf, "CRLF")
            .on_hover_text("As RFC 4180 asks, and Windows programs expect");
        ui.radio_value(&mut csv.line_ending, LineEnding::Lf, "LF");
    });
    ui.end_row();
    ui.label("");
    ui.checkbox(&mut csv.bom, "Byte order mark")
        .on_hover_text("What Excel needs to read accents in a UTF-8 file");
    ui.end_row();
}

fn xlsx(ui: &mut Ui, options: &mut ExportOptions, texts: &mut OptionTexts) {
    let xlsx = &mut options.xlsx;
    header(ui, &mut xlsx.header);
    ui.label("");
    ui.add_enabled(
        xlsx.header,
        egui::Checkbox::new(&mut xlsx.freeze_header, "Keep the header in view"),
    );
    ui.end_row();
    ui.label("");
    ui.add_enabled(
        xlsx.header,
        egui::Checkbox::new(&mut xlsx.autofilter, "Filter buttons on the header"),
    );
    ui.end_row();
    ui.label("Sheet name");
    let field = ui.add(egui::TextEdit::singleline(&mut texts.sheet_name).desired_width(200.0));
    if field.changed()
        && let Ok(name) = SheetName::try_from(texts.sheet_name.clone())
    {
        xlsx.sheet_name = name;
    }
    ui.end_row();
}

fn html(ui: &mut Ui, options: &mut ExportOptions) {
    let html = &mut options.html;
    ui.label("Write");
    ui.horizontal(|ui| {
        ui.radio_value(&mut html.document, HtmlDocument::Page, "A whole page");
        ui.radio_value(&mut html.document, HtmlDocument::Fragment, "The table only")
            .on_hover_text("To paste into an email or a page");
    });
    ui.end_row();
    null_text(ui, &mut html.null_text);
}

fn parquet(ui: &mut Ui, compression: &mut ParquetCompression) {
    ui.label("Compression");
    ui.horizontal(|ui| {
        ui.radio_value(compression, ParquetCompression::Zstd, "zstd")
            .on_hover_text("Smaller files; what most tools default to");
        ui.radio_value(compression, ParquetCompression::Snappy, "Snappy")
            .on_hover_text("For tools that read only Snappy");
    });
    ui.end_row();
}

fn rows_per_statement(ui: &mut Ui, rows: &mut RowsPerInsert) {
    ui.label("Rows a statement");
    let mut count = rows.get();
    ui.add(egui::DragValue::new(&mut count).range(1..=MAX_ROWS_PER_INSERT))
        .on_hover_text("Up to 1,000, SQL Server's limit for one VALUES list");
    *rows = RowsPerInsert::clamped(count);
    ui.end_row();
}

fn header(ui: &mut Ui, header: &mut bool) {
    ui.label("");
    ui.checkbox(header, "Header with the column names");
    ui.end_row();
}

fn null_text(ui: &mut Ui, text: &mut String) {
    ui.label("NULL as");
    let hint = if text.is_empty() { NULL_HINT } else { "" };
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(120.0),
    );
    ui.end_row();
}
