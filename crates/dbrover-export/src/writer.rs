//! Writing rows to a file a page at a time, put in place only once complete.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use dbrover_core::Cell;

use crate::text::TextFormat;
use crate::{ExportColumn, ExportSpec};

#[derive(Debug, thiserror::Error)]
#[error("could not write {}", path.display())]
pub struct ExportError {
    path: PathBuf,
    #[source]
    source: io::Error,
}

/// Writes `rows` under a header of `columns` to `path`, replacing it.
///
/// # Errors
///
/// [`ExportError`] if the file can't be created or written.
pub fn export_rows(
    path: &Path,
    spec: &ExportSpec,
    columns: &[ExportColumn],
    rows: &[Vec<Cell>],
) -> Result<(), ExportError> {
    let mut writer = ExportWriter::create(path, spec, columns)?;
    writer.write(rows)?;
    writer.finish()
}

/// Writes rows to a file a page at a time, so an export never holds them all.
///
/// They go to a file beside `path` (its name plus `.partial`), renamed to `path` by
/// [`ExportWriter::finish`]: an export that fails or is abandoned never leaves half a file
/// under the name chosen, nor replaces a file that was there.
pub struct ExportWriter {
    out: BufWriter<File>,
    format: Box<dyn TextFormat>,
    /// How many rows were written so far.
    rows: usize,
    path: PathBuf,
    partial: PathBuf,
}

impl ExportWriter {
    /// Starts the file, writing the format's header.
    ///
    /// # Errors
    ///
    /// [`ExportError`] if the file can't be created or written.
    pub fn create(
        path: &Path,
        spec: &ExportSpec,
        columns: &[ExportColumn],
    ) -> Result<Self, ExportError> {
        let mut partial_name = path.file_name().unwrap_or_default().to_owned();
        partial_name.push(".partial");
        let partial = path.with_file_name(partial_name);
        let failed = |source| ExportError {
            path: path.to_owned(),
            source,
        };
        let mut out = BufWriter::new(File::create(&partial).map_err(failed)?);
        let mut format = spec.text_format();
        if let Err(error) = format.header(&mut out, columns) {
            drop(out);
            // The header failed to write; the partial file's removal can only add to that.
            let _ = fs::remove_file(&partial);
            return Err(failed(error));
        }
        Ok(Self {
            out,
            format,
            rows: 0,
            path: path.to_owned(),
            partial,
        })
    }

    /// Appends rows.
    ///
    /// # Errors
    ///
    /// [`ExportError`] if the file can't be written.
    pub fn write(&mut self, rows: &[Vec<Cell>]) -> Result<(), ExportError> {
        for row in rows {
            self.format
                .row(&mut self.out, row)
                .map_err(|source| self.error(source))?;
            self.rows += 1;
        }
        Ok(())
    }

    /// How many rows have been written.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Ends the file and puts it in place.
    ///
    /// # Errors
    ///
    /// [`ExportError`] if the file can't be written or renamed.
    pub fn finish(mut self) -> Result<(), ExportError> {
        let finished = self
            .format
            .footer(&mut self.out)
            .and_then(|()| self.out.flush())
            .and_then(|()| fs::rename(&self.partial, &self.path));
        finished.map_err(|source| self.error(source))
    }

    /// Stops writing and removes what was written.
    pub fn abandon(self) {
        let Self { out, partial, .. } = self;
        drop(out);
        // Nothing more can be done about a partial file that won't go; its name says what it is.
        let _ = fs::remove_file(partial);
    }

    fn error(&self, source: io::Error) -> ExportError {
        ExportError {
            path: self.path.clone(),
            source,
        }
    }
}
