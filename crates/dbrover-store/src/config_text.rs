//! Reading the hand-editable config files, which are all optional.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

/// The file's text, or `None` if there is no such file, which every config file reads as
/// "nothing changed from the defaults".
pub(crate) fn read_if_exists(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes `text` as a new file, creating its folder. Never replaces an existing file, which
/// holds the user's own settings.
pub(crate) fn create_new(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(text.as_bytes())
}
