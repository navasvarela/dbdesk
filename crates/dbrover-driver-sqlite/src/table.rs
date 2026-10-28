//! Describing a table for editing its rows.

use dbrover_core::{
    ColumnDescription, DriverError, ForeignKey, Identity, IdentityGeneration, Quoting,
    RelationName, TableDescription, TableName,
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::connection_error;

/// Finds the table the way SQLite resolves an unqualified name (`temp` first, then `main`,
/// then attached databases), ignoring case as SQLite does, then reads its columns, primary
/// key and foreign keys both ways. Views, virtual and shadow tables aren't plain tables.
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
        .prepare(
            "SELECT name, type, \"notnull\", dflt_value, pk \
             FROM pragma_table_info(?1, ?2) ORDER BY cid",
        )
        .map_err(connection_error)?;
    let columns: Vec<(ColumnDescription, i64)> = statement
        .query_map(params![name, schema], |row| {
            let not_null: bool = row.get(2)?;
            let column = ColumnDescription {
                name: row.get(0)?,
                data_type: row.get(1)?,
                nullable: !not_null,
                default: row.get(3)?,
                identity: None,
            };
            Ok((column, row.get(4)?))
        })
        .map_err(connection_error)?
        .collect::<Result<_, _>>()
        .map_err(connection_error)?;
    // `pk` is the column's 1-based position in the key, 0 outside it.
    let mut key: Vec<(i64, String)> = columns
        .iter()
        .filter(|(_, position)| *position > 0)
        .map(|(column, position)| (*position, column.name.clone()))
        .collect();
    key.sort();
    let primary_key: Vec<String> = key.into_iter().map(|(_, name)| name).collect();
    let mut columns: Vec<ColumnDescription> =
        columns.into_iter().map(|(column, _)| column).collect();
    if let [key] = primary_key.as_slice()
        && autoincrements(connection, &schema, &name)?
        && let Some(column) = columns.iter_mut().find(|column| &column.name == key)
    {
        column.identity = Some(Identity::counting(IdentityGeneration::ByDefault));
    }
    let me = TableName {
        schema: schema.clone(),
        name: name.clone(),
    };
    let foreign_keys = outgoing_keys(connection, &me)?;
    let referenced_by = incoming_keys(connection, &me, &primary_key)?;
    Ok(Some(TableDescription {
        schema,
        name,
        columns,
        primary_key,
        foreign_keys,
        referenced_by,
    }))
}

/// Whether the table's key says `AUTOINCREMENT`, which only an `INTEGER PRIMARY KEY` may: its
/// numbers then never repeat one a deleted row had. SQLite keeps no flag for it, only the
/// statement that created the table.
fn autoincrements(connection: &Connection, schema: &str, table: &str) -> Result<bool, DriverError> {
    let sql = format!(
        "SELECT sql FROM {}.sqlite_schema WHERE type = 'table' AND name = ?1",
        Quoting::STANDARD.identifier(schema)
    );
    let definition: Option<String> = connection
        .query_row(&sql, [table], |row| row.get(0))
        .optional()
        .map_err(connection_error)?
        .flatten();
    Ok(definition.is_some_and(|sql| sql.to_ascii_uppercase().contains("AUTOINCREMENT")))
}

/// One row of `pragma_foreign_key_list`, with the table it belongs to: a column of key `id`.
/// `to` is `None` where the key names no columns and so refers to the parent's primary key.
struct KeyColumn {
    table: String,
    id: i64,
    parent: String,
    from: String,
    to: Option<String>,
}

