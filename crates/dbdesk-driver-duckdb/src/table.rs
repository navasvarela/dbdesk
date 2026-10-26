//! Describing a table for editing its rows.
//!
//! An unquoted name matches ignoring case, as DuckDB resolves it, and a quoted one exactly.
//! Without a schema, a name means the session's temporary table of that name if there is one,
//! and otherwise the table in the current schema of the current database.

use dbdesk_core::{ColumnDescription, DriverError, Identifier, RelationName, TableDescription};

use crate::connection_error;

/// Where DuckDB keeps a session's temporary tables.
const TEMPORARY_DATABASE: &str = "temp";

pub(crate) fn describe_table(
    connection: &duckdb::Connection,
    table: &RelationName,
) -> Result<Option<TableDescription>, DriverError> {
    let matches = |column: &str, name: &Identifier| {
        if name.quoted {
            format!("{column} = ?")
        } else {
            format!("lower({column}) = lower(?)")
        }
    };
    let (schema_match, schema) = match &table.schema {
        Some(schema) => (matches("schema_name", schema), schema.name.as_str()),
        None => ("schema_name = current_schema()".to_owned(), ""),
    };
    let sql = format!(
        "SELECT database_name, schema_name, table_name FROM duckdb_tables() \
         WHERE {} AND {schema_match} \
         AND database_name IN (current_database(), '{TEMPORARY_DATABASE}') \
         ORDER BY database_name = '{TEMPORARY_DATABASE}' DESC LIMIT 1",
        matches("table_name", &table.name),
    );
    let mut params = vec![table.name.name.as_str()];
    if table.schema.is_some() {
        params.push(schema);
    }
    let mut statement = connection.prepare(&sql).map_err(connection_error)?;
    let mut rows = statement
        .query(duckdb::params_from_iter(&params))
        .map_err(connection_error)?;
    let Some(row) = rows.next().map_err(connection_error)? else {
        return Ok(None);
    };
    let found: [String; 3] = [
        row.get(0).map_err(connection_error)?,
        row.get(1).map_err(connection_error)?,
        row.get(2).map_err(connection_error)?,
    ];
    let names: Vec<&str> = found.iter().map(String::as_str).collect();
    let columns = columns(connection, &names)?;
    let primary_key = names_of(
        connection,
        "SELECT unnest(constraint_column_names) FROM duckdb_constraints() \
         WHERE database_name = ? AND schema_name = ? AND table_name = ? \
         AND constraint_type = 'PRIMARY KEY'",
        &names,
    )?;
    let [_, schema, name] = found;
    Ok(Some(TableDescription {
        schema,
        name,
        columns,
        primary_key,
    }))
}

fn names_of(
    connection: &duckdb::Connection,
    sql: &str,
    names: &[&str],
) -> Result<Vec<String>, DriverError> {
    let mut statement = connection.prepare(sql).map_err(connection_error)?;
    let rows = statement
        .query_map(duckdb::params_from_iter(names), |row| row.get(0))
        .map_err(connection_error)?;
    rows.collect::<Result<_, _>>().map_err(connection_error)
}

fn columns(
    connection: &duckdb::Connection,
    names: &[&str],
) -> Result<Vec<ColumnDescription>, DriverError> {
    let mut statement = connection
        .prepare(
            "SELECT column_name, data_type, is_nullable, column_default FROM duckdb_columns() \
             WHERE database_name = ? AND schema_name = ? AND table_name = ? \
             ORDER BY column_index",
        )
        .map_err(connection_error)?;
    let rows = statement
        .query_map(duckdb::params_from_iter(names), |row| {
            Ok(ColumnDescription {
                name: row.get(0)?,
                data_type: row.get(1)?,
                nullable: row.get(2)?,
                default: row.get(3)?,
            })
        })
        .map_err(connection_error)?;
    rows.collect::<Result<_, _>>().map_err(connection_error)
}
