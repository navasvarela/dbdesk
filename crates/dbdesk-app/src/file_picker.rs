//! The platform's open and save dialogs, for choosing an existing SQLite database or naming a
//! new one, and for choosing a CA certificate.

use std::path::{Path, PathBuf};

const OPEN_TITLE: &str = "Choose a SQLite database";
const NEW_TITLE: &str = "Create a SQLite database";
const CA_TITLE: &str = "Choose a CA certificate (PEM)";
/// What the Save dialog suggests for a new database.
const NEW_FILE_NAME: &str = "database.db";

/// Shows the open-file dialog and returns the chosen file, or `None` if it was cancelled.
///
/// Blocks the UI thread until the dialog closes, unlike everything else in the UI (see
/// ADR-0003). That is deliberate: the dialog is modal anyway, macOS requires it to run on the
/// main thread, and rfd's async variant would need its future polled from egui's frame loop
/// for no visible difference.
///
/// No extension filter: SQLite files are named `.db`, `.sqlite`, `.sqlite3`, `.db3`, or have
/// no extension at all, and on macOS rfd merges all filters into one allowed list, so an "All
/// files" escape hatch isn't possible. The connection test catches files that aren't SQLite.
pub fn pick_sqlite_file(current: &str) -> Option<PathBuf> {
    dialog(OPEN_TITLE, current).pick_file()
}

/// Shows the save-file dialog for a new database and returns the chosen path, or `None` if it
/// was cancelled. Nothing is created here; the engine creates the file, and refuses if it
/// already exists, even if the dialog asked "Replace?".
pub fn pick_new_sqlite_file(current: &str) -> Option<PathBuf> {
    dialog(NEW_TITLE, current)
        .set_file_name(NEW_FILE_NAME)
        .save_file()
}

/// Shows the open-file dialog for a PEM file of trusted authorities. No extension filter, for
/// the same reason as [`pick_sqlite_file`]: they come as `.pem`, `.crt` and `.cer`.
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
