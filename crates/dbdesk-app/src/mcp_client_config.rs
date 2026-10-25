//! Copy MCP configuration: the entry an MCP client's configuration needs to start
//! `dbdesk mcp` for a project, with this machine's path to dbdesk.

use std::io;
use std::path::{Path, PathBuf};

use dbdesk_store::ProjectSlug;
use serde_json::json;

/// Set by the AppImage runtime to the `.AppImage` file itself. The running executable is
/// inside the image's temporary mount, which is gone once dbdesk quits, so a client
/// configured with that path couldn't start it.
const APPIMAGE: &str = "APPIMAGE";

/// The path a client should run: the executable inside the `.app` bundle on macOS (what
/// `current_exe` already is there), the AppImage on Linux, and the installed `.exe` on
/// Windows. Symbolic links are resolved, so the path keeps working whatever the link was.
pub fn executable() -> io::Result<PathBuf> {
    if let Some(appimage) = std::env::var_os(APPIMAGE).filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(appimage));
    }
    let current = std::env::current_exe()?;
    Ok(current.canonicalize().unwrap_or(current))
}

/// The `mcpServers` entry for `slug`, as the JSON clients such as Claude Desktop, Claude Code
/// and Cursor read, named `dbdesk-<slug>` so several projects can sit side by side.
pub fn client_configuration(slug: &ProjectSlug, executable: &Path) -> String {
    let configuration = json!({
        "mcpServers": {
            format!("dbdesk-{slug}"): {
                "command": executable.display().to_string(),
                "args": ["mcp", "--project", slug.as_str()],
            }
        }
    });
    // Serializing a `Value` built from strings can't fail; the fallback is never used.
    serde_json::to_string_pretty(&configuration).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_project_and_the_executable() {
        let slug = ProjectSlug::from_name("Analytics");
        let text = client_configuration(
            &slug,
            Path::new("/Applications/dbdesk.app/Contents/MacOS/dbdesk"),
        );

        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            parsed,
            json!({
                "mcpServers": {
                    "dbdesk-analytics": {
                        "command": "/Applications/dbdesk.app/Contents/MacOS/dbdesk",
                        "args": ["mcp", "--project", "analytics"]
                    }
                }
            })
        );
    }

    #[test]
    fn finds_this_executable() {
        let path = executable().unwrap();
        assert!(path.is_absolute(), "{}", path.display());
    }
}
