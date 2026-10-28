//! `dbdesk mcp --project <slug>`: the MCP server, run by an MCP client, with no window.

use anyhow::Context as _;
use dbdesk_mcp::{McpConfig, serve_stdio};
use dbdesk_store::{AppPaths, DataSourceFile, HistoryFile, ProjectFile, os_keychain};

use crate::{previous_install, telemetry};

/// Serves the project until the client closes standard input. Needs no display: nothing here
/// touches the windowing stack, so it runs on a headless machine or over SSH.
pub fn run(project: &str) -> anyhow::Result<()> {
    // Whichever starts first under a new name, the app or an MCP client's server, brings over
    // the previous install.
    let adopted = previous_install::adopt(
        AppPaths::discover()
            .context("the OS reports no home directory, so dbdesk can't find its files")?,
    );
    let paths = &adopted.paths;
    let _file_guard = telemetry::init_headless(&paths.logs_dir())?;
    adopted.log();
    let config = McpConfig {
        data_sources: DataSourceFile::new(paths.data_sources_file()),
        projects: ProjectFile::new(paths.projects_file()),
        history: HistoryFile::new(paths.history_file()),
        secrets: os_keychain(),
        known_hosts: paths.ssh_known_hosts_file(),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_name("dbdesk-mcp")
        .enable_all()
        .build()
        .context("could not start the server's runtime")?;
    let served = runtime.block_on(serve_stdio(config, project));
    if let Err(error) = &served {
        tracing::error!(project, "the MCP server stopped: {error:#}");
    }
    served.with_context(|| format!("could not serve the project {project:?}"))
}
