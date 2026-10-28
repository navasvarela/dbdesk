//! Describing a table for editing its rows.
//!
//! An unquoted name matches ignoring case, as DuckDB resolves it, and a quoted one exactly.
//! Without a schema, a name means the session's temporary table of that name if there is one,
//! and otherwise the table in the current schema of the current database.

use dbrover_core::{
    CheckConstraint, ColumnDescription, DriverError, ForeignKey, ForeignKeyConstraint, Identifier,
    ReferentialAction, RelationName, TableConstraints, TableDescription, TableName,
    UniqueConstraint,
};

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
    let foreign_keys = keys_where(connection, OUTGOING, &names)?;
    let referenced_by = keys_where(connection, INCOMING, &names)?;
    let [_, schema, name] = found;
    Ok(Some(TableDescription {
        schema,
        name,
        columns,
        primary_key,
        foreign_keys,
        referenced_by,
    }))
}

/// Keys whose table is the one asked about: the table's own foreign keys.
const OUTGOING: &str = "c.table_name = ?";
/// Keys that refer to the table asked about. DuckDB keeps the referenced table's name as the
/// `REFERENCES` clause wrote it, and matches names ignoring case.
const INCOMING: &str = "lower(c.referenced_table) = lower(?)";

/// The foreign keys `duckdb_constraints()` lists in the table's schema where `condition`
/// holds, each column list in key order. A key refers to a table in its own schema, which is
/// reported by its stored name.
fn keys_where(
    connection: &duckdb::Connection,
    condition: &str,
    names: &[&str],
) -> Result<Vec<ForeignKey>, DriverError> {
    let sql = format!(
        "SELECT c.table_name, c.constraint_index, c.constraint_column_names[i], \
         coalesce(t.table_name, c.referenced_table), c.referenced_column_names[i] \
         FROM duckdb_constraints() c \
         LEFT JOIN duckdb_tables() t ON t.database_name = c.database_name \
           AND t.schema_name = c.schema_name AND lower(t.table_name) = lower(c.referenced_table) \
         CROSS JOIN range(1, len(c.constraint_column_names) + 1) r(i) \
         WHERE c.database_name = ? AND c.schema_name = ? AND {condition} \
           AND c.constraint_type = 'FOREIGN KEY' \
         ORDER BY c.table_name, c.constraint_index, i"
    );
    let mut statement = connection.prepare(&sql).map_err(connection_error)?;
    let rows = statement
        .query_map(duckdb::params_from_iter(names), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(connection_error)?;
    let schema = names.get(1).copied().unwrap_or_default();
    let table_name = |name: String| TableName {
        schema: schema.to_owned(),
        name,
    };
    let mut keys: Vec<(i64, ForeignKey)> = Vec::new();
    for row in rows {
        let (table, index, column, referenced, referenced_column) =
            row.map_err(connection_error)?;
        match keys.last_mut() {
            Some((last, key)) if *last == index && key.table.name == table => {
                key.columns.push(column);
                key.referenced_columns.push(referenced_column);
            }
            _ => keys.push((
                index,
                ForeignKey {
                    table: table_name(table),
                    columns: vec![column],
                    referenced: table_name(referenced),
                    referenced_columns: vec![referenced_column],
                },
            )),
        }
    }
    Ok(keys.into_iter().map(|(_, key)| key).collect())
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
                identity: None,
            })
        })
        .map_err(connection_error)?;
    rows.collect::<Result<_, _>>().map_err(connection_error)
}

/// The table's keys and checks, from `duckdb_constraints()`, one row per column of each. A
/// foreign key points into the table's own schema, with no actions: DuckDB has none.
pub(crate) fn describe_constraints(
    connection: &duckdb::Connection,
    table: &TableName,
) -> Result<TableConstraints, DriverError> {
    let mut statement = connection
        .prepare(
            "SELECT c.constraint_index, c.constraint_type, c.constraint_name, c.expression, \
             c.referenced_table, c.constraint_column_names[i], c.referenced_column_names[i] \
             FROM duckdb_constraints() c \
             CROSS JOIN range(1, greatest(len(c.constraint_column_names), 1) + 1) r(i) \
             WHERE c.database_name = current_database() AND c.schema_name = ? \
             AND c.table_name = ? \
             AND c.constraint_type IN ('PRIMARY KEY', 'UNIQUE', 'FOREIGN KEY', 'CHECK') \
             ORDER BY c.constraint_index, i",
        )
        .map_err(connection_error)?;
    type ConstraintColumn = (
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let rows = statement
        .query_map([&table.schema, &table.name], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })
        .map_err(connection_error)?
        .collect::<Result<Vec<ConstraintColumn>, _>>()
        .map_err(connection_error)?;
    let mut constraints = TableConstraints::default();
    let mut last = None;
    for (index, kind, name, expression, referenced, column, referenced_column) in rows {
        let first_column = last != Some(index);
        last = Some(index);
        match kind.as_str() {
            "PRIMARY KEY" => constraints.primary_key_name = name,
            "UNIQUE" if first_column => constraints.unique_keys.push(UniqueConstraint {
                name,
                columns: column.into_iter().collect(),
            }),
            "UNIQUE" => {
                if let Some(key) = constraints.unique_keys.last_mut() {
                    key.columns.extend(column);
                }
            }
            "FOREIGN KEY" if first_column => {
                constraints.foreign_keys.push(ForeignKeyConstraint {
                    name,
                    columns: column.into_iter().collect(),
                    referenced: TableName {
                        schema: table.schema.clone(),
                        name: referenced.unwrap_or_default(),
                    },
                    referenced_columns: referenced_column.into_iter().collect(),
                    on_delete: ReferentialAction::NoAction,
                    on_update: ReferentialAction::NoAction,
                });
            }
            "FOREIGN KEY" => {
                if let Some(key) = constraints.foreign_keys.last_mut() {
                    key.columns.extend(column);
                    key.referenced_columns.extend(referenced_column);
                }
            }
            _ if first_column => constraints.checks.push(CheckConstraint {
                name,
                expression: expression.unwrap_or_default(),
            }),
            _ => {}
        }
    }
    Ok(constraints)
}
