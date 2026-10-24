//! dbdesk's MCP server: lets an AI assistant browse and query the data sources of one
//! project, read-only, through the Model Context Protocol. See `docs/mcp-server.md`.
//!
//! The app's binary runs it as `dbdesk mcp --project <slug>`, over standard input and output;
//! this crate holds everything but the command line, so it can be tested without the app.
//!
//! Reading is enforced twice: [`dbdesk_sql::classify`] refuses any statement that isn't a
//! read (or can't be parsed) before it is sent, and every call runs inside the database's
//! own read-only transaction, which catches what the parser can't see, such as a function
//! that writes.

mod audit;
mod catalog;
mod connections;
mod project;
mod read_only;
mod server;

use std::sync::Arc;

use dbdesk_store::{DataSourceFile, HistoryFile, ProjectFile, SecretStore};
use rmcp::ServiceExt as _;
use rmcp::service::ServerInitializeError;
use tokio::io::{AsyncRead, AsyncWrite};

pub use project::{Limits, ServedProject, StartError};

use crate::audit::Audit;
use crate::connections::Connections;
use crate::server::DbdeskMcp;

/// Where the server finds what the app saved.
pub struct McpConfig {
    pub data_sources: DataSourceFile,
    pub projects: ProjectFile,
    pub history: HistoryFile,
    pub secrets: Arc<dyn SecretStore>,
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error(transparent)]
    Start(#[from] StartError),
    #[error("the MCP client's handshake failed")]
    Initialize(#[source] Box<ServerInitializeError>),
    #[error("the MCP session ended with an error")]
    Session(#[source] tokio::task::JoinError),
}

/// Serves the project with `slug` over standard input and output until the client closes
/// them. Nothing else may write to standard output: it carries the protocol.
///
/// # Errors
///
/// [`ServeError`] if the project can't be served or the session fails.
pub async fn serve_stdio(config: McpConfig, slug: &str) -> Result<(), ServeError> {
    serve(config, slug, (tokio::io::stdin(), tokio::io::stdout())).await
}

/// [`serve_stdio`] over any pair of streams, for tests.
///
/// # Errors
///
/// [`ServeError`] if the project can't be served or the session fails.
pub async fn serve<R, W>(config: McpConfig, slug: &str, transport: (R, W)) -> Result<(), ServeError>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let project = ServedProject::load(&config.projects, &config.data_sources, slug)?;
    tracing::info!(
        project = project.slug(),
        data_sources = project.data_sources.len(),
        max_rows = project.limits.max_rows,
        "serving the project to an MCP client, read-only"
    );
    let audit = Audit::open(project.slug(), &config.history);
    let connections = Connections::new(config.secrets, &project.data_sources);
    connections.sweep_idle();
    let server = DbdeskMcp::new(project, Arc::clone(&connections), audit);
    let running = server
        .serve(transport)
        .await
        .map_err(|error| ServeError::Initialize(Box::new(error)))?;
    let quit = running.waiting().await.map_err(ServeError::Session)?;
    tracing::info!("the MCP client disconnected ({quit:?})");
    connections.close_all().await;
    Ok(())
}