/// The table's own foreign keys. SQLite keeps a key's parent table as the `REFERENCES` clause
/// wrote it, in the same schema; it is looked up to report it as stored.
fn outgoing_keys(connection: &Connection, me: &TableName) -> Result<Vec<ForeignKey>, DriverError> {
    let mut statement = connection
        .prepare(
            "SELECT ?1, id, \"table\", \"from\", \"to\" FROM pragma_foreign_key_list(?1, ?2) \
             ORDER BY id, seq",
        )
        .map_err(connection_error)?;
    let columns = key_columns(&mut statement, params![me.name, me.schema])?;
    let mut keys = Vec::new();
    for group in group_by_key(columns) {
        let parent = TableName {
            schema: me.schema.clone(),
            name: stored_name(connection, &me.schema, &group[0].parent)?,
        };
        let parent_key = primary_key(connection, &parent)?;
        keys.push(foreign_key(me.clone(), parent, &group, &parent_key));
    }
    Ok(keys)
}

/// Other tables' foreign keys to this one, in its schema.
fn incoming_keys(
    connection: &Connection,
    me: &TableName,
    primary_key: &[String],
) -> Result<Vec<ForeignKey>, DriverError> {
    let mut statement = connection
        .prepare(
            "SELECT t.name, k.id, k.\"table\", k.\"from\", k.\"to\" \
             FROM pragma_table_list t JOIN pragma_foreign_key_list(t.name, t.schema) k \
             WHERE t.schema = ?1 AND t.type = 'table' AND k.\"table\" = ?2 COLLATE NOCASE \
             ORDER BY t.name, k.id, k.seq",
        )
        .map_err(connection_error)?;
    let columns = key_columns(&mut statement, params![me.schema, me.name])?;
    Ok(group_by_key(columns)
        .into_iter()
        .map(|group| {
            let child = TableName {
                schema: me.schema.clone(),
                name: group[0].table.clone(),
            };
            foreign_key(child, me.clone(), &group, primary_key)
        })
        .collect())
}

fn key_columns(
    statement: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> Result<Vec<KeyColumn>, DriverError> {
    statement
        .query_map(params, |row| {
            Ok(KeyColumn {
                table: row.get(0)?,
                id: row.get(1)?,
                parent: row.get(2)?,
                from: row.get(3)?,
                to: row.get(4)?,
            })
        })
        .map_err(connection_error)?
        .collect::<Result<_, _>>()
        .map_err(connection_error)
}

/// Splits key columns, listed key by key, into one group per key.
fn group_by_key(columns: Vec<KeyColumn>) -> Vec<Vec<KeyColumn>> {
    let mut groups: Vec<Vec<KeyColumn>> = Vec::new();
    for column in columns {
        match groups.last_mut() {
            Some(group) if group[0].table == column.table && group[0].id == column.id => {
                group.push(column);
            }
            _ => groups.push(vec![column]),
        }
    }
    groups
}

/// A key from its columns; a key that names no parent columns refers to the parent's
/// primary key.
fn foreign_key(
    table: TableName,
    referenced: TableName,
    columns: &[KeyColumn],
    parent_key: &[String],
) -> ForeignKey {
    let named: Option<Vec<String>> = columns.iter().map(|column| column.to.clone()).collect();
    ForeignKey {
        table,
        columns: columns.iter().map(|column| column.from.clone()).collect(),
        referenced,
        referenced_columns: named.unwrap_or_else(|| parent_key.to_vec()),
    }
}

/// The table's name as stored, for a name written in any case; as written if there is no
/// such table, which SQLite allows a key to name.
fn stored_name(
    connection: &Connection,
    schema: &str,
    written: &str,
) -> Result<String, DriverError> {
    let stored: Option<String> = connection
        .query_row(
            "SELECT name FROM pragma_table_list WHERE schema = ?1 AND name = ?2 COLLATE NOCASE",
            params![schema, written],
            |row| row.get(0),
        )
        .optional()
        .map_err(connection_error)?;
    Ok(stored.unwrap_or_else(|| written.to_owned()))
}

fn primary_key(connection: &Connection, table: &TableName) -> Result<Vec<String>, DriverError> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1, ?2) WHERE pk > 0 ORDER BY pk")
        .map_err(connection_error)?;
    statement
        .query_map(params![table.name, table.schema], |row| row.get(0))
        .map_err(connection_error)?
        .collect::<Result<_, _>>()
        .map_err(connection_error)
}
