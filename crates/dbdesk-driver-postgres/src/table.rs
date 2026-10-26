//! Describing a table for editing its rows.

use dbdesk_core::{
    ColumnDescription, DriverError, ForeignKey, Quoting, RelationName, TableDescription, TableName,
};
use tokio_postgres::Client;

use crate::connection_error;

/// Kinds of `pg_class` rows whose rows can be updated: ordinary and partitioned tables.
const PLAIN_TABLE_KINDS: [&str; 2] = ["r", "p"];

/// Resolves `table` with `to_regclass`, which applies the search path and folds unquoted
/// names to lowercase exactly as a statement would, then reads its columns (with their types,
/// nullability and defaults), primary key and foreign keys both ways.
pub(crate) async fn describe_table(
    client: &Client,
    table: &RelationName,
) -> Result<Option<TableDescription>, DriverError> {
    let written = table.to_sql(Quoting::STANDARD);
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
            "SELECT a.attname, array_position(i.indkey::int2[], a.attnum), \
             format_type(a.atttypid, a.atttypmod), NOT a.attnotnull, \
             pg_get_expr(d.adbin, d.adrelid) \
             FROM pg_attribute a \
             LEFT JOIN pg_index i ON i.indrelid = a.attrelid AND i.indisprimary \
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
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
        foreign_keys: Vec::new(),
        referenced_by: Vec::new(),
    };
    let mut key = Vec::new();
    for column in &columns {
        let name: String = column.try_get(0).map_err(connection_error)?;
        let key_position: Option<i32> = column.try_get(1).map_err(connection_error)?;
        if let Some(position) = key_position {
            key.push((position, name.clone()));
        }
        description.columns.push(ColumnDescription {
            name,
            data_type: column.try_get(2).map_err(connection_error)?,
            nullable: column.try_get(3).map_err(connection_error)?,
            default: column.try_get(4).map_err(connection_error)?,
        });
    }
    key.sort();
    description.primary_key = key.into_iter().map(|(_, name)| name).collect();
    let me = description.table_name();
    for key in foreign_keys(client, &written).await? {
        if key.table == me {
            description.foreign_keys.push(key.clone());
        }
        if key.referenced == me {
            description.referenced_by.push(key);
        }
    }
    Ok(Some(description))
}

/// The foreign keys from or to the table, each column list in key order.
///
/// A key of or to a partitioned table is copied onto its partitions, each copy naming its
/// parent in `conparentid`. A copy on a partition of the table's own is a key that partition
/// has; a copy that only points at a partition of the referenced table repeats a key the
/// table already lists, and is left out.
async fn foreign_keys(client: &Client, written: &str) -> Result<Vec<ForeignKey>, DriverError> {
    let rows = client
        .query(
            "SELECT tn.nspname, t.relname, \
             ARRAY(SELECT a.attname::text FROM unnest(c.conkey) WITH ORDINALITY k(attnum, i) \
               JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum \
               ORDER BY k.i), \
             rn.nspname, r.relname, \
             ARRAY(SELECT a.attname::text FROM unnest(c.confkey) WITH ORDINALITY k(attnum, i) \
               JOIN pg_attribute a ON a.attrelid = c.confrelid AND a.attnum = k.attnum \
               ORDER BY k.i) \
             FROM pg_constraint c \
             JOIN pg_class t ON t.oid = c.conrelid \
             JOIN pg_namespace tn ON tn.oid = t.relnamespace \
             JOIN pg_class r ON r.oid = c.confrelid \
             JOIN pg_namespace rn ON rn.oid = r.relnamespace \
             WHERE c.contype = 'f' \
             AND (c.conrelid = to_regclass($1) OR c.confrelid = to_regclass($1)) \
             AND NOT EXISTS (SELECT 1 FROM pg_constraint p \
               WHERE p.oid = c.conparentid AND p.conrelid = c.conrelid) \
             ORDER BY c.conname",
            &[&written],
        )
        .await
        .map_err(connection_error)?;
    rows.iter()
        .map(|row| {
            let get = |index| row.try_get::<_, String>(index).map_err(connection_error);
            Ok(ForeignKey {
                table: TableName {
                    schema: get(0)?,
                    name: get(1)?,
                },
                columns: row.try_get(2).map_err(connection_error)?,
                referenced: TableName {
                    schema: get(3)?,
                    name: get(4)?,
                },
                referenced_columns: row.try_get(5).map_err(connection_error)?,
            })
        })
        .collect()
}
