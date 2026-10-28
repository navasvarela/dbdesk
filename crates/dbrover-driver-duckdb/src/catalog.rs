//! Catalog introspection: one query on DuckDB's `duckdb_*()` table functions per tree level,
//! and definitions from their `sql` columns.
//!
//! The tree is databases (the one opened, and any attached), their schemas, and each schema's
//! tables, views, macros and sequences. Macros are DuckDB's user-defined functions, so they
//! are listed under functions.

use crate::connection_error;
use dbrover_core::{CatalogNode, CatalogPath, DriverError, FolderKind, NodeKind, PathSegment};

const SCHEMA_FOLDERS: &[FolderKind] = &[
    FolderKind::Tables,
    FolderKind::Views,
    FolderKind::Functions,
    FolderKind::Sequences,
];

/// Which level of the tree a path points at, with the names its query needs.
enum Level<'a> {
    Databases,
    Schemas {
        database: &'a str,
    },
    SchemaFolders,
    SchemaObjects {
        database: &'a str,
        schema: &'a str,
        folder: FolderKind,
    },
    RelationFolders {
        relation_kind: NodeKind,
    },
    RelationChildren {
        database: &'a str,
        schema: &'a str,
        relation: &'a str,
        folder: FolderKind,
    },
}

impl<'a> Level<'a> {
    fn of(path: &'a CatalogPath) -> Option<Self> {
        match path.segments() {
            [] => Some(Self::Databases),
            [database] if database.kind == NodeKind::Database => Some(Self::Schemas {
                database: &database.name,
            }),
            [database, schema, rest @ ..]
                if database.kind == NodeKind::Database && schema.kind == NodeKind::Schema =>
            {
                Self::within_schema(&database.name, &schema.name, rest)
            }
            _ => None,
        }
    }

    fn within_schema(database: &'a str, schema: &'a str, rest: &'a [PathSegment]) -> Option<Self> {
        match rest {
            [] => Some(Self::SchemaFolders),
            [folder] => folder_kind(folder)
                .filter(|folder| SCHEMA_FOLDERS.contains(folder))
                .map(|folder| Self::SchemaObjects {
                    database,
                    schema,
                    folder,
                }),
            [folder, relation] if is_relation_in(folder, relation) => Some(Self::RelationFolders {
                relation_kind: relation.kind,
            }),
            [folder, relation, child_folder] if is_relation_in(folder, relation) => {
                folder_kind(child_folder)
                    .filter(|child| relation_folders(relation.kind).contains(child))
                    .map(|folder| Self::RelationChildren {
                        database,
                        schema,
                        relation: &relation.name,
                        folder,
                    })
            }
            _ => None,
        }
    }
}

fn folder_kind(segment: &PathSegment) -> Option<FolderKind> {
    match segment.kind {
        NodeKind::Folder(folder) => Some(folder),
        _ => None,
    }
}

fn is_relation_in(folder: &PathSegment, relation: &PathSegment) -> bool {
    let expected = match relation.kind {
        NodeKind::Table => FolderKind::Tables,
        NodeKind::View => FolderKind::Views,
        _ => return false,
    };
    folder_kind(folder) == Some(expected)
}

fn relation_folders(kind: NodeKind) -> &'static [FolderKind] {
    match kind {
        NodeKind::Table => &[
            FolderKind::Columns,
            FolderKind::Indexes,
            FolderKind::Constraints,
        ],
        NodeKind::View => &[FolderKind::Columns],
        _ => &[],
    }
}

