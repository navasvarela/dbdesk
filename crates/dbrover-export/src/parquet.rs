//! Parquet's options. The file itself is written by DuckDB, through the engine: see
//! `ExportSpec::file_format`.

use serde::{Deserialize, Serialize};

/// What the user chooses about a Parquet file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParquetOptions {
    pub compression: ParquetCompression,
}

/// How the file's pages are compressed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParquetCompression {
    /// Smaller files; what DuckDB and most tools default to.
    #[default]
    Zstd,
    /// Faster to read, and what some older tools read only.
    Snappy,
}
