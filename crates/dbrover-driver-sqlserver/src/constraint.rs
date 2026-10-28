//! A table constraint's definition, as the `ALTER TABLE … ADD CONSTRAINT` that recreates it.
//!
//! SQL Server keeps no such statement: a key's columns are its index's, a foreign key's are in
//! `foreign_key_columns`, and only checks and defaults store their expression. Each kind is
//! read from its catalog view and written out here, for Show definition and for Copy DDL,
//! which adds them after a `CREATE TABLE` rebuilt from the table's description.

use dbrover_core::{
    CheckConstraint, DriverError, ForeignKeyConstraint, ReferentialAction, TableConstraints,
    TableName, UniqueConstraint,
};

use crate::{SQL_SERVER_QUOTING, SqlClient, column, connection_error};

/// The definition of the constraint `name` (three-part, quoted), whose database's catalog
/// views are `catalog` (`[sales].sys`).
pub(crate) async fn definition(
    client: &mut SqlClient,
    catalog: &str,
    name: &str,
) -> Result<String, DriverError> {
    let owner = rows(
        client,
        &format!(
            "SELECT RTRIM(o.type), o.name, ts.name, t.name FROM {catalog}.objects o \
             JOIN {catalog}.objects t ON t.object_id = o.parent_object_id \
             JOIN {catalog}.schemas ts ON ts.schema_id = t.schema_id \
             WHERE o.object_id = OBJECT_ID(@P1)"
        ),
        name,
        4,
    )
    .await?;
    let [kind, constraint, schema, table] = owner
        .into_iter()
        .next()
        .and_then(|row| <[String; 4]>::try_from(row).ok())
        .ok_or_else(|| connection_error(format!("no constraint {name}")))?;
    let body = match kind.as_str() {
        "PK" | "UQ" => {
            let key = if kind == "PK" {
                "PRIMARY KEY"
            } else {
                "UNIQUE"
            };
            let columns = first_column(rows(client, &key_columns(catalog), name, 1).await?);
            format!("{key} {}", column_list(&columns))
        }
        "F" => foreign_key(client, catalog, name).await?,
        "C" => {
            let sql = format!(
                "SELECT definition FROM {catalog}.check_constraints \
                 WHERE object_id = OBJECT_ID(@P1)"
            );
            let expression = first_column(rows(client, &sql, name, 1).await?);
            format!("CHECK {}", expression.concat())
        }
        "D" => {
            let sql = format!(
                "SELECT d.definition, c.name FROM {catalog}.default_constraints d \
                 JOIN {catalog}.columns c \
                 ON c.object_id = d.parent_object_id AND c.column_id = d.parent_column_id \
                 WHERE d.object_id = OBJECT_ID(@P1)"
            );
            let [expression, target] = rows(client, &sql, name, 2)
                .await?
                .into_iter()
                .next()
                .and_then(|row| <[String; 2]>::try_from(row).ok())
                .ok_or_else(|| connection_error(format!("no default {name}")))?;
            format!("DEFAULT {expression} FOR {}", quote(&target))
        }
        other => return Err(connection_error(format!("a constraint of type {other}"))),
    };
    Ok(format!(
        "ALTER TABLE {}.{} ADD CONSTRAINT {} {body};",
        quote(&schema),
        quote(&table),
        quote(&constraint)
    ))
}

/// A primary or unique key's columns, in key order.
fn key_columns(catalog: &str) -> String {
    format!(
        "SELECT c.name FROM {catalog}.key_constraints k \
         JOIN {catalog}.index_columns ic \
         ON ic.object_id = k.parent_object_id AND ic.index_id = k.unique_index_id \
         JOIN {catalog}.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
         WHERE k.object_id = OBJECT_ID(@P1) ORDER BY ic.key_ordinal"
    )
}

/// `FOREIGN KEY (…) REFERENCES … (…)`, with the actions that aren't the default.
async fn foreign_key(
    client: &mut SqlClient,
    catalog: &str,
    name: &str,
) -> Result<String, DriverError> {
    let target = format!(
        "SELECT rs.name, rt.name, fk.delete_referential_action_desc, \
         fk.update_referential_action_desc FROM {catalog}.foreign_keys fk \
         JOIN {catalog}.objects rt ON rt.object_id = fk.referenced_object_id \
         JOIN {catalog}.schemas rs ON rs.schema_id = rt.schema_id \
         WHERE fk.object_id = OBJECT_ID(@P1)"
    );
    let [schema, table, on_delete, on_update] = rows(client, &target, name, 4)
        .await?
        .into_iter()
        .next()
        .and_then(|row| <[String; 4]>::try_from(row).ok())
        .ok_or_else(|| connection_error(format!("no foreign key {name}")))?;
    let pairs = format!(
        "SELECT pc.name, rc.name FROM {catalog}.foreign_key_columns fkc \
         JOIN {catalog}.columns pc \
         ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id \
         JOIN {catalog}.columns rc \
         ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id \
         WHERE fkc.constraint_object_id = OBJECT_ID(@P1) ORDER BY fkc.constraint_column_id"
    );
    let (columns, referenced): (Vec<String>, Vec<String>) = rows(client, &pairs, name, 2)
        .await?
        .into_iter()
        .filter_map(|row| <[String; 2]>::try_from(row).ok())
        .map(|[column, referenced]| (column, referenced))
        .unzip();
    let mut sql = format!(
        "FOREIGN KEY {} REFERENCES {}.{} {}",
        column_list(&columns),
        quote(&schema),
        quote(&table),
        column_list(&referenced)
    );
    // The catalog spells actions `NO_ACTION`, `SET_NULL`, …; `NO ACTION` is the default.
    for (event, action) in [("DELETE", on_delete), ("UPDATE", on_update)] {
        if action != "NO_ACTION" {
            sql.push_str(&format!(" ON {event} {}", action.replace('_', " ")));
        }
    }
    Ok(sql)
}

