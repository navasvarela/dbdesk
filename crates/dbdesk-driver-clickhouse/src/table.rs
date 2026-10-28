//! Describing a table for editing its rows.
//!
//! A ClickHouse table has no key that tells its rows apart: its `PRIMARY KEY` is its sort
//! order and may repeat. So a description never has a primary key, and the grid keeps the
//! table's rows read-only.

use clickhouse::Client;
use dbdesk_core::{ColumnDescription, DriverError, RelationName, TableDescription};

use crate::CLICKHOUSE_QUOTING;
use crate::execution::rows;

/// Finds the table as a statement on the session would: a temporary table (database `''`)
/// first, since it hides a table of the same name in the current database. Names match
/// exactly, quoted or not.
const RESOLVE: &str = "SELECT database FROM system.tables \
     WHERE name = {name:String} AND engine NOT IN ('View', 'MaterializedView', 'Dictionary') \
     AND (database = {schema:String} OR (is_temporary AND {unqualified:Bool})) \
     ORDER BY is_temporary DESC LIMIT 1";

pub(crate) async fn describe_table(
    session: &Client,
    table: &RelationName,
) -> Result<Option<TableDescription>, DriverError> {
    let current;
    let (schema, unqualified) = match &table.schema {
        Some(schema) => (schema.name.as_str(), "false"),
        None => {
            current = crate::execution::single_value(session, "SELECT currentDatabase()")
                .await
                .map_err(crate::connection_error)?;
            (current.as_str(), "true")
        }
    };
    let name = table.name.name.as_str();
    let params = [
        ("name", name),
        ("schema", schema),
        ("unqualified", unqualified),
    ];
    let found = rows(session, RESOLVE, &params).await?;
    let Some(database) = found.first().and_then(|row| row.first().cloned().flatten()) else {
        return Ok(None);
    };
    let quoted = if database.is_empty() {
        CLICKHOUSE_QUOTING.identifier(name)
    } else {
        format!(
            "{}.{}",
            CLICKHOUSE_QUOTING.identifier(&database),
            CLICKHOUSE_QUOTING.identifier(name)
        )
    };
    let columns = rows(session, &format!("DESCRIBE TABLE {quoted}"), &[])
        .await?
        .into_iter()
        .filter_map(described_column)
        .collect();
    Ok(Some(TableDescription {
        schema: database,
        name: name.to_owned(),
        columns,
        primary_key: Vec::new(),
        // ClickHouse has no foreign keys.
        foreign_keys: Vec::new(),
        referenced_by: Vec::new(),
    }))
}

/// A row of `DESCRIBE TABLE`: the name, type, kind of default (`DEFAULT`, `MATERIALIZED`,
/// `ALIAS` or empty) and its expression, then comments and codecs. A column computed by
/// `MATERIALIZED` or `ALIAS` has no default to report.
fn described_column(row: Vec<Option<String>>) -> Option<ColumnDescription> {
    let mut fields = row.into_iter();
    let name = fields.next().flatten()?;
    let data_type = fields.next().flatten().unwrap_or_default();
    let default_kind = fields.next().flatten();
    let expression = fields.next().flatten();
    Some(ColumnDescription {
        nullable: is_nullable(&data_type),
        default: expression.filter(|_| default_kind.as_deref() == Some("DEFAULT")),
        identity: None,
        name,
        data_type,
    })
}

/// ClickHouse's types hold NULL only when wrapped in `Nullable`, which `LowCardinality` may
/// wrap in turn.
fn is_nullable(data_type: &str) -> bool {
    data_type.starts_with("Nullable(") || data_type.starts_with("LowCardinality(Nullable(")
}
