//! Every format's options at once, as an export dialog edits and remembers them.

use dbrover_core::ResultShape;
use serde::{Deserialize, Serialize};

use crate::parquet::ParquetOptions;
use crate::text::{
    CsvOptions, HtmlOptions, InsertOptions, InsertSpec, MarkdownOptions, RowSource, TsvOptions,
    UpdateSpec,
};
use crate::xlsx::XlsxOptions;
use crate::{ExportFormat, ExportSpec};

/// The options of every format, so switching format keeps what was chosen for each. Saved as
/// they are; a field missing from what was saved takes its default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportOptions {
    pub csv: CsvOptions,
    pub xlsx: XlsxOptions,
    pub tsv: TsvOptions,
    pub markdown: MarkdownOptions,
    pub insert: InsertOptions,
    pub parquet: ParquetOptions,
    pub html: HtmlOptions,
}

impl ExportOptions {
    /// `format` with these options, for rows from `source`.
    ///
    /// # Errors
    ///
    /// Why the format can't write these rows: the SQL formats need to know their table.
    pub fn spec(
        &self,
        format: ExportFormat,
        source: &RowSource,
    ) -> Result<ExportSpec, &'static str> {
        Ok(match format {
            ExportFormat::Csv => ExportSpec::Csv(self.csv.clone()),
            ExportFormat::Xlsx => ExportSpec::Xlsx(self.xlsx.clone()),
            ExportFormat::Tsv => ExportSpec::Tsv(self.tsv.clone()),
            ExportFormat::Markdown => ExportSpec::Markdown(self.markdown.clone()),
            ExportFormat::SqlInsert => {
                ExportSpec::SqlInsert(Box::new(InsertSpec::for_rows(source, self.insert)?))
            }
            ExportFormat::Json => match source.shape {
                ResultShape::Rows => ExportSpec::Json,
                ResultShape::Documents => ExportSpec::JsonDocuments,
            },
            ExportFormat::JsonLines => match source.shape {
                ResultShape::Rows => ExportSpec::JsonLines,
                ResultShape::Documents => ExportSpec::JsonLinesDocuments,
            },
            ExportFormat::Parquet => ExportSpec::Parquet(self.parquet),
            ExportFormat::Html => ExportSpec::Html(self.html.clone()),
            ExportFormat::SqlUpdate => {
                ExportSpec::SqlUpdate(Box::new(UpdateSpec::for_rows(source)?))
            }
        })
    }
}
