//! Reading the hand-editable config files, which are all optional.

use std::io;
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
