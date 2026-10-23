//! Describing a table for editing its rows.

use dbdesk_core::{DriverError, RelationName, TableDescription};
use tokio_postgres::Client;

use crate::connection_error;

/// Kinds of `pg_class` rows whose rows can be updated: ordinary and partitioned tables.
const PLAIN_TABLE_KINDS: [&str; 2] = ["r", "p"];

/// Resolves `table` with `to_regclass`, which applies the search path and folds unquoted
/// names to lowercase exactly as a statement would, then reads its columns and primary key.
pub(crate) async fn describe_table(
    client: &Client,
    table: &RelationName,
) -> Result<Option<TableDescription>, DriverError> {
    let written = table.to_sql();
    let found = client
        .query(
            "SELECT n.nspname, c.relname, c.relkind::text \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.oid = to_regclass($1)",
            &[&written],
        )
        .await
        .map_err(connection_error)?;
    let Some(row) = found.first() else {
        return Ok(None);
    };
    let kind: String = row.try_get(2).map_err(connection_error)?;
    if !PLAIN_TABLE_KINDS.contains(&kind.as_str()) {
        return Ok(None);
    }
    let columns = client
        .query(
            "SELECT a.attname, array_position(i.indkey::int2[], a.attnum) \
             FROM pg_attribute a \
             LEFT JOIN pg_index i ON i.indrelid = a.attrelid AND i.indisprimary \
             WHERE a.attrelid = to_regclass($1) AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            &[&written],
        )
        .await
        .map_err(connection_error)?;
    let mut description = TableDescription {
        schema: row.try_get(0).map_err(connection_error)?,
        name: row.try_get(1).map_err(connection_error)?,
        columns: Vec::new(),
        primary_key: Vec::new(),
    };
    let mut key = Vec::new();
    for column in &columns {
        let name: String = column.try_get(0).map_err(connection_error)?;
        let key_position: Option<i32> = column.try_get(1).map_err(connection_error)?;
        if let Some(position) = key_position {
            key.push((position, name.clone()));
        }
        description.columns.push(name);
    }
    key.sort();
    description.primary_key = key.into_iter().map(|(_, name)| name).collect();
    Ok(Some(description))
}
