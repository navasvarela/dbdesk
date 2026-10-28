use std::path::Path;
use std::{fs, io};

/// Writes `contents` to a temporary file next to `path`, creating the folder if needed, and
/// renames it into place, so a crash mid-write leaves either the old or the new file, never
/// a truncated one.
pub(crate) fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut temp_name = path.file_name().unwrap_or_default().to_owned();
    temp_name.push(".tmp");
    let temp_path = path.with_file_name(temp_name);
    fs::write(&temp_path, contents)?;
    fs::rename(&temp_path, path)
}
