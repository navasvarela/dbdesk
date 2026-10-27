//! Which build this is, and the few command-line flags dbdesk understands.

use std::path::PathBuf;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Set by `build.rs`: the commit the binary was built from, or "unknown".
pub const COMMIT: &str = env!("DBDESK_COMMIT");

/// `dbdesk 0.1.0 (1a2b3c4d5e)`: what `--version` prints and bug reports should quote.
pub fn describe() -> String {
    format!("dbdesk {VERSION} ({COMMIT})")
}

const USAGE: &str = "\
Usage: dbdesk [--version | --help] [FILE.sql …]
       dbdesk mcp --project <slug>

Starts the dbdesk desktop app, opening each FILE.sql given in a console. With --version (-V),
prints the version and exits.

`dbdesk mcp --project <slug>` runs an MCP server for the project with that slug, read-only,
over standard input and output, without opening a window. An MCP client starts it; see
\"Connecting an AI assistant\" in the manual.";

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliRequest {
    Version,
    Help,
    /// Serve a project to an MCP client, headless.
    Mcp {
        project: String,
    },
    /// Arguments that can't be followed, with what was wrong.
    Invalid(String),
    /// Open the app, and in it the files named. Options dbdesk doesn't know are ignored,
    /// because launchers add their own: older macOS passes `-psn_…` to apps started from
    /// Finder.
    Run {
        files: Vec<PathBuf>,
    },
}

impl CliRequest {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut args = args.into_iter().peekable();
        if args.peek().is_some_and(|first| first == "mcp") {
            return Self::parse_mcp(args.skip(1));
        }
        let mut help = false;
        let mut files = Vec::new();
        for arg in args {
            match arg.as_str() {
                "--version" | "-V" => return Self::Version,
                "--help" | "-h" => help = true,
                option if option.starts_with('-') => {}
                file => files.push(PathBuf::from(file)),
            }
        }
        if help {
            Self::Help
        } else {
            Self::Run { files }
        }
    }

    /// Unlike the app, the subcommand refuses arguments it doesn't know: an MCP client's
    /// configuration is typed by hand, and a typo should fail loudly, not be ignored.
    fn parse_mcp(args: impl Iterator<Item = String>) -> Self {
        let mut args = args.peekable();
        let mut project = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--project" | "-p" => match args.next() {
                    Some(slug) => project = Some(slug),
                    None => return Self::Invalid("--project needs a project's slug".to_owned()),
                },
                "--help" | "-h" => return Self::Help,
                "--version" | "-V" => return Self::Version,
                other => match other.strip_prefix("--project=") {
                    Some(slug) => project = Some(slug.to_owned()),
                    None => return Self::Invalid(format!("unknown argument {other:?}")),
                },
            }
        }
        match project {
            Some(project) if !project.is_empty() => Self::Mcp { project },
            _ => Self::Invalid("dbdesk mcp needs --project <slug>".to_owned()),
        }
    }

    /// Prints what was asked for, if it is only something to print. Returns `true` if
    /// nothing else should happen.
    pub fn answer(&self) -> bool {
        match self {
            Self::Version => println!("{}", describe()),
            Self::Help => println!("{}\n\n{USAGE}", describe()),
            Self::Mcp { .. } | Self::Invalid(_) | Self::Run { .. } => return false,
        }
        true
    }

    pub fn usage() -> &'static str {
        USAGE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_mcp_subcommand_strictly() {
        let mcp = |project: &str| CliRequest::Mcp {
            project: project.to_owned(),
        };
        let cases: [(&[&str], CliRequest); 8] = [
            (&["mcp", "--project", "analytics"], mcp("analytics")),
            (&["mcp", "--project=shop-eu"], mcp("shop-eu")),
            (&["mcp", "-p", "shop"], mcp("shop")),
            (&["mcp", "--help"], CliRequest::Help),
            (
                &["mcp"],
                CliRequest::Invalid("dbdesk mcp needs --project <slug>".to_owned()),
            ),
            (
                &["mcp", "--project"],
                CliRequest::Invalid("--project needs a project's slug".to_owned()),
            ),
            (
                &["mcp", "--projcet", "shop"],
                CliRequest::Invalid("unknown argument \"--projcet\"".to_owned()),
            ),
            // Only as the first argument; elsewhere it names a file to open.
            (
                &["-psn_0_1", "mcp"],
                CliRequest::Run {
                    files: vec!["mcp".into()],
                },
            ),
        ];
        for (args, expected) in cases {
            let parsed = CliRequest::parse(args.iter().map(|arg| (*arg).to_owned()));
            assert_eq!(parsed, expected, "{args:?}");
        }
    }

    #[test]
    fn version_wins_files_are_opened_and_unknown_options_ignored() {
        let run = |files: &[&str]| CliRequest::Run {
            files: files.iter().map(PathBuf::from).collect(),
        };
        let cases: [(&[&str], CliRequest); 6] = [
            (&[], run(&[])),
            (&["-psn_0_12345"], run(&[])),
            (
                &["-psn_0_1", "a.sql", "../b.sql"],
                run(&["a.sql", "../b.sql"]),
            ),
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
