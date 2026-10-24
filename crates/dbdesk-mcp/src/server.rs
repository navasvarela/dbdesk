//! The MCP tools, on rmcp's router.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use dbdesk_core::{
    CatalogNode, CatalogPath, Connection, DataSource, DriverError, Identifier, NodeKind,
    RelationName, describe_error,
};
use dbdesk_sql::{Dialect, StatementKind, classify};
use dbdesk_store::HistoryOutcome;
use jiff::Timestamp;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::audit::{Audit, Ran};
use crate::catalog::{self, NodeJson, PathStep};
use crate::connections::Connections;
use crate::project::ServedProject;
use crate::read_only::{self, Rows, RunError};

/// Work to run on a connection inside the read-only transaction. Boxed because it borrows
/// the connection for as long as it runs.
type Work<'c, T> = Pin<Box<dyn Future<Output = Result<T, DriverError>> + Send + 'c>>;

#[derive(Clone)]
pub(crate) struct DbdeskMcp {
    inner: Arc<Inner>,
    tool_router: ToolRouter<Self>,
}

struct Inner {
    project: ServedProject,
    connections: Arc<Connections>,
    audit: Audit,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct ListObjectsArgs {
    /// The data source's name, as list_data_sources gives it (or its id).
    data_source: String,
    /// Which node's children to list: a `path` from an earlier list_objects result. Leave
    /// it out for the top level.
    #[serde(default)]
    path: Vec<PathStep>,
    /// Also list system schemas and tables such as pg_catalog or information_schema.
    #[serde(default)]
    include_system: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct DescribeTableArgs {
    /// The data source's name, as list_data_sources gives it (or its id).
    data_source: String,
    /// The table's name.
    table: String,
    /// The table's schema (on MySQL and MariaDB, its database). Leave it out for the
    /// connection's default schema.
    schema: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct DefinitionArgs {
    /// The data source's name, as list_data_sources gives it (or its id).
    data_source: String,
    /// The object's `path` from list_objects; only nodes with has_definition have one.
    path: Vec<PathStep>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct StatementArgs {
    /// The data source's name, as list_data_sources gives it (or its id).
    data_source: String,
    /// One statement that only reads, such as a SELECT, in the data source's dialect.
    sql: String,
}

/// A failed call, as the client sees it: enough to fix the call and try again.
#[derive(Debug)]
struct ToolError {
    kind: &'static str,
    message: String,
    sqlstate: Option<String>,
    /// Byte offset into the SQL the client sent.
    position: Option<usize>,
}

impl ToolError {
    fn refused(message: impl Into<String>) -> Self {
        Self::plain("refused", message)
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::plain("invalid_arguments", message)
    }

    fn plain(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            sqlstate: None,
            position: None,
        }
    }

    fn from_run(error: RunError, prefix_len: usize) -> Self {
        match error {
            RunError::Driver(DriverError::Statement {
                source,
                position,
                sqlstate,
            }) => Self {
                kind: "database",
                message: describe_error(source.as_ref()),
                sqlstate,
                position: position.and_then(|position| position.checked_sub(prefix_len)),
            },
            RunError::Driver(error @ (DriverError::Connect(_) | DriverError::Connection(_))) => {
                Self::plain("connection", describe_error(&error))
            }
            RunError::Driver(error @ DriverError::InvalidPath(_)) => {
                Self::invalid(describe_error(&error))
            }
            error @ RunError::TimedOut(_) => Self::plain("timeout", describe_error(&error)),
        }
    }

    fn into_result(self) -> CallToolResult {
        CallToolResult::structured_error(json!({
            "error": {
                "kind": self.kind,
                "message": self.message,
                "sqlstate": self.sqlstate,
                "position": self.position,
            }
        }))
    }
}

type ToolResult = Result<CallToolResult, ErrorData>;

/// Turns a tool body's outcome into what rmcp sends: domain failures are tool results with
/// `isError`, so the model reads them, rather than protocol errors.
fn reply(outcome: Result<Value, ToolError>) -> ToolResult {
    Ok(match outcome {
        Ok(value) => CallToolResult::structured(value),
        Err(error) => error.into_result(),
    })
}

#[tool_router]
impl DbdeskMcp {
    pub(crate) fn new(project: ServedProject, connections: Arc<Connections>, audit: Audit) -> Self {
        Self {
            inner: Arc::new(Inner {
                project,
                connections,
                audit,
            }),
            tool_router: Self::tool_router(),
        }
    }

    /// Lists the project's data sources: their names (which the other tools take), engines
    /// and server versions once connected, and the limits every call runs under.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn list_data_sources(&self) -> ToolResult {
        self.inner.audit.call("list_data_sources", None, None);
        let project = &self.inner.project;
        let data_sources: Vec<Value> = project
            .data_sources
            .iter()
            .map(|data_source| {
                json!({
                    "name": data_source.name,
                    "id": data_source.id.to_string(),
                    "engine": data_source.config.driver().display_name(),
                    "color": data_source.color.map(|color| color.display_name()),
                    "server": self
                        .inner
                        .connections
                        .server_info(data_source.id)
                        .map(|server| server.to_string()),
                })
            })
            .collect();
        reply(Ok(json!({
            "project": project.project.name,
            "data_sources": data_sources,
            "limits": {
                "max_rows": project.limits.max_rows,
                "statement_timeout_ms": project.limits.statement_timeout.as_millis(),
                "read_only": true,
            },
        })))
    }

    /// Lists one level of a data source's catalog, as dbdesk's explorer shows it: databases,
    /// schemas, folders of tables and views, then tables, then their columns, indexes and
    /// constraints. Pass a node's `path` to list its children.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn list_objects(&self, Parameters(args): Parameters<ListObjectsArgs>) -> ToolResult {
        reply(self.list_objects_json(args).await)
    }

    /// Describes a table: its columns with their types, its primary key, its indexes and its
    /// constraints.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn describe_table(&self, Parameters(args): Parameters<DescribeTableArgs>) -> ToolResult {
        reply(self.describe_table_json(args).await)
    }

