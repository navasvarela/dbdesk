//! The platform's open and save dialogs, for choosing an existing database file (SQLite or
//! DuckDB) or naming a new one, and for choosing a CA certificate.

use std::path::{Path, PathBuf};

use dbdesk_engine::DatabaseFile;

const CA_TITLE: &str = "Choose a CA certificate (PEM)";

/// An engine whose databases are files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileEngine {
    Sqlite,
    DuckDb,
}

impl FileEngine {
    fn open_title(self) -> &'static str {
        match self {
            Self::Sqlite => "Choose a SQLite database",
            Self::DuckDb => "Choose a DuckDB database",
        }
    }

    fn new_title(self) -> &'static str {
        match self {
            Self::Sqlite => "Create a SQLite database",
            Self::DuckDb => "Create a DuckDB database",
        }
    }

    /// What the Save dialog suggests for a new database.
    fn new_file_name(self) -> &'static str {
        match self {
            Self::Sqlite => "database.db",
            Self::DuckDb => "database.duckdb",
        }
    }

    /// The new database file at `path`, for the engine to create.
    pub fn file(self, path: PathBuf) -> DatabaseFile {
        match self {
            Self::Sqlite => DatabaseFile::Sqlite(path),
            Self::DuckDb => DatabaseFile::DuckDb(path),
        }
    }
}

/// Shows the open-file dialog and returns the chosen file, or `None` if it was cancelled.
///
/// Blocks the UI thread until the dialog closes, unlike everything else in the UI (see
/// ADR-0003). That is deliberate: the dialog is modal anyway, macOS requires it to run on the
/// main thread, and rfd's async variant would need its future polled from egui's frame loop
/// for no visible difference.
///
/// No extension filter: SQLite files are named `.db`, `.sqlite`, `.sqlite3`, `.db3`, or have
/// no extension at all (DuckDB's `.duckdb` or `.db`), and on macOS rfd merges all filters into
/// one allowed list, so an "All files" escape hatch isn't possible. The connection test catches
/// files of another kind.
pub fn pick_database_file(engine: FileEngine, current: &str) -> Option<PathBuf> {
    dialog(engine.open_title(), current).pick_file()
}

/// Shows the save-file dialog for a new database and returns the chosen path, or `None` if it
/// was cancelled. Nothing is created here; the engine creates the file, and refuses if it
/// already exists, even if the dialog asked "Replace?".
pub fn pick_new_database_file(engine: FileEngine, current: &str) -> Option<PathBuf> {
    dialog(engine.new_title(), current)
        .set_file_name(engine.new_file_name())
        .save_file()
}

/// Shows the open-file dialog for a PEM file of trusted authorities. No extension filter, for
/// the same reason as [`pick_database_file`]: they come as `.pem`, `.crt` and `.cer`.
pub fn pick_ca_file(current: &str) -> Option<PathBuf> {
    dialog(CA_TITLE, current).pick_file()
}

fn dialog(title: &str, current: &str) -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new().set_title(title);
    match starting_directory(current) {
        Some(directory) => dialog.set_directory(directory),
        None => dialog,
    }
}

/// Where the dialog opens: the folder of the path already in the field, when that folder
/// exists. Otherwise the platform's default (usually the last folder used).
fn starting_directory(current: &str) -> Option<PathBuf> {
    let path = Path::new(current.trim());
    if path.as_os_str().is_empty() {
        return None;
    }
    if path.is_dir() {
        return Some(path.to_owned());
    }
    path.parent()
        .filter(|parent| parent.is_dir())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_in_the_folder_of_the_current_path_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shop.db");
        let file_text = file.display().to_string();
        let missing_folder = dir.path().join("gone/shop.db").display().to_string();

        let cases = [
            ("", None),
            ("   ", None),
            (file_text.as_str(), Some(dir.path().to_owned())),
            (
                &dir.path().display().to_string(),
                Some(dir.path().to_owned()),
            ),
            (missing_folder.as_str(), None),
        ];
        for (current, expected) in cases {
            assert_eq!(starting_directory(current), expected, "{current:?}");
        }
    }
}
