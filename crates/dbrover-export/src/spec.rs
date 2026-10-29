//! The formats, and a format with its options: what an export is asked to write.

use crate::text::{
    Csv, CsvOptions, Html, HtmlOptions, Json, JsonLines, Markdown, MarkdownOptions, TextFormat,
    Tsv, TsvOptions,
};

/// A format DbRover writes. What each one is, where it can go and what its file is called are
/// methods here, so adding a format is one compiler-checked change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExportFormat {
    Csv,
    Tsv,
    Markdown,
    Json,
    JsonLines,
    Html,
}

impl ExportFormat {
    /// In the order menus list them: the most asked for first.
    pub const ALL: [Self; 6] = [
        Self::Csv,
        Self::Tsv,
        Self::Markdown,
        Self::Json,
        Self::JsonLines,
        Self::Html,
    ];

    /// The format's name, as menus and the save dialog show it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Tsv => "TSV",
            Self::Markdown => "Markdown",
            Self::Json => "JSON",
            Self::JsonLines => "JSON Lines",
            Self::Html => "HTML",
        }
    }

    /// The file name extension, without its dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Markdown => "md",
            Self::Json => "json",
            Self::JsonLines => "jsonl",
            Self::Html => "html",
        }
    }

    /// The format with every option at its default.
    pub fn default_spec(self) -> ExportSpec {
        match self {
            Self::Csv => ExportSpec::Csv(CsvOptions::default()),
            Self::Tsv => ExportSpec::Tsv(TsvOptions::default()),
            Self::Markdown => ExportSpec::Markdown(MarkdownOptions::default()),
            Self::Json => ExportSpec::Json,
            Self::JsonLines => ExportSpec::JsonLines,
            Self::Html => ExportSpec::Html(HtmlOptions::default()),
        }
    }
}

/// A format with its options. Each format has its own options type, so no option can be
/// given to a format it doesn't apply to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportSpec {
    Csv(CsvOptions),
    Tsv(TsvOptions),
    Markdown(MarkdownOptions),
    /// An array of objects, values as strings: what DbRover has always written.
    Json,
    JsonLines,
    Html(HtmlOptions),
}

impl ExportSpec {
    pub fn format(&self) -> ExportFormat {
        match self {
            Self::Csv(_) => ExportFormat::Csv,
            Self::Tsv(_) => ExportFormat::Tsv,
            Self::Markdown(_) => ExportFormat::Markdown,
            Self::Json => ExportFormat::Json,
            Self::JsonLines => ExportFormat::JsonLines,
            Self::Html(_) => ExportFormat::Html,
        }
    }

    /// A writer of this format, ready for its header.
    pub(crate) fn text_format(&self) -> Box<dyn TextFormat> {
        match self {
            Self::Csv(options) => Box::new(Csv::new(options.clone())),
            Self::Tsv(options) => Box::new(Tsv::new(options.clone())),
            Self::Markdown(options) => Box::new(Markdown::new(options.clone())),
            Self::Json => Box::<Json>::default(),
            Self::JsonLines => Box::<JsonLines>::default(),
            Self::Html(options) => Box::new(Html::new(options.clone())),
        }
    }
}

/// An entry in a list of formats to export to: a format with its defaults, or a preset of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportChoice {
    pub name: &'static str,
    /// What the entry writes, for a tooltip.
    pub description: &'static str,
    pub spec: ExportSpec,
}

/// Every format at its defaults, in [`ExportFormat::ALL`]'s order, with **CSV for Excel** after
/// CSV.
pub fn export_choices() -> Vec<ExportChoice> {
    let mut choices: Vec<ExportChoice> = ExportFormat::ALL
        .into_iter()
        .map(|format| ExportChoice {
            name: format.name(),
            description: description(format),
            spec: format.default_spec(),
        })
        .collect();
    choices.insert(
        1,
        ExportChoice {
            name: "CSV for Excel",
            description: "CSV with a byte order mark, so Excel reads accents and other \
                          non-English letters. Excel set to a language that separates lists \
                          with semicolons may still put each line in one column.",
            spec: ExportSpec::Csv(CsvOptions::for_excel()),
        },
    );
    choices
}

fn description(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Csv => "Comma-separated values, as RFC 4180 describes them",
        ExportFormat::Tsv => "Tab-separated values, as Copy puts them on the clipboard",
        ExportFormat::Markdown => "A table for a pull request, an issue or a chat",
        ExportFormat::Json => "An array of objects, one a row, every value a string",
        ExportFormat::JsonLines => "One JSON object a line, as data pipelines and jq -c read",
        ExportFormat::Html => "A web page with the rows as a table",
    }
}