    /// The source of a view, function, trigger, index or other object that has one
    /// (`has_definition` in list_objects).
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn get_definition(&self, Parameters(args): Parameters<DefinitionArgs>) -> ToolResult {
        reply(self.definition_json(args).await)
    }

    /// Runs one statement that only reads, such as a SELECT, in a read-only transaction that
    /// is then rolled back. Returns at most the project's row limit, with `truncated` set
    /// when there were more rows; cells are text, or null.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn query(&self, Parameters(args): Parameters<StatementArgs>) -> ToolResult {
        reply(self.statement_json("query", args, false).await)
    }

    /// Shows how the database would run one read statement, without running it.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn explain(&self, Parameters(args): Parameters<StatementArgs>) -> ToolResult {
        reply(self.statement_json("explain", args, true).await)
    }
}

impl DbdeskMcp {
    fn data_source(&self, tool: &str, name: &str) -> Result<&DataSource, ToolError> {
        self.inner.project.data_source(name).ok_or_else(|| {
            let known: Vec<&str> = self
                .inner
                .project
                .data_sources
                .iter()
                .map(|data_source| data_source.name.as_str())
                .collect();
            let message = format!(
                "the project has no data source {name:?}; it has: {}",
                known.join(", ")
            );
            self.inner.audit.refused(tool, name, &message);
            ToolError::invalid(message)
        })
    }

    async fn list_objects_json(&self, args: ListObjectsArgs) -> Result<Value, ToolError> {
        let tool = "list_objects";
        let data_source = self.data_source(tool, &args.data_source)?;
        self.inner.audit.call(tool, Some(&data_source.name), None);
        let path =
            catalog::path(&args.path).map_err(|error| ToolError::invalid(error.to_string()))?;
        let nodes = self
            .on_connection(tool, data_source, move |connection| {
                Box::pin(async move { connection.children(&path).await })
            })
            .await?;
        let objects: Vec<NodeJson> = nodes
            .iter()
            .filter(|node| args.include_system || !node.system)
            .map(NodeJson::from)
            .collect();
        Ok(json!({ "objects": objects }))
    }

    async fn describe_table_json(&self, args: DescribeTableArgs) -> Result<Value, ToolError> {
        let tool = "describe_table";
        let data_source = self.data_source(tool, &args.data_source)?;
        self.inner.audit.call(tool, Some(&data_source.name), None);
        let database = data_source.config.database().map(str::to_owned);
        let described = self
            .on_connection(tool, data_source, move |connection| {
                Box::pin(async move {
                    describe(
                        connection,
                        database.as_deref(),
                        args.schema.as_deref(),
                        &args.table,
                    )
                    .await
                })
            })
            .await?;
        described.ok_or_else(|| {
            ToolError::invalid(
                "there is no such table (views have no description; try get_definition)",
            )
        })
    }

