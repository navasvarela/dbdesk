//! The object helpers' DDL on the explorer's connection: applying generated statements,
//! loading a table's, view's or role's `CREATE` statement for Copy DDL, and loading a table
//! for Modify table.

use std::time::Instant;

use dbdesk_core::{
    CatalogNode, CatalogPath, Connection, DriverError, FolderKind, Identifier, NodeKind,
    RelationName, TableDescription, describe_error,
};
use dbdesk_sql::{ColumnSpec, DdlOperation, ExistingTable, ObjectName, RoleSpec, TableSpec};
use dbdesk_store::{HistoryEntry, HistoryOutcome, HistorySource, redact_secrets};
use jiff::Timestamp;

use super::{Disconnected, SessionContext, Worker, transaction};
use crate::{DdlFailure, Event};

/// Runs `statements` and reports [`Event::DdlApplied`]: in one transaction where the
/// dialect's DDL is transactional, one after the other otherwise.
pub(super) async fn apply(
    worker: &mut Worker<'_>,
    statements: &[String],
) -> Result<(), Disconnected> {
    let context = worker.context;
    let result = if context.read_only.is_some() {
        tracing::warn!(data_source = %context.name(), "DDL refused: read-only");
        Err(DdlFailure {
            statement: None,
            message: format!(
                "Refused: \u{201c}{}\u{201d} is read-only, so nothing in it can be created or \
                 changed.",
                context.name()
            ),
            applied: 0,
        })
    } else if context.dialect().capabilities().transactional_ddl {
        in_transaction(worker, statements).await?
    } else {
        run_each(worker, statements).await?
    };
    match &result {
        Ok(()) => tracing::info!(
            data_source = %context.name(),
            "applied {} DDL statements",
            statements.len()
        ),
        Err(failure) => tracing::warn!(
            data_source = %context.name(),
            "DDL failed after {} statements stayed applied: {}",
            failure.applied,
            failure.message
        ),
    }
    context.send(Event::DdlApplied {
        data_source: context.data_source.id,
        result,
    });
    Ok(())
}

/// Opens a transaction, runs the statements and commits them, or rolls everything back at the
/// first failure.
async fn in_transaction(
    worker: &mut Worker<'_>,
    statements: &[String],
) -> Result<Result<(), DdlFailure>, Disconnected> {
    let unwrapped = |error: &DriverError| DdlFailure {
        statement: None,
        message: describe_error(error),
        applied: 0,
    };
    let begin = worker.context.dialect().begin_transaction();
    if let Err(error) = transaction::run_counting(worker, begin, None).await? {
        return Ok(Err(unwrapped(&error)));
    }
    let failure = match run_each(worker, statements).await? {
        Ok(()) => match transaction::run_counting(worker, "COMMIT", None).await? {
            Ok(_) => return Ok(Ok(())),
            Err(error) => unwrapped(&error),
        },
        Err(failure) => failure,
    };
    if let Err(error) = transaction::run_counting(worker, "ROLLBACK", None).await? {
        tracing::warn!(
            data_source = %worker.context.name(),
            "ROLLBACK failed after DDL failed: {}",
            describe_error(&error)
        );
    }
    Ok(Err(DdlFailure {
        applied: 0,
        ..failure
    }))
}

/// Runs the statements in order, recording each in the history, and stops at the first that
/// fails: the statements before it are applied, in whatever transaction is open.
async fn run_each(
    worker: &mut Worker<'_>,
    statements: &[String],
) -> Result<Result<(), DdlFailure>, Disconnected> {
    for (index, sql) in statements.iter().enumerate() {
        tracing::info!(data_source = %worker.context.name(), "{}", redact_secrets(sql));
        let started_at = Timestamp::now();
        let started = Instant::now();
        let result = transaction::run_counting(worker, sql, None).await?;
        let outcome = match &result {
            Ok(_) => HistoryOutcome::Ok,
            Err(_) => HistoryOutcome::Error,
        };
        worker.context.record(HistoryEntry {
            data_source: worker.context.data_source.id,
            executed_at: started_at,
            sql: sql.clone(),
            duration: Some(started.elapsed()),
            rows: None,
            outcome,
            source: HistorySource::ObjectHelper,
        });
        if let Err(error) = result {
            let message = match &error {
                DriverError::Statement { source, .. } => describe_error(source.as_ref()),
                other => describe_error(other),
            };
            return Ok(Err(DdlFailure {
                statement: Some(index),
                message,
                applied: index,
            }));
        }
    }
    Ok(Ok(()))
}

/// The constraints Copy DDL adds after a rebuilt `CREATE TABLE`, by the detail PostgreSQL's
/// and SQL Server's catalogs give them. The primary key is in the table's description already,
/// and so are NOT NULL (PostgreSQL 18 lists those as constraints too) and SQL Server's
/// defaults, in its columns.
const COPIED_CONSTRAINTS: &[&str] = &["foreign key", "unique", "check", "exclusion"];

