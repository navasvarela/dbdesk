//! Bringing over what the app saved under its previous name, once, on the first start under the
//! new one. See "Migrating existing installs" in `docs/releasing.md`.
//!
//! 1. Nothing is done when the new settings folder exists (copied on an earlier start, or a new
//!    install that has run before) or the previous one doesn't. Otherwise a lock is taken, so
//!    the app and an MCP client's server starting at once copy one after the other, and step 1
//!    is checked again under it.
//! 2. Each previous folder is copied into a staging folder beside its new place, then renamed
//!    into place. The settings folder goes last: its rename is the commit point step 1 looks
//!    for, so a crash anywhere before it means the next start does everything again.
//! 3. A note is left in the previous settings folder. Nothing else there is changed.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use directories::ProjectDirs;
use rusqlite::{Connection, OpenFlags};

use crate::paths::{APP_NAME, HISTORY_FILE_NAME};

/// Not copied: the logs would never be pruned under the new log file names.
const LOGS_FOLDER: &str = "logs";

/// SQLite's write-ahead log, beside its database file.
const WAL_SUFFIX: &str = "-wal";

/// How long reading the history waits for an older copy of the app that is writing to it.
const HISTORY_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The two folders one name of the app keeps its files in. They are the same folder on macOS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallFolders {
    /// Data sources, projects, consoles, the history, settings and the user's own files.
    pub settings: PathBuf,
    /// eframe's `app.ron` (the window, the layout, open tabs) and the logs.
    pub window_state: PathBuf,
}

impl InstallFolders {
    pub(crate) fn of(dirs: &ProjectDirs) -> Self {
        Self {
            settings: dirs.config_dir().to_owned(),
            window_state: dirs.data_dir().to_owned(),
        }
    }

    fn share_one_folder(&self) -> bool {
        self.settings == self.window_state
    }
}

/// What [`adopt_previous_install`] did.
#[derive(Debug)]
pub enum Adoption {
    /// There was nothing to bring over, or it was brought over on an earlier start.
    NotNeeded,
    Copied(CopiedInstall),
}

/// A previous install whose folders are now the current ones.
#[derive(Debug)]
pub struct CopiedInstall {
    /// The previous settings folder, left as it was apart from the note.
    pub from: PathBuf,
    /// Why SQLite couldn't read the previous history, which was then copied as files instead.
    pub history_read_as_files: Option<rusqlite::Error>,
    /// Why the note couldn't be left in `from`. The copy is complete either way.
    pub note_not_written: Option<io::Error>,
}

#[derive(Debug, thiserror::Error)]
#[error("could not copy {} to {}", from.display(), to.display())]
pub struct AdoptionError {
    from: PathBuf,
    to: PathBuf,
    #[source]
    source: io::Error,
}

/// Copies the `previous` name's folders to the `current` name's, unless the current settings
/// folder exists already. Nothing in `previous` is changed or deleted, apart from the note
/// that says where it was copied.
///
/// # Errors
///
/// When a folder can't be copied. The current settings folder is then left uncreated, so the
/// next start tries again; a staging folder may be left behind, and is replaced then.
pub fn adopt_previous_install(
    previous: &InstallFolders,
    current: &InstallFolders,
) -> Result<Adoption, AdoptionError> {
    if !needs_copy(previous, current) {
        return Ok(Adoption::NotNeeded);
    }
    let lock = lock_file(&current.settings);
    let held = take_lock(&lock).map_err(|source| AdoptionError {
        from: previous.settings.clone(),
        to: current.settings.clone(),
        source,
    })?;
    // Whoever held the lock before may have copied everything meanwhile.
    if !needs_copy(previous, current) {
        return Ok(Adoption::NotNeeded);
    }
    let copied = copy_folders(previous, current);
    drop(held);
    // A process waiting for the lock holds the file open and finds the copy done once it has
    // the lock; a later one stops at the first check. A file left behind holds nothing.
    let _ = fs::remove_file(&lock);
    copied
}

fn needs_copy(previous: &InstallFolders, current: &InstallFolders) -> bool {
    previous != current && !current.settings.exists() && previous.settings.is_dir()
}