/// Each row's first `width` columns, as text.
async fn rows(
    client: &mut SqlClient,
    sql: &str,
    name: &str,
    width: usize,
) -> Result<Vec<Vec<String>>, DriverError> {
    let rows = client
        .query(sql, &[&name])
        .await
        .map_err(connection_error)?
        .into_first_result()
        .await
        .map_err(connection_error)?;
    Ok(rows
        .iter()
        .map(|row| {
            (0..width)
                .map(|index| column::<&str>(row, index).unwrap_or_default().to_owned())
                .collect()
        })
        .collect())
}

fn first_column(rows: Vec<Vec<String>>) -> Vec<String> {
    rows.into_iter()
        .filter_map(|row| row.into_iter().next())
        .collect()
}

fn quote(name: &str) -> String {
    SQL_SERVER_QUOTING.identifier_if_needed(name)
}

fn column_list(columns: &[String]) -> String {
    let names: Vec<String> = columns.iter().map(|column| quote(column)).collect();
    format!("({})", names.join(", "))
}

/// The table's keys, foreign keys, checks and defaults, by name: SQL Server drops each by its
/// name, defaults included.
pub(crate) async fn describe_constraints(
    client: &mut SqlClient,
    table: &TableName,
) -> Result<TableConstraints, DriverError> {
    let object = format!("{}.{}", quote(&table.schema), quote(&table.name));
    let of_table = "parent_object_id = OBJECT_ID(@P1, N'U')";
    let mut constraints = TableConstraints::default();
    let keys = format!(
        "SELECT k.name, RTRIM(k.type), c.name FROM sys.key_constraints k \
         JOIN sys.index_columns ic \
         ON ic.object_id = k.parent_object_id AND ic.index_id = k.unique_index_id \
         JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
         WHERE k.{of_table} ORDER BY k.name, ic.key_ordinal"
    );
    for [name, kind, column] in fixed_rows(rows(client, &keys, &object, 3).await?) {
        if kind == "PK" {
            constraints.primary_key_name = Some(name);
            continue;
        }
        match constraints.unique_keys.last_mut() {
            Some(key) if key.name.as_deref() == Some(name.as_str()) => key.columns.push(column),
            _ => constraints.unique_keys.push(UniqueConstraint {
                name: Some(name),
                columns: vec![column],
            }),
        }
    }
    let foreign = format!(
        "SELECT fk.name, pc.name, rs.name, rt.name, rc.name, \
         fk.delete_referential_action_desc, fk.update_referential_action_desc \
         FROM sys.foreign_keys fk \
         JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id \
         JOIN sys.columns pc \
         ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id \
         JOIN sys.columns rc \
         ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id \
         JOIN sys.objects rt ON rt.object_id = fk.referenced_object_id \
         JOIN sys.schemas rs ON rs.schema_id = rt.schema_id \
         WHERE fk.{of_table} ORDER BY fk.name, fkc.constraint_column_id"
    );
    for [
        name,
        column,
        schema,
        referenced,
        referenced_column,
        on_delete,
        on_update,
    ] in fixed_rows(rows(client, &foreign, &object, 7).await?)
    {
        match constraints.foreign_keys.last_mut() {
            Some(key) if key.name.as_deref() == Some(name.as_str()) => {
                key.columns.push(column);
                key.referenced_columns.push(referenced_column);
            }
            _ => constraints.foreign_keys.push(ForeignKeyConstraint {
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
    let checks = format!(
        "SELECT name, definition FROM sys.check_constraints WHERE {of_table} ORDER BY name"
    );
    constraints.checks = fixed_rows(rows(client, &checks, &object, 2).await?)
        .map(|[name, expression]| CheckConstraint {
            name: Some(name),
            expression,
        })
        .collect();
    let defaults = format!(
        "SELECT c.name, d.name FROM sys.default_constraints d \
         JOIN sys.columns c ON c.object_id = d.parent_object_id AND c.column_id = d.parent_column_id \
         WHERE d.{of_table}"
    );
    constraints.default_names = fixed_rows(rows(client, &defaults, &object, 2).await?)
        .map(|[column, name]| (column, name))
        .collect();
    Ok(constraints)
}

fn fixed_rows<const N: usize>(rows: Vec<Vec<String>>) -> impl Iterator<Item = [String; N]> {
    rows.into_iter()
        .filter_map(|row| <[String; N]>::try_from(row).ok())
}