/// Reports [`Event::DdlLoaded`] for the table or view at `path`.
pub(super) async fn load(
    context: &SessionContext,
    connection: &mut dyn Connection,
    path: CatalogPath,
) {
    let kind = path.last().map(|segment| segment.kind);
    let result =
        if kind == Some(NodeKind::Table) && context.dialect().capabilities().rebuilds_table_ddl {
            rebuild(context, connection, &path).await
        } else if kind == Some(NodeKind::Role) {
            rebuild_role(context, connection, &path).await
        } else {
            connection
                .definition(&path)
                .await
                .map_err(|error| describe_error(&error))
        };
    if let Err(error) = &result {
        tracing::warn!(data_source = %context.name(), "could not load the DDL of {path}: {error}");
    }
    context.send(Event::DdlLoaded {
        data_source: context.data_source.id,
        path,
        result,
    });
}

/// `CREATE TABLE` from the table's description, through the same builder as New table,
/// followed by an `ALTER TABLE … ADD CONSTRAINT` per constraint, as the catalog defines them:
/// the order `pg_dump` writes them in, so a foreign key to a table created later still works
/// once both exist.
async fn rebuild(
    context: &SessionContext,
    connection: &mut dyn Connection,
    path: &CatalogPath,
) -> Result<String, String> {
    let description = describe_at(connection, path).await?;
    let spec = TableSpec {
        name: ObjectName::new(Some(&description.schema), &description.name),
        columns: description
            .columns
            .into_iter()
            .map(ColumnSpec::from)
            .collect(),
        primary_key: description.primary_key,
        unique_keys: Vec::new(),
        foreign_keys: Vec::new(),
        checks: Vec::new(),
    };
    let mut statements = DdlOperation::CreateTable(spec)
        .statements(context.dialect())
        .map_err(|error| error.to_string())?;
    let constraints = connection
        .children(&path.child(
            NodeKind::Folder(FolderKind::Constraints),
            FolderKind::Constraints.label(),
        ))
        .await
        .map_err(|error| describe_error(&error))?;
    for constraint in constraints.iter().filter(|node| is_copied(node)) {
        let definition = connection
            .definition(&constraint.path)
            .await
            .map_err(|error| describe_error(&error))?;
        statements.push(definition.trim_end().trim_end_matches(';').to_owned());
    }
    Ok(statements
        .iter()
        .map(|statement| format!("{statement};\n"))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Describes the table at `path`, named exactly as the tree shows it.
async fn describe_at(
    connection: &mut dyn Connection,
    path: &CatalogPath,
) -> Result<TableDescription, String> {
    let not_a_table = || format!("{path} is not a table");
    let schema = path.name_of(NodeKind::Schema).ok_or_else(not_a_table)?;
    let table = path.name_of(NodeKind::Table).ok_or_else(not_a_table)?;
    let exactly = |name: &str| Identifier {
        name: name.to_owned(),
        quoted: true,
    };
    let relation = RelationName {
        schema: Some(exactly(schema)),
        name: exactly(table),
    };
    connection
        .describe_table(&relation)
        .await
        .map_err(|error| describe_error(&error))?
        .ok_or_else(not_a_table)
}

/// Reports [`Event::TableLoaded`] for the table at `path`: its description and constraints,
/// as Modify table starts from them.
pub(super) async fn load_table(
    context: &SessionContext,
    connection: &mut dyn Connection,
    path: CatalogPath,
) {
    let result = existing_table(context, connection, &path).await;
    if let Err(error) = &result {
        tracing::warn!(data_source = %context.name(), "could not load the table {path}: {error}");
    }
    context.send(Event::TableLoaded {
        data_source: context.data_source.id,
        path,
        result,
    });
}

async fn existing_table(
    context: &SessionContext,
    connection: &mut dyn Connection,
    path: &CatalogPath,
) -> Result<ExistingTable, String> {
    let description = describe_at(connection, path).await?;
    let constraints = connection
        .describe_constraints(&description.table_name())
        .await
        .map_err(|error| describe_error(&error))?;
    Ok(ExistingTable::from_catalog(
        description,
        constraints,
        context.dialect(),
    ))
}

/// `CREATE ROLE` for the role at `path`, from its description, through the same builder as
/// New role: everything but its password, which the catalog only shows to superusers and
/// which would be a secret on the clipboard.
async fn rebuild_role(
    context: &SessionContext,
    connection: &mut dyn Connection,
    path: &CatalogPath,
) -> Result<String, String> {
    let role = connection
        .describe_role(path)
        .await
        .map_err(|error| describe_error(&error))?;
    let statements = DdlOperation::CreateRole {
        name: role.name.clone(),
        spec: RoleSpec::from(&role),
        password: None,
    }
    .statements(context.dialect())
    .map_err(|error| error.to_string())?;
    Ok(statements
        .iter()
        .map(|statement| format!("{statement};\n"))
        .collect())
}

fn is_copied(constraint: &CatalogNode) -> bool {
    constraint.has_definition
        && constraint
            .detail
            .as_deref()
            .is_some_and(|detail| COPIED_CONSTRAINTS.contains(&detail))
}