fn copy_folders(
    previous: &InstallFolders,
    current: &InstallFolders,
) -> Result<Adoption, AdoptionError> {
    if !previous.share_one_folder()
        && previous.window_state.is_dir()
        && !current.window_state.exists()
    {
        // The window state folder has no history, so there is nothing to report from it.
        copy_into_place(&previous.window_state, &current.window_state)?;
    }
    let history_read_as_files = copy_into_place(&previous.settings, &current.settings)?;
    Ok(Adoption::Copied(CopiedInstall {
        from: previous.settings.clone(),
        history_read_as_files,
        note_not_written: leave_note(&previous.settings, &current.settings).err(),
    }))
}

fn lock_file(settings: &Path) -> PathBuf {
    with_suffix(settings, ".migrating.lock")
}

/// Blocks until no other process holds the lock. The OS releases it if the holder crashes.
fn take_lock(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    file.lock()?;
    Ok(file)
}

/// Copies `from` to a staging folder beside `to`, then renames it to `to`. Returns why the
/// history had to be copied as files, if it had.
fn copy_into_place(from: &Path, to: &Path) -> Result<Option<rusqlite::Error>, AdoptionError> {
    let failed = |source| AdoptionError {
        from: from.to_owned(),
        to: to.to_owned(),
        source,
    };
    let staging = staging_folder(to);
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(failed)?;
    }
    if let Some(parent) = staging.parent() {
        fs::create_dir_all(parent).map_err(failed)?;
    }
    let history_read_as_files = copy_folder(from, &staging).map_err(failed)?;
    fs::rename(&staging, to).map_err(failed)?;
    Ok(history_read_as_files)
}

fn staging_folder(to: &Path) -> PathBuf {
    with_suffix(to, ".migrating")
}

fn copy_folder(from: &Path, to: &Path) -> io::Result<Option<rusqlite::Error>> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if is_copied_file_by_file(&name) {
            copy_entry(&entry.path(), &to.join(&name))?;
        }
    }
    let history = from.join(HISTORY_FILE_NAME);
    if history.is_file() {
        return copy_history(&history, &to.join(HISTORY_FILE_NAME));
    }
    Ok(None)
}

/// Everything but the logs, the history, which [`copy_history`] copies through SQLite, and a
/// note left by an earlier copy, which describes that one.
fn is_copied_file_by_file(name: &OsString) -> bool {
    let Some(name) = name.to_str() else {
        return true;
    };
    name != LOGS_FOLDER && name != note_file_name() && !name.starts_with(HISTORY_FILE_NAME)
}

/// The note left in the previous settings folder.
fn note_file_name() -> String {
    format!("copied-to-{APP_NAME}.txt")
}

fn copy_entry(from: &Path, to: &Path) -> io::Result<()> {
    if !from.is_dir() {
        return fs::copy(from, to).map(drop);
    }
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        copy_entry(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

/// Copies the history through SQLite's `VACUUM INTO`, which reads one consistent snapshot, the
/// write-ahead log included, even while an older copy of the app has the file open; copying
/// the files then could catch half a transaction. A database SQLite can't read is copied as
/// files instead, so it is never left behind, and the reason is returned.
fn copy_history(from: &Path, to: &Path) -> io::Result<Option<rusqlite::Error>> {
    let Err(error) = snapshot(from, to) else {
        return Ok(None);
    };
    if to.exists() {
        fs::remove_file(to)?;
    }
    fs::copy(from, to)?;
    let wal = with_suffix(from, WAL_SUFFIX);
    if wal.is_file() {
        fs::copy(wal, with_suffix(to, WAL_SUFFIX))?;
    }
    Ok(Some(error))
}

/// Read-only, so the previous file is never written: a read-write connection would checkpoint
/// the write-ahead log into it when it closes. SQLite may still create the `-shm` index beside
/// it, which holds nothing of the user's.
fn snapshot(from: &Path, to: &Path) -> rusqlite::Result<()> {
    let connection = Connection::open_with_flags(
        from,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(HISTORY_BUSY_TIMEOUT)?;
    let to = to
        .to_str()
        .ok_or_else(|| rusqlite::Error::InvalidPath(to.to_owned()))?;
    connection.execute("VACUUM INTO ?1", [to])?;
    Ok(())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

fn leave_note(previous: &Path, current: &Path) -> io::Result<()> {
    let note = format!(
        "{APP_NAME} copied this folder to\n\n    {}\n\nthe first time it started. It doesn't read \
         this folder any more, so changes made here aren't seen by {APP_NAME}. Once {APP_NAME} \
         shows everything you had, this folder can be deleted.\n",
        current.display()
    );
    fs::write(previous.join(note_file_name()), note)
}

#[cfg(test)]
mod tests;
