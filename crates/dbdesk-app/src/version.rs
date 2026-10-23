//! Which build this is, and the few command-line flags dbdesk understands.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Set by `build.rs`: the commit the binary was built from, or "unknown".
pub const COMMIT: &str = env!("DBDESK_COMMIT");

/// `dbdesk 0.1.0 (1a2b3c4d5e)`: what `--version` prints and bug reports should quote.
pub fn describe() -> String {
    format!("dbdesk {VERSION} ({COMMIT})")
}

const USAGE: &str = "\
Usage: dbdesk [--version | --help]

Starts the dbdesk desktop app. With --version (-V), prints the version and exits.";

/// What the command line asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliRequest {
    Version,
    Help,
    /// Open the app. Arguments dbdesk doesn't know are ignored, because launchers add their
    /// own: older macOS passes `-psn_…` to apps started from Finder.
    Run,
}

impl CliRequest {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut request = Self::Run;
        for arg in args {
            match arg.as_str() {
                "--version" | "-V" => return Self::Version,
                "--help" | "-h" => request = Self::Help,
                _ => {}
            }
        }
        request
    }

    /// Prints what was asked for. Returns `true` if the app should not start.
    pub fn answer(self) -> bool {
        match self {
            Self::Version => println!("{}", describe()),
            Self::Help => println!("{}\n\n{USAGE}", describe()),
            Self::Run => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_wins_and_unknown_arguments_are_ignored() {
        let cases: [(&[&str], CliRequest); 5] = [
            (&[], CliRequest::Run),
            (&["-psn_0_12345"], CliRequest::Run),
            (&["--version"], CliRequest::Version),
            (&["--help", "-V"], CliRequest::Version),
            (&["-h"], CliRequest::Help),
        ];
        for (args, expected) in cases {
            let parsed = CliRequest::parse(args.iter().map(|arg| (*arg).to_owned()));
            assert_eq!(parsed, expected, "{args:?}");
        }
    }

    #[test]
    fn describes_the_version_and_commit() {
        let description = describe();
        assert!(description.starts_with(&format!("dbdesk {VERSION} (")));
        assert!(!COMMIT.is_empty());
    }
}