    async fn definition_json(&self, args: DefinitionArgs) -> Result<Value, ToolError> {
        let tool = "get_definition";
        let data_source = self.data_source(tool, &args.data_source)?;
        self.inner.audit.call(tool, Some(&data_source.name), None);
        let path =
            catalog::path(&args.path).map_err(|error| ToolError::invalid(error.to_string()))?;
        let definition = self
            .on_connection(tool, data_source, move |connection| {
                Box::pin(async move { connection.definition(&path).await })
            })
            .await?;
        Ok(json!({ "definition": definition }))
    }

    /// `query` and `explain`: classify, refuse anything but reading, run, record.
    async fn statement_json(
        &self,
        tool: &'static str,
        args: StatementArgs,
        explain: bool,
    ) -> Result<Value, ToolError> {
        let data_source = self.data_source(tool, &args.data_source)?;
        let dialect = Dialect::from(data_source.config.driver());
        let sql = args.sql.trim().trim_end_matches(';').trim_end();
        let kind = match classify(sql, dialect) {
            Ok(kind) => kind,
            Err(unclassified) => {
                let reason = unclassified.to_string();
                self.inner.audit.refused(tool, &data_source.name, &reason);
                return Err(ToolError::refused(reason));
            }
        };
        self.inner
            .audit
            .call(tool, Some(&data_source.name), Some(kind));
        if kind != StatementKind::Read {
            let reason = format!(
                "this statement {}, and this project only allows reading",
                kind.description()
            );
            self.inner.audit.refused(tool, &data_source.name, &reason);
            return Err(ToolError::refused(reason));
        }

        let prefix = if explain {
            let server = self.inner.connections.server_info(data_source.id);
            dialect.explain_prefix(server.as_ref())
        } else {
            ""
        };
        let statement = format!("{prefix}{sql}");
        let max_rows = self.inner.project.limits.max_rows;
        let started = Timestamp::now();
        let clock = Instant::now();
        let to_run = statement.clone();
        let outcome = self
            .run_statement(data_source, move |connection| {
                Box::pin(async move { read_only::fetch(connection, &to_run, max_rows).await })
            })
            .await;
        let elapsed = clock.elapsed();
        self.inner
            .audit
            .record(Ran {
                data_source,
                sql: &statement,
                started,
                elapsed,
                rows: outcome.as_ref().ok().map(|rows| rows.rows.len() as u64),
                outcome: match &outcome {
                    Ok(_) => HistoryOutcome::Ok,
                    Err(RunError::TimedOut(_)) => HistoryOutcome::Cancelled,
                    Err(_) => HistoryOutcome::Error,
                },
            })
            .await;
        let rows = outcome.map_err(|error| self.failure(tool, data_source, error, prefix.len()))?;
        Ok(rows_json(&rows, elapsed))
    }

