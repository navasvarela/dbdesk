//! Describing a table for editing its rows.
//!
//! Through `SHOW` statements rather than `information_schema`: they resolve a name exactly as
//! a statement on this connection would (whether table names match ignoring case depends on
//! the server's `lower_case_table_names`), and they see temporary tables, which
//! `information_schema` doesn't list. Foreign keys come from `information_schema` once the
//! table's stored name is known: temporary tables can't have any.

use std::collections::HashMap;

use dbdesk_core::{
    CheckConstraint, ColumnDescription, DriverError, ForeignKey, ForeignKeyConstraint, Identity,
    IdentityGeneration, ReferentialAction, RelationName, TableConstraints, TableDescription,
    TableName, UniqueConstraint,
};
use mysql_async::Conn;
use mysql_async::prelude::Queryable;

use crate::{Flavour, MYSQL_QUOTING, connection_error};

/// `ER_NO_SUCH_TABLE`.
const NO_SUCH_TABLE: u16 = 1146;
/// `ER_NO_DB_ERROR`: an unqualified name on a connection without a default database.
const NO_DATABASE_SELECTED: u16 = 1046;
/// `ER_BAD_DB_ERROR`: a qualified name in a database that doesn't exist.
const NO_SUCH_DATABASE: u16 = 1049;

pub(crate) async fn describe_table(
    conn: &mut Conn,
    table: &RelationName,
    flavour: Flavour,
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
    let mut columns = columns(conn, &format!("SHOW COLUMNS FROM {name}")).await?;
    let primary_key = field_values(
        conn,
        &format!("SHOW KEYS FROM {name} WHERE Key_name = 'PRIMARY'"),
        "Column_name",
    )
    .await?;
    let me = TableName {
        schema,
        name: stored_name,
    };
    let defaults = defaults(conn, &me, flavour).await?;
    for column in &mut columns {
        if let Some(default) = defaults.get(&column.name) {
            column.default.clone_from(default);
        }
    }
    let foreign_keys = keys_where(conn, OUTGOING, &me).await?;
    let referenced_by = keys_where(conn, INCOMING, &me).await?;
    Ok(Some(TableDescription {
        schema: me.schema,
        name: me.name,
        columns,
        primary_key,
        foreign_keys,
        referenced_by,
    }))
}

/// Each column's default as an SQL expression, which `SHOW COLUMNS` doesn't give: it prints a
/// literal default unquoted, and on MariaDB an expression just like a literal. A temporary
/// table isn't in `information_schema`, so its columns keep what `SHOW COLUMNS` says.
///
/// MySQL's `COLUMN_DEFAULT` is unquoted too, but says an expression is one in `EXTRA`
/// (`DEFAULT_GENERATED`), written back in parentheses as MySQL needs; MariaDB's is the SQL
/// already, with `NULL` for a default of `NULL`.
async fn defaults(
    conn: &mut Conn,
    table: &TableName,
    flavour: Flavour,
) -> Result<HashMap<String, Option<String>>, DriverError> {
    let rows: Vec<(String, Option<String>, String)> = conn
        .exec(
            "SELECT COLUMN_NAME, COLUMN_DEFAULT, EXTRA FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
            (&table.schema, &table.name),
        )
        .await
        .map_err(connection_error)?;
    Ok(rows
        .into_iter()
        .map(|(column, default, extra)| {
            let default = default.and_then(|default| match flavour {
                Flavour::MariaDb => (default != "NULL").then_some(default),
                Flavour::Mysql if extra.contains("DEFAULT_GENERATED") => {
                    Some(format!("({default})"))
                }
                Flavour::Mysql => Some(MYSQL_QUOTING.string_literal(&default)),
            });
            (column, default)
        })
        .collect())
}

/// Key columns whose table is the one asked about: the table's own foreign keys.
const OUTGOING: &str = "TABLE_SCHEMA = ? AND TABLE_NAME = ? AND REFERENCED_TABLE_NAME IS NOT NULL";
/// Key columns that refer to the table asked about: other tables' keys to it.
const INCOMING: &str = "REFERENCED_TABLE_SCHEMA = ? AND REFERENCED_TABLE_NAME = ?";

