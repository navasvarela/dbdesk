//! Describing a table for editing its rows.
//!
//! Through `SHOW` statements rather than `information_schema`: they resolve a name exactly as
//! a statement on this connection would (whether table names match ignoring case depends on
//! the server's `lower_case_table_names`), and they see temporary tables, which
//! `information_schema` doesn't list.

use dbdesk_core::{DriverError, RelationName, TableDescription};
use mysql_async::Conn;
use mysql_async::prelude::Queryable;

use crate::{MYSQL_QUOTING, connection_error};

/// `ER_NO_SUCH_TABLE`.
const NO_SUCH_TABLE: u16 = 1146;
/// `ER_NO_DB_ERROR`: an unqualified name on a connection without a default database.
const NO_DATABASE_SELECTED: u16 = 1046;
/// `ER_BAD_DB_ERROR`: a qualified name in a database that doesn't exist.
const NO_SUCH_DATABASE: u16 = 1049;

pub(crate) async fn describe_table(
    conn: &mut Conn,
    table: &RelationName,
) -> Result<Option<TableDescription>, DriverError> {
    let name = table.to_sql(MYSQL_QUOTING);
    // `SHOW CREATE TABLE` answers for views too, naming its first column `View` instead of
    // `Table`, and gives the table's name as stored.
    let Some(created) = show(conn, &format!("SHOW CREATE TABLE {name}")).await? else {
        return Ok(None);
    };
    let Some(first) = created.first() else {
        return Ok(None);
    };
    if first.columns_ref().first().map(|column| column.name_str()) != Some("Table".into()) {
        return Ok(None);
    }
    let stored_name: Option<String> = first.get_opt(0).and_then(Result::ok);
    let schema = match &table.schema {
        Some(schema) => Some(schema.name.clone()),
        None => conn
            .query_first::<Option<String>, _>("SELECT DATABASE()")
            .await
            .map_err(connection_error)?
            .flatten(),
    };
    let (Some(schema), Some(stored_name)) = (schema, stored_name) else {
        return Ok(None);
    };
    let columns = field_values(conn, &format!("SHOW COLUMNS FROM {name}"), "Field").await?;
    let primary_key = field_values(
        conn,
        &format!("SHOW KEYS FROM {name} WHERE Key_name = 'PRIMARY'"),
        "Column_name",
    )
    .await?;
    Ok(Some(TableDescription {
        schema,
        name: stored_name,
        columns,
        primary_key,
    }))
}

/// Runs a `SHOW` statement, or `None` if the table or its database doesn't exist.
async fn show(conn: &mut Conn, sql: &str) -> Result<Option<Vec<mysql_async::Row>>, DriverError> {
    match conn.query(sql).await {
        Ok(rows) => Ok(Some(rows)),
        Err(mysql_async::Error::Server(error))
            if [NO_SUCH_TABLE, NO_DATABASE_SELECTED, NO_SUCH_DATABASE].contains(&error.code) =>
        {
            Ok(None)
        }
        Err(error) => Err(connection_error(error)),
    }
}

/// One column of a `SHOW` result, in the order the server lists the rows: `SHOW COLUMNS`
/// in table order, `SHOW KEYS` in key order.
async fn field_values(
    conn: &mut Conn,
    sql: &str,
    column: &str,
) -> Result<Vec<String>, DriverError> {
    let rows = show(conn, sql).await?.unwrap_or_default();
    rows.iter()
        .map(|row| {
            row.get_opt::<String, _>(column)
                .and_then(Result::ok)
                .ok_or_else(|| connection_error(format!("{sql} returned no {column}")))
        })
        .collect()
}