    async fn on_connection<T: Send>(
        &self,
        tool: &str,
        data_source: &DataSource,
        work: impl for<'c> FnOnce(&'c mut dyn Connection) -> Work<'c, T>,
    ) -> Result<T, ToolError> {
        self.run_statement(data_source, work)
            .await
            .map_err(|error| self.failure(tool, data_source, error, 0))
    }

    fn failure(
        &self,
        tool: &str,
        data_source: &DataSource,
        error: RunError,
        prefix_len: usize,
    ) -> ToolError {
        let error = ToolError::from_run(error, prefix_len);
        self.inner.audit.failed(
            tool,
            &data_source.name,
            &error.message,
            error.sqlstate.as_deref(),
        );
        error
    }

    /// Runs `work` on the data source's connection inside the read-only transaction, under
    /// the time limit. A connection the server closed while it sat idle fails on `BEGIN`;
    /// that is retried once on a new connection, before `work` has run.
    async fn run_statement<T: Send>(
        &self,
        data_source: &DataSource,
        work: impl for<'c> FnOnce(&'c mut dyn Connection) -> Work<'c, T>,
    ) -> Result<T, RunError> {
        let Some(mode) = read_only::mode(data_source.config.driver()) else {
            // `ServedProject` leaves such data sources out, so no call can name one.
            unreachable!("a served data source without a read-only mode");
        };
        let timeout = self.inner.project.limits.statement_timeout;
        let mut retried = false;
        let mut connection = loop {
            let mut connection = self
                .inner
                .connections
                .checkout(data_source.id)
                .await
                .map_err(|error| RunError::Driver(DriverError::Connect(Box::new(error))))?;
            match read_only::begin(&mut *connection, mode, timeout).await {
                Ok(()) => break connection,
                Err(DriverError::Connection(_)) if !retried => {
                    tracing::info!(data_source = %data_source.name, "reconnecting");
                    connection.discard();
                    retried = true;
                }
                Err(error) => {
                    let outcome = read_only::end(&mut *connection, mode, Err(error.into())).await;
                    if outcome.as_ref().is_err_and(RunError::breaks_connection) {
                        connection.discard();
                    }
                    return outcome;
                }
            }
        };
        let cancel = connection.cancel_handle();
        let outcome = read_only::watched(cancel, timeout, work(&mut *connection)).await;
        let outcome = read_only::end(&mut *connection, mode, outcome).await;
        if outcome.as_ref().is_err_and(RunError::breaks_connection) {
            connection.discard();
        }
        outcome
    }
}

fn rows_json(rows: &Rows, elapsed: std::time::Duration) -> Value {
    let columns: Vec<Value> = rows
        .columns
        .iter()
        .map(|column| json!({ "name": column.name, "type": column.type_name }))
        .collect();
    json!({
        "columns": columns,
        "rows": rows.rows,
        "row_count": rows.rows.len(),
        "truncated": rows.truncated,
        "elapsed_ms": elapsed.as_millis(),
    })
}

/// A table's description: the driver's for its name and key, and the explorer's catalog
/// levels for column types, indexes and constraints. A name given as typed is tried first,
/// then as an unquoted name the database folds to its own case.
async fn describe(
    connection: &mut dyn Connection,
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
) -> Result<Option<Value>, DriverError> {
    let identifier = |name: &str, quoted| Identifier {
        name: name.to_owned(),
        quoted,
    };
    let mut description = None;
    for quoted in [true, false] {
        let name = RelationName {
            schema: schema.map(|schema| identifier(schema, quoted)),
            name: identifier(table, quoted),
        };
        description = connection.describe_table(&name).await?;
        if description.is_some() {
            break;
        }
    }
    let Some(description) = description else {
        return Ok(None);
    };
    let mut table_path = CatalogPath::root();
    if let Some(database) = database {
        table_path = table_path.child(NodeKind::Database, database);
    }
    let table_path = table_path
        .child(NodeKind::Schema, &description.schema)
        .child(
            NodeKind::Folder(dbdesk_core::FolderKind::Tables),
            dbdesk_core::FolderKind::Tables.label(),
        )
        .child(NodeKind::Table, &description.name);
    let mut parts = serde_json::Map::new();
    for folder in connection.children(&table_path).await? {
        let Some(NodeKind::Folder(kind)) = folder.kind() else {
            continue;
        };
        let children = connection.children(&folder.path).await?;
        parts.insert(
            kind.label().to_owned(),
            Value::from(named_details(&children)),
        );
    }
    let mut value = json!({
        "schema": description.schema,
        "name": description.name,
        "primary_key": description.primary_key,
    });
    if let Value::Object(object) = &mut value {
        object.extend(parts);
    }
    Ok(Some(value))
}

fn named_details(nodes: &[CatalogNode]) -> Vec<Value> {
    nodes
        .iter()
        .map(|node| json!({ "name": node.label, "detail": node.detail }))
        .collect()
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DbdeskMcp {
    fn get_info(&self) -> ServerConfig {
        let project = &self.inner.project;
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("dbdesk", env!("CARGO_PKG_VERSION")))
            .with_instructions(format!(
                "Databases of the dbdesk project \"{}\". Start with list_data_sources; the other \
                 tools take a data source's name. Everything is read-only: query and explain run \
                 one statement that only reads, in a read-only transaction, return at most {} \
                 rows, and are cancelled after {} s. Statements that change anything are refused.",
                project.project.name,
                project.limits.max_rows,
                project.limits.statement_timeout.as_secs()
            ))
    }
}
