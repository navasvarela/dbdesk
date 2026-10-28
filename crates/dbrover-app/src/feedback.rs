//! Help › Report a problem… and Help › Ask a question…: the project's issue forms and
//! Discussions, opened in the browser.

use std::env::consts::{ARCH, OS};

use crate::version;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// `.github/ISSUE_TEMPLATE/bug_report.yml`. Its `version` and `os` fields take their values from
/// the query parameters of the same names, so renaming either id there breaks the prefill.
const BUG_REPORT_TEMPLATE: &str = "bug_report.yml";
/// The slug GitHub gave the Q&A category when Discussions was turned on.
const QUESTIONS_CATEGORY: &str = "q-a";

/// A new bug report with the version, commit and platform filled in, and nothing else: the
/// address stays in the browser's history and GitHub's logs, so anything about the user's work
/// is theirs to type into the form, or not.
pub fn bug_report_url() -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("template", BUG_REPORT_TEMPLATE)
        .append_pair("version", &version::describe())
        .append_pair("os", &platform())
        .finish();
    format!("{REPOSITORY}/issues/new?{query}")
}

pub fn new_question_url() -> String {
    format!("{REPOSITORY}/discussions/new?category={QUESTIONS_CATEGORY}")
}

/// `macOS, aarch64`: the system DbRover was built for. The standard library can't tell its
/// version, which the form asks the user to add.
fn platform() -> String {
    let name = match OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    format!("{name}, {ARCH}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bug_report_carries_only_the_version_and_platform() {
        let url = bug_report_url();
        let (address, query) = url.split_once('?').unwrap_or((&url, ""));
        let pairs: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();

        assert_eq!(address, "https://github.com/navasvarela/dbrover/issues/new");
        assert_eq!(
            pairs,
            [
                ("template".to_owned(), "bug_report.yml".to_owned()),
                ("version".to_owned(), version::describe()),
                ("os".to_owned(), platform()),
            ]
        );
    }

    #[test]
    fn the_version_is_encoded_for_the_address() {
        assert!(bug_report_url().contains(&format!(
            "version=dbrover+{}+%28{}%29",
            version::VERSION,
            version::COMMIT
        )));
    }

    #[test]
    fn a_question_opens_the_q_and_a_category() {
        assert_eq!(
            new_question_url(),
            "https://github.com/navasvarela/dbrover/discussions/new?category=q-a"
        );
    }
}