pub(crate) fn children(
    connection: &duckdb::Connection,
    path: &CatalogPath,
) -> Result<Vec<CatalogNode>, DriverError> {
    let level = Level::of(path).ok_or_else(|| DriverError::InvalidPath(path.clone()))?;
    match level {
        Level::Databases => {
            let sql = "SELECT database_name, internal, false, NULL FROM duckdb_databases() \
                       ORDER BY database_name";
            Ok(nodes(
                path,
                NodeKind::Database,
                true,
                list(connection, sql, &[])?,
            ))
        }
        // DuckDB counts `main` among its internal schemas too, but it is where tables go.
        Level::Schemas { database } => {
            let sql = "SELECT schema_name, \
                       schema_name IN ('information_schema', 'pg_catalog'), false, NULL \
                       FROM duckdb_schemas() WHERE database_name = ? ORDER BY schema_name";
            Ok(nodes(
                path,
                NodeKind::Schema,
                true,
                list(connection, sql, &[database])?,
            ))
        }
        Level::SchemaFolders => Ok(SCHEMA_FOLDERS
            .iter()
            .map(|folder| CatalogNode::folder(path, *folder))
            .collect()),
        Level::SchemaObjects {
            database,
            schema,
            folder,
        } => schema_objects(connection, path, database, schema, folder),
        Level::RelationFolders { relation_kind } => Ok(relation_folders(relation_kind)
            .iter()
            .map(|folder| CatalogNode::folder(path, *folder))
            .collect()),
        Level::RelationChildren {
            database,
            schema,
            relation,
            folder,
        } => relation_children(connection, path, [database, schema, relation], folder),
    }
}

/// One catalog row, as every listing query returns it: the name, whether DuckDB made it
/// itself, whether it has a definition, and a detail or `NULL`.
struct Listed {
    name: String,
    system: bool,
    has_definition: bool,
    detail: Option<String>,
}

fn list(
    connection: &duckdb::Connection,
    sql: &str,
    names: &[&str],
) -> Result<Vec<Listed>, DriverError> {
    let mut statement = connection.prepare(sql).map_err(connection_error)?;
    let rows = statement
        .query_map(duckdb::params_from_iter(names), |row| {
            Ok(Listed {
                name: row.get(0)?,
                system: row.get(1)?,
                has_definition: row.get(2)?,
                detail: row.get(3)?,
            })
        })
        .map_err(connection_error)?;
    rows.collect::<Result<_, _>>().map_err(connection_error)
}

fn nodes(
    parent: &CatalogPath,
    kind: NodeKind,
    has_children: bool,
    listed: Vec<Listed>,
) -> Vec<CatalogNode> {
    listed
        .into_iter()
        .map(|listed| CatalogNode {
            path: parent.child(kind, listed.name.clone()),
            label: listed.name,
            detail: listed.detail,
            has_children,
            has_definition: listed.has_definition,
            system: listed.system,
        })
        .collect()
}

fn schema_objects(
    connection: &duckdb::Connection,
    path: &CatalogPath,
    database: &str,
    schema: &str,
    folder: FolderKind,
) -> Result<Vec<CatalogNode>, DriverError> {
    let (kind, sql) = match folder {
        FolderKind::Tables => (
            NodeKind::Table,
            "SELECT table_name, internal, sql IS NOT NULL, NULL FROM duckdb_tables() \
             WHERE database_name = ? AND schema_name = ? ORDER BY table_name",
        ),
        FolderKind::Views => (
            NodeKind::View,
            "SELECT view_name, internal, sql IS NOT NULL, NULL FROM duckdb_views() \
             WHERE database_name = ? AND schema_name = ? ORDER BY view_name",
        ),
        // A macro may have several parameter lists: one node, with every one in its
        // definition.
        FolderKind::Functions => (
            NodeKind::Function,
            "SELECT function_name, bool_or(internal), true, \
             CASE WHEN bool_or(function_type = 'table_macro') THEN 'table macro' \
             ELSE 'macro' END \
             FROM duckdb_functions() \
             WHERE database_name = ? AND schema_name = ? \
             AND function_type IN ('macro', 'table_macro') \
             GROUP BY function_name ORDER BY function_name",
        ),
        FolderKind::Sequences => (
            NodeKind::Sequence,
            "SELECT sequence_name, false, sql IS NOT NULL, NULL FROM duckdb_sequences() \
             WHERE database_name = ? AND schema_name = ? ORDER BY sequence_name",
        ),
        _ => return Err(DriverError::InvalidPath(path.clone())),
    };
    let is_relation = matches!(kind, NodeKind::Table | NodeKind::View);
    let listed = list(connection, sql, &[database, schema])?;
    Ok(nodes(path, kind, is_relation, listed))
}

