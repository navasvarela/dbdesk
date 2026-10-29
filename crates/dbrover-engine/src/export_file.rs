//! Starting an export's file in its format. Every format but Parquet is written by
//! `dbrover-export`; Parquet is written by DuckDB, which the engine has through its driver and
//! `dbrover-export` doesn't, so that it stays light for the crates that only write text.

use std::io;
use std::path::{Path, PathBuf};

use dbrover_core::Cell;
use dbrover_driver_duckdb::{ParquetCompression as DuckDbCompression, ParquetWriter};
use dbrover_export::{
    ExportColumn, ExportError, ExportSpec, ExportWriter, FileFormat, ParquetCompression,
    ParquetOptions,
};

/// Starts writing `columns` to `path` in the format `spec` says.
///
/// # Errors
///
/// [`ExportError`] if the file can't be started.
pub(crate) fn create_writer(
    path: &Path,
    spec: &ExportSpec,
    columns: &[ExportColumn],
) -> Result<ExportWriter, ExportError> {
    match spec {
        ExportSpec::Parquet(options) => ExportWriter::with_format(path, |partial| {
            Ok(Box::new(ParquetFile::create(partial, *options, columns)?))
        }),
        spec => ExportWriter::create(path, spec, columns),
    }
}

/// A Parquet file through [`ParquetWriter`], as an export's [`FileFormat`].
struct ParquetFile {
    writer: ParquetWriter,
    path: PathBuf,
    compression: DuckDbCompression,
}

impl ParquetFile {
    fn create(path: &Path, options: ParquetOptions, columns: &[ExportColumn]) -> io::Result<Self> {
        let columns = columns
            .iter()
            .map(|column| (column.name.clone(), column.kind))
            .collect();
        Ok(Self {
            writer: ParquetWriter::create(columns)?,
            path: path.to_owned(),
            compression: match options.compression {
                ParquetCompression::Zstd => DuckDbCompression::Zstd,
                ParquetCompression::Snappy => DuckDbCompression::Snappy,
            },
        })
    }
}

impl FileFormat for ParquetFile {
    fn write(&mut self, rows: &[Vec<Cell>]) -> io::Result<()> {
        self.writer.append(rows)
    }

    fn finish(self: Box<Self>) -> io::Result<()> {
        let Self {
            writer,
            path,
            compression,
        } = *self;
        writer.finish(&path, compression)
    }
}
