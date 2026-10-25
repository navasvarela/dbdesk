//! Rewriting file paths so they mean the same file on another machine: under the home folder
//! as `~/…`, beside the exported file as a path relative to it.

use std::path::{Component, Path, PathBuf};

use dbdesk_core::has_placeholder;

/// Where an exported file is written, and whose home folder it leaves.
#[derive(Debug, Clone)]
pub(crate) struct ExportPaths {
    /// The folder the exported file is in.
    pub(crate) dir: PathBuf,
    pub(crate) home: Option<PathBuf>,
}

impl ExportPaths {
    /// `path` as the export writes it.
    ///
    /// Beside the file wins over the home folder: a file and the databases next to it, shared
    /// in a repository, then work wherever the repository is checked out. Rewritten paths are
    /// spelt with `/`, which Windows reads too, so a file exported there opens elsewhere.
    pub(crate) fn portable(&self, path: &Path) -> PathBuf {
        // `has_root` rather than `is_absolute`: on Windows, `\data\shop.db` has no drive
        // letter, so isn't absolute, but is no more relative to the export than on Unix.
        // A path that isn't Unicode is kept as it is, rather than spelt lossily.
        if !path.has_root() || path.to_str().is_none_or(has_placeholder) {
            return path.to_owned();
        }
        if let Ok(beside) = path.strip_prefix(&self.dir) {
            return slashed(None, beside);
        }
        if let Some(home) = &self.home
            && let Ok(in_home) = path.strip_prefix(home)
        {
            return slashed(Some("~"), in_home);
        }
        path.to_owned()
    }
}

/// `relative`'s components joined with `/`, after `first` if given.
fn slashed(first: Option<&str>, relative: &Path) -> PathBuf {
    let parts = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy());
    let joined: Vec<_> = first.map(Into::into).into_iter().chain(parts).collect();
    PathBuf::from(joined.join("/"))
}

/// Where an imported file was read from, and whose home folder it arrives in.
#[derive(Debug, Clone)]
pub(crate) struct ImportPaths {
    /// The folder the imported file is in.
    pub(crate) dir: PathBuf,
    pub(crate) home: Option<PathBuf>,
}

impl ImportPaths {
    /// `path` as this machine's data sources hold it: `~/…` under this home folder, and a
    /// relative path next to the imported file. A path with a placeholder is kept as it is,
    /// to be resolved when connecting.
    pub(crate) fn local(&self, path: &Path) -> PathBuf {
        if path.to_str().is_some_and(has_placeholder) {
            return path.to_owned();
        }
        let mut components = path.components();
        if components.next() == Some(Component::Normal("~".as_ref())) {
            return match &self.home {
                Some(home) => home.join(components.as_path()),
                None => path.to_owned(),
            };
        }
        if !path.has_root() {
            return self.dir.join(path);
        }
        path.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_paths_beside_the_file_or_in_the_home_folder_portably() {
        let export = ExportPaths {
            dir: "/home/juan/team".into(),
            home: Some("/home/juan".into()),
        };
        let cases = [
            ("/home/juan/team/data/shop.db", "data/shop.db"),
            ("/home/juan/certs/ca.pem", "~/certs/ca.pem"),
            ("/srv/data/shop.db", "/srv/data/shop.db"),
            ("${DATA}/shop.db", "${DATA}/shop.db"),
            ("~/already.db", "~/already.db"),
        ];
        for (path, expected) in cases {
            assert_eq!(
                export.portable(Path::new(path)),
                PathBuf::from(expected),
                "{path}"
            );
        }
    }

    #[test]
    fn resolves_portable_paths_on_the_importing_machine() {
        let import = ImportPaths {
            dir: "/Users/ana/Downloads".into(),
            home: Some("/Users/ana".into()),
        };
        let cases = [
            ("data/shop.db", "/Users/ana/Downloads/data/shop.db"),
            ("~/certs/ca.pem", "/Users/ana/certs/ca.pem"),
            ("/srv/data/shop.db", "/srv/data/shop.db"),
            ("${DATA}/shop.db", "${DATA}/shop.db"),
            ("~${SUFFIX}/x.db", "~${SUFFIX}/x.db"),
        ];
        for (path, expected) in cases {
            assert_eq!(
                import.local(Path::new(path)),
                PathBuf::from(expected),
                "{path}"
            );
        }
    }
}