fn relation_children(
    connection: &duckdb::Connection,
    path: &CatalogPath,
    names: [&str; 3],
    folder: FolderKind,
) -> Result<Vec<CatalogNode>, DriverError> {
    let (kind, sql) = match folder {
        FolderKind::Columns => (
            NodeKind::Column,
            "SELECT column_name, false, false, \
             data_type || CASE WHEN is_nullable THEN '' ELSE ', not null' END \
             FROM duckdb_columns() \
             WHERE database_name = ? AND schema_name = ? AND table_name = ? \
             ORDER BY column_index",
        ),
        FolderKind::Indexes => (
            NodeKind::Index,
            "SELECT index_name, false, sql IS NOT NULL, \
             CASE WHEN is_unique THEN 'unique' END FROM duckdb_indexes() \
             WHERE database_name = ? AND schema_name = ? AND table_name = ? \
             ORDER BY index_name",
        ),
        // NOT NULL is shown on the column; the others are constraints of the table.
        FolderKind::Constraints => (
            NodeKind::Constraint,
            "SELECT constraint_name, false, false, lower(constraint_type) \
             FROM duckdb_constraints() \
             WHERE database_name = ? AND schema_name = ? AND table_name = ? \
             AND constraint_type <> 'NOT NULL' ORDER BY constraint_name",
        ),
        _ => return Err(DriverError::InvalidPath(path.clone())),
    };
    Ok(nodes(path, kind, false, list(connection, sql, &names)?))
}

pub(crate) fn definition(
    connection: &duckdb::Connection,
    path: &CatalogPath,
) -> Result<String, DriverError> {
    // Only paths `children` returns: the level above must be a valid one.
    let valid_parent = path
        .parent()
        .is_some_and(|parent| Level::of(&parent).is_some());
    let invalid = || DriverError::InvalidPath(path.clone());
    let (Some(object), Some(database), Some(schema), true) = (
        path.last(),
        path.name_of(NodeKind::Database),
        path.name_of(NodeKind::Schema),
        valid_parent,
    ) else {
        return Err(invalid());
    };
    let names = [database, schema, object.name.as_str()];
    let sql = match object.kind {
        NodeKind::Table => {
            "SELECT sql FROM duckdb_tables() \
             WHERE database_name = ? AND schema_name = ? AND table_name = ?"
        }
        NodeKind::View => {
            "SELECT sql FROM duckdb_views() \
             WHERE database_name = ? AND schema_name = ? AND view_name = ?"
        }
        NodeKind::Sequence => {
            "SELECT sql FROM duckdb_sequences() \
             WHERE database_name = ? AND schema_name = ? AND sequence_name = ?"
        }
        NodeKind::Index => {
            let table = path.name_of(NodeKind::Table).ok_or_else(invalid)?;
            return single(
                connection,
                "SELECT sql FROM duckdb_indexes() WHERE database_name = ? \
                 AND schema_name = ? AND table_name = ? AND index_name = ?",
                &[database, schema, table, &object.name],
            );
        }
        // Rebuilt from the parts DuckDB keeps: it stores a macro's body but not the statement
        // that created it.
        NodeKind::Function => {
            "SELECT string_agg('CREATE MACRO ' || schema_name || '.' || function_name || '(' \
             || array_to_string(parameters, ', ') || ') AS ' \
             || CASE WHEN function_type = 'table_macro' THEN 'TABLE ' ELSE '' END \
             || macro_definition || ';', chr(10)) \
             FROM duckdb_functions() WHERE database_name = ? AND schema_name = ? \
             AND function_name = ? AND function_type IN ('macro', 'table_macro')"
        }
        _ => return Err(invalid()),
    };
    single(connection, sql, &names)
}

/// The one text value `sql` selects.
fn single(
    connection: &duckdb::Connection,
    sql: &str,
    names: &[&str],
) -> Result<String, DriverError> {
    let mut statement = connection.prepare(sql).map_err(connection_error)?;
    let found: Option<String> = statement
        .query_row(duckdb::params_from_iter(names), |row| row.get(0))
        .map_err(connection_error)?;
    found.ok_or_else(|| connection_error("DuckDB keeps no definition of it"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_it_never_returns() {
        let connection = duckdb::Connection::open_in_memory().unwrap();
        let nonsense = CatalogPath::root().child(NodeKind::Column, "c");
        assert!(matches!(
            children(&connection, &nonsense),
            Err(DriverError::InvalidPath(_))
        ));
    }
}
