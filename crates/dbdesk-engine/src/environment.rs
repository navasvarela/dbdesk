//! The process's environment, which `${NAME}` placeholders in data sources are resolved from.

use std::path::PathBuf;

use dbdesk_core::Variables;

/// The variables dbdesk was started with. On macOS an app opened from the Finder gets only
/// the login session's variables, not those a shell profile sets; the manual says so.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessEnvironment;

impl Variables for ProcessEnvironment {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn home(&self) -> Option<PathBuf> {
        std::env::home_dir()
    }
}
