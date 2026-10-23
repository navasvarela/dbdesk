//! Describing a table for editing its rows.

use dbdesk_core::{DriverError, RelationName, TableDescription};
use rusqlite::{Connection, OptionalExtension, params};

use crate::connection_error;

/// Finds the table the way SQLite resolves an unqualified name (`temp` first, then `main`,
/// then attached databases), ignoring case as SQLite does, then reads its columns and
/// primary key. Views, virtual and shadow tables aren't plain tables.
pub(crate) fn describe_table(
    connection: &Connection,
    table: &RelationName,
) -> Result<Option<TableDescription>, DriverError> {
    let schema = table.schema.as_ref().map(|schema| schema.name.as_str());
    let found: Option<(String, String, String)> = connection
        .query_row(
            "SELECT schema, name, type FROM pragma_table_list \
             WHERE name = ?1 COLLATE NOCASE AND (?2 IS NULL OR schema = ?2 COLLATE NOCASE) \
             ORDER BY schema = 'temp' DESC, schema = 'main' DESC LIMIT 1",
            params![table.name.name, schema],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(connection_error)?;
    let Some((schema, name, _)) = found.filter(|(_, _, kind)| kind == "table") else {
        return Ok(None);
    };
    let mut statement = connection
        .prepare("SELECT name, pk FROM pragma_table_info(?1, ?2) ORDER BY cid")
        .map_err(connection_error)?;
    let mut columns: Vec<(String, i64)> = statement
        .query_map(params![name, schema], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(connection_error)?
        .collect::<Result<_, _>>()
        .map_err(connection_error)?;
    let names = columns.iter().map(|(name, _)| name.clone()).collect();
    // `pk` is the column's 1-based position in the key, 0 outside it.
    columns.retain(|(_, position)| *position > 0);
    columns.sort_by_key(|(_, position)| *position);
    Ok(Some(TableDescription {
        schema,
        name,
        columns: names,
        primary_key: columns.into_iter().map(|(name, _)| name).collect(),
    }))
}