/// The foreign keys `KEY_COLUMN_USAGE` lists where `condition` holds for `table`, each
/// column list in key order.
async fn keys_where(
    conn: &mut Conn,
    condition: &str,
    table: &TableName,
) -> Result<Vec<ForeignKey>, DriverError> {
    type KeyColumn = (String, String, String, String, String, String, String);
    let columns: Vec<KeyColumn> = conn
        .exec(
            format!(
                "SELECT TABLE_SCHEMA, TABLE_NAME, CONSTRAINT_NAME, COLUMN_NAME, \
                 REFERENCED_TABLE_SCHEMA, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
                 FROM information_schema.KEY_COLUMN_USAGE WHERE {condition} \
                 ORDER BY TABLE_SCHEMA, TABLE_NAME, CONSTRAINT_NAME, ORDINAL_POSITION"
            ),
            (&table.schema, &table.name),
        )
        .await
        .map_err(connection_error)?;
    let mut keys: Vec<(String, ForeignKey)> = Vec::new();
    for (schema, name, constraint, column, referenced_schema, referenced_name, referenced) in
        columns
    {
        let key_table = TableName { schema, name };
        match keys.last_mut() {
            Some((last, key)) if *last == constraint && key.table == key_table => {
                key.columns.push(column);
                key.referenced_columns.push(referenced);
            }
            _ => keys.push((
                constraint,
                ForeignKey {
                    table: key_table,
                    columns: vec![column],
                    referenced: TableName {
                        schema: referenced_schema,
                        name: referenced_name,
                    },
                    referenced_columns: vec![referenced],
                },
            )),
        }
    }
    Ok(keys.into_iter().map(|(_, key)| key).collect())
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

/// `SHOW COLUMNS`'s rows, in table order. Its `Default` is the default's value, or for an
/// expression (MySQL 8.0.13 and later) the expression.
async fn columns(conn: &mut Conn, sql: &str) -> Result<Vec<ColumnDescription>, DriverError> {
    let rows = show(conn, sql).await?.unwrap_or_default();
    rows.iter()
        .map(|row| {
            let text = |column: &str| -> Option<String> {
                row.get_opt::<Option<String>, _>(column)
                    .and_then(Result::ok)
                    .flatten()
            };
            let missing = |column: &str| connection_error(format!("{sql} returned no {column}"));
            Ok(ColumnDescription {
                name: text("Field").ok_or_else(|| missing("Field"))?,
                data_type: text("Type").ok_or_else(|| missing("Type"))?,
                nullable: text("Null").as_deref() == Some("YES"),
                default: text("Default"),
                identity: text("Extra")
                    .is_some_and(|extra| extra.to_lowercase().contains("auto_increment"))
                    .then_some(Identity::counting(IdentityGeneration::ByDefault)),
            })
        })
        .collect()
}

/// One column of a `SHOW` result, in the order the server lists the rows: `SHOW KEYS` lists
/// them in key order.
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

/// The table's unique keys, foreign keys (with their actions) and checks, by name. Its primary
/// key is always called `PRIMARY`, and dropped without a name.
pub(crate) async fn describe_constraints(
    conn: &mut Conn,
    table: &TableName,
    flavour: Flavour,
) -> Result<TableConstraints, DriverError> {
    let at = (&table.schema, &table.name);
    let unique_columns: Vec<(String, String)> = conn
        .exec(
            "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME \
             FROM information_schema.TABLE_CONSTRAINTS c \
             JOIN information_schema.KEY_COLUMN_USAGE k \
               ON k.CONSTRAINT_SCHEMA = c.CONSTRAINT_SCHEMA \
               AND k.CONSTRAINT_NAME = c.CONSTRAINT_NAME AND k.TABLE_NAME = c.TABLE_NAME \
             WHERE c.TABLE_SCHEMA = ? AND c.TABLE_NAME = ? AND c.CONSTRAINT_TYPE = 'UNIQUE' \
             ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION",
            at,
        )
        .await
        .map_err(connection_error)?;
    let mut unique_keys: Vec<UniqueConstraint> = Vec::new();
    for (name, column) in unique_columns {
        match unique_keys.last_mut() {
            Some(key) if key.name.as_deref() == Some(name.as_str()) => key.columns.push(column),
            _ => unique_keys.push(UniqueConstraint {
                name: Some(name),
                columns: vec![column],
            }),
        }
    }
    type KeyColumn = (String, String, String, String, String, String, String);
    let key_columns: Vec<KeyColumn> = conn
        .exec(
            "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_TABLE_SCHEMA, \
             k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, r.DELETE_RULE, r.UPDATE_RULE \
             FROM information_schema.KEY_COLUMN_USAGE k \
             JOIN information_schema.REFERENTIAL_CONSTRAINTS r \
               ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA \
               AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND r.TABLE_NAME = k.TABLE_NAME \
             WHERE k.TABLE_SCHEMA = ? AND k.TABLE_NAME = ? \
             AND k.REFERENCED_TABLE_NAME IS NOT NULL \
             ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION",
            at,
        )
        .await
        .map_err(connection_error)?;
    let mut foreign_keys: Vec<ForeignKeyConstraint> = Vec::new();
    for (name, column, schema, referenced, referenced_column, on_delete, on_update) in key_columns {
        match foreign_keys.last_mut() {
            Some(key) if key.name.as_deref() == Some(name.as_str()) => {
                key.columns.push(column);
                key.referenced_columns.push(referenced_column);
            }
            _ => foreign_keys.push(ForeignKeyConstraint {
                name: Some(name),
                columns: vec![column],
                referenced: TableName {
                    schema,
                    name: referenced,
                },
                referenced_columns: vec![referenced_column],
                on_delete: ReferentialAction::from_catalog(&on_delete).unwrap_or_default(),
                on_update: ReferentialAction::from_catalog(&on_update).unwrap_or_default(),
            }),
        }
    }
    // MySQL names checks uniquely in a schema, MariaDB in a table, whose name its
    // `CHECK_CONSTRAINTS` has (and MySQL's doesn't).
    let same_table = match flavour {
        Flavour::Mysql => "",
        Flavour::MariaDb => "AND k.TABLE_NAME = c.TABLE_NAME",
    };
    let checks: Vec<(String, String)> = conn
        .exec(
            format!(
                "SELECT k.CONSTRAINT_NAME, k.CHECK_CLAUSE \
                 FROM information_schema.TABLE_CONSTRAINTS c \
                 JOIN information_schema.CHECK_CONSTRAINTS k \
                   ON k.CONSTRAINT_SCHEMA = c.CONSTRAINT_SCHEMA \
                   AND k.CONSTRAINT_NAME = c.CONSTRAINT_NAME {same_table} \
                 WHERE c.TABLE_SCHEMA = ? AND c.TABLE_NAME = ? AND c.CONSTRAINT_TYPE = 'CHECK' \
                 ORDER BY k.CONSTRAINT_NAME"
            ),
            at,
        )
        .await
        .map_err(connection_error)?;
    Ok(TableConstraints {
        primary_key_name: None,
        unique_keys,
        foreign_keys,
        checks: checks
            .into_iter()
            .map(|(name, expression)| CheckConstraint {
                name: Some(name),
                expression,
            })
            .collect(),
        default_names: Default::default(),
    })
}
