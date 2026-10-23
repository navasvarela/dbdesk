//! Records which commit the binary was built from, for `dbdesk --version` and the About dialog.
//!
//! `DBDESK_COMMIT` in the environment wins (CI sets it from the tag's commit). Otherwise the
//! script asks git, and falls back to "unknown" when building from a source archive with no
//! repository.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=DBDESK_COMMIT");
    let commit = std::env::var("DBDESK_COMMIT")
        .ok()
        .filter(|commit| !commit.trim().is_empty())
        .or_else(git_commit)
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=DBDESK_COMMIT={}", short(commit.trim()));

    // Rebuild when HEAD moves, so a local build never reports a stale commit.
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    if let Some(refs) = git(&["rev-parse", "--git-path", "refs/heads"]) {
        println!("cargo:rerun-if-changed={refs}");
    }
}

/// Ten hex digits: unambiguous in a repository this size, short enough to read out.
fn short(commit: &str) -> &str {
    commit.get(..10).unwrap_or(commit)
}

fn git_commit() -> Option<String> {
    git(&["rev-parse", "HEAD"])
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
