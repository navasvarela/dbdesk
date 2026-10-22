//! Catalog introspection: one `pg_catalog` query per tree level, plus object definitions.
//!
//! `pg_catalog` rather than `information_schema`: it is faster, knows about materialized
//! views, and doesn't silently hide objects the user lacks privileges on, which would be
//! confusing in a browsing tool. See `docs/features/connection-explorer.md`.

use dbdesk_core::{
    CatalogNode, CatalogPath, DriverError, FolderKind, NodeKind, PathSegment, quote_identifier,
};
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, Row};

use crate::connection_error;

const SCHEMA_FOLDERS: &[FolderKind] = &[
    FolderKind::Tables,
    FolderKind::Views,
    FolderKind::MaterializedViews,
    FolderKind::Functions,
    FolderKind::Sequences,
];

/// The oid of relation `$2` in schema `$1`. Spliced into queries that take those parameters.
const RELATION_OID: &str = "(SELECT c.oid FROM pg_class c \
     JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = $1 AND c.relname = $2)";

/// Which level of the tree a path points at, with the names its query needs.
enum Level<'a> {
    Databases,
    Schemas,
    SchemaFolders,
    SchemaObjects {
        schema: &'a str,
        folder: FolderKind,
    },
    RelationFolders {
        relation_kind: NodeKind,
    },
    RelationChildren {
        schema: &'a str,
        relation: &'a str,
        folder: FolderKind,
    },
}

impl<'a> Level<'a> {
    fn of(path: &'a CatalogPath) -> Option<Self> {
        match path.segments() {
            [] => Some(Self::Databases),
            [database] if database.kind == NodeKind::Database => Some(Self::Schemas),
            [database, schema, rest @ ..]
                if database.kind == NodeKind::Database && schema.kind == NodeKind::Schema =>
            {
                Self::within_schema(&schema.name, rest)
            }
            _ => None,
        }
    }

    fn within_schema(schema: &'a str, rest: &'a [PathSegment]) -> Option<Self> {
        match rest {
            [] => Some(Self::SchemaFolders),
            [folder] => folder_kind(folder)
                .filter(|folder| SCHEMA_FOLDERS.contains(folder))
                .map(|folder| Self::SchemaObjects { schema, folder }),
            [folder, relation] if is_relation_in(folder, relation) => Some(Self::RelationFolders {
                relation_kind: relation.kind,
            }),
            [folder, relation, child_folder] if is_relation_in(folder, relation) => {
                folder_kind(child_folder)
                    .filter(|child| relation_folders(relation.kind).contains(child))
                    .map(|folder| Self::RelationChildren {
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

/// The schema folder a relation kind is listed in, or `None` if `kind` isn't a relation.
fn relation_folder(kind: NodeKind) -> Option<FolderKind> {
    match kind {
        NodeKind::Table => Some(FolderKind::Tables),
        NodeKind::View => Some(FolderKind::Views),
        NodeKind::MaterializedView => Some(FolderKind::MaterializedViews),
        _ => None,
    }
}

fn is_relation_in(folder: &PathSegment, relation: &PathSegment) -> bool {
    relation_folder(relation.kind).is_some()
        && folder_kind(folder) == relation_folder(relation.kind)
}

fn relation_folders(kind: NodeKind) -> &'static [FolderKind] {
    match kind {
        NodeKind::Table => &[
            FolderKind::Columns,
            FolderKind::Indexes,
            FolderKind::Constraints,
            FolderKind::Triggers,
        ],
        NodeKind::View => &[FolderKind::Columns],
        NodeKind::MaterializedView => &[FolderKind::Columns, FolderKind::Indexes],
        _ => &[],
    }
}

fn is_system_schema(name: &str) -> bool {
    matches!(name, "pg_catalog" | "information_schema")
        || name.starts_with("pg_toast")
        || name.starts_with("pg_temp_")
}

pub(crate) async fn children(
    client: &Client,
    path: &CatalogPath,
) -> Result<Vec<CatalogNode>, DriverError> {
    let level = Level::of(path).ok_or_else(|| DriverError::InvalidPath(path.clone()))?;
    match level {
        Level::Databases => databases(client, path).await,
        Level::Schemas => schemas(client, path).await,
        Level::SchemaFolders => Ok(folders(path, SCHEMA_FOLDERS)),
        Level::SchemaObjects { schema, folder } => {
            schema_objects(client, path, schema, folder).await
        }
        Level::RelationFolders { relation_kind } => {
            Ok(folders(path, relation_folders(relation_kind)))
        }
        Level::RelationChildren {
            schema,
            relation,
            folder,
        } => relation_children(client, path, schema, relation, folder).await,
    }
}

fn folders(parent: &CatalogPath, folders: &[FolderKind]) -> Vec<CatalogNode> {
    folders
        .iter()
        .map(|folder| CatalogNode::folder(parent, *folder))
        .collect()
}

/// A node with no detail, not a system object.
fn node(
    parent: &CatalogPath,
    kind: NodeKind,
    name: String,
    has_children: bool,
    has_definition: bool,
) -> CatalogNode {
    CatalogNode {
        path: parent.child(kind, name.clone()),
        label: name,
        detail: None,
        has_children,
        has_definition,
        system: false,
    }
}

async fn query(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, DriverError> {
    client.query(sql, params).await.map_err(connection_error)
}

fn text(row: &Row, column: usize) -> Result<String, DriverError> {
    row.try_get(column).map_err(connection_error)
}

async fn databases(client: &Client, path: &CatalogPath) -> Result<Vec<CatalogNode>, DriverError> {
    let sql = "SELECT datname FROM pg_database \
               WHERE datallowconn AND NOT datistemplate ORDER BY datname";
    query(client, sql, &[])
        .await?
        .iter()
        .map(|row| Ok(node(path, NodeKind::Database, text(row, 0)?, true, false)))
        .collect()
}

async fn schemas(client: &Client, path: &CatalogPath) -> Result<Vec<CatalogNode>, DriverError> {
    let sql = "SELECT nspname FROM pg_namespace ORDER BY nspname";
    query(client, sql, &[])
        .await?
        .iter()
        .map(|row| {
            let name = text(row, 0)?;
            let system = is_system_schema(&name);
            Ok(CatalogNode {
                system,
                ..node(path, NodeKind::Schema, name, true, false)
            })
        })
        .collect()
}

async fn schema_objects(
    client: &Client,
    path: &CatalogPath,
    schema: &str,
    folder: FolderKind,
) -> Result<Vec<CatalogNode>, DriverError> {
    let (kind, relkinds): (NodeKind, &[&str]) = match folder {
        FolderKind::Tables => (NodeKind::Table, &["r", "p"]),
        FolderKind::Views => (NodeKind::View, &["v"]),
        FolderKind::MaterializedViews => (NodeKind::MaterializedView, &["m"]),
        FolderKind::Sequences => (NodeKind::Sequence, &["S"]),
        FolderKind::Functions => return functions(client, path, schema).await,
        _ => return Err(DriverError::InvalidPath(path.clone())),
    };
    let sql = "SELECT c.relname FROM pg_class c \
               JOIN pg_namespace n ON n.oid = c.relnamespace \
               WHERE n.nspname = $1 AND c.relkind::text = ANY($2) ORDER BY c.relname";
    let is_relation = relation_folder(kind).is_some();
    let has_definition = matches!(kind, NodeKind::View | NodeKind::MaterializedView);
    query(client, sql, &[&schema, &relkinds])
        .await?
        .iter()
        .map(|row| Ok(node(path, kind, text(row, 0)?, is_relation, has_definition)))
        .collect()
}

/// Functions are named with their argument list, `add(integer, integer)`, because overloads
/// share a name and the path must identify exactly one.
async fn functions(
    client: &Client,
    path: &CatalogPath,
    schema: &str,
) -> Result<Vec<CatalogNode>, DriverError> {
    // Aggregates (prokind 'a') are left out: pg_get_functiondef can't show them.
    let sql = "SELECT p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')' \
               FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
               WHERE n.nspname = $1 AND p.prokind IN ('f', 'p', 'w') ORDER BY 1";
    query(client, sql, &[&schema])
        .await?
        .iter()
        .map(|row| Ok(node(path, NodeKind::Function, text(row, 0)?, false, true)))
        .collect()
}

async fn relation_children(
    client: &Client,
    path: &CatalogPath,
    schema: &str,
    relation: &str,
    folder: FolderKind,
) -> Result<Vec<CatalogNode>, DriverError> {
    match folder {
        FolderKind::Columns => columns(client, path, schema, relation).await,
        FolderKind::Indexes => indexes(client, path, schema, relation).await,
        FolderKind::Constraints => constraints(client, path, schema, relation).await,
        FolderKind::Triggers => triggers(client, path, schema, relation).await,
        _ => Err(DriverError::InvalidPath(path.clone())),
    }
}

async fn columns(
    client: &Client,
    path: &CatalogPath,
    schema: &str,
    relation: &str,
) -> Result<Vec<CatalogNode>, DriverError> {
    let sql = format!(
        "SELECT a.attname, format_type(a.atttypid, a.atttypmod), a.attnotnull \
         FROM pg_attribute a \
         WHERE a.attrelid = {RELATION_OID} AND a.attnum > 0 AND NOT a.attisdropped \
         ORDER BY a.attnum"
    );
    query(client, &sql, &[&schema, &relation])
        .await?
        .iter()
        .map(|row| {
            let column_type = text(row, 1)?;
            let not_null: bool = row.try_get(2).map_err(connection_error)?;
            Ok(CatalogNode {
                detail: Some(if not_null {
                    format!("{column_type}, not null")
                } else {
                    column_type
                }),
                ..node(path, NodeKind::Column, text(row, 0)?, false, false)
            })
        })
        .collect()
}

async fn indexes(
    client: &Client,
    path: &CatalogPath,
    schema: &str,
    relation: &str,
) -> Result<Vec<CatalogNode>, DriverError> {
    let sql = format!(
        "SELECT i.relname, x.indisprimary, x.indisunique \
         FROM pg_index x JOIN pg_class i ON i.oid = x.indexrelid \
         WHERE x.indrelid = {RELATION_OID} ORDER BY i.relname"
    );
    query(client, &sql, &[&schema, &relation])
        .await?
        .iter()
        .map(|row| {
            let primary: bool = row.try_get(1).map_err(connection_error)?;
            let unique: bool = row.try_get(2).map_err(connection_error)?;
            let detail = match (primary, unique) {
                (true, _) => Some("primary key".to_owned()),
                (false, true) => Some("unique".to_owned()),
                (false, false) => None,
            };
            Ok(CatalogNode {
                detail,
                ..node(path, NodeKind::Index, text(row, 0)?, false, true)
            })
        })
        .collect()
}

async fn constraints(
    client: &Client,
    path: &CatalogPath,
    schema: &str,
    relation: &str,
) -> Result<Vec<CatalogNode>, DriverError> {
    let sql = format!(
        "SELECT conname, contype::text FROM pg_constraint \
         WHERE conrelid = {RELATION_OID} ORDER BY conname"
    );
    query(client, &sql, &[&schema, &relation])
        .await?
        .iter()
        .map(|row| {
            let detail = match text(row, 1)?.as_str() {
                "p" => "primary key",
                "f" => "foreign key",
                "u" => "unique",
                "c" => "check",
                "x" => "exclusion",
                _ => "constraint",
            };
            Ok(CatalogNode {
                detail: Some(detail.to_owned()),
                ..node(path, NodeKind::Constraint, text(row, 0)?, false, true)
            })
        })
        .collect()
}

async fn triggers(
    client: &Client,
    path: &CatalogPath,
    schema: &str,
    relation: &str,
) -> Result<Vec<CatalogNode>, DriverError> {
    let sql = format!(
        "SELECT tgname FROM pg_trigger \
         WHERE tgrelid = {RELATION_OID} AND NOT tgisinternal ORDER BY tgname"
    );
    query(client, &sql, &[&schema, &relation])
        .await?
        .iter()
        .map(|row| Ok(node(path, NodeKind::Trigger, text(row, 0)?, false, true)))
        .collect()
}

pub(crate) async fn definition(client: &Client, path: &CatalogPath) -> Result<String, DriverError> {
    let invalid = || DriverError::InvalidPath(path.clone());
    let leaf = path.last().ok_or_else(invalid)?;
    let schema = path.name_of(NodeKind::Schema).ok_or_else(invalid)?;
    let qualified = path.qualified_name().ok_or_else(invalid)?;
    let name = leaf.name.as_str();

    match leaf.kind {
        NodeKind::View | NodeKind::MaterializedView => {
            let sql = format!("SELECT pg_get_viewdef({RELATION_OID}, true)");
            let body = single_text(client, path, &sql, &[&schema, &name]).await?;
            let keyword = if leaf.kind == NodeKind::View {
                "VIEW"
            } else {
                "MATERIALIZED VIEW"
            };
            Ok(format!("CREATE {keyword} {qualified} AS\n{body}"))
        }
        NodeKind::Function => {
            let sql = "SELECT pg_get_functiondef(p.oid) \
                       FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                       WHERE n.nspname = $1 \
                         AND p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')' = $2";
            single_text(client, path, sql, &[&schema, &name]).await
        }
        NodeKind::Index => {
            let sql = format!("SELECT pg_get_indexdef({RELATION_OID})");
            let body = single_text(client, path, &sql, &[&schema, &name]).await?;
            Ok(format!("{body};"))
        }
        NodeKind::Constraint => {
            let relation = relation_name(path).ok_or_else(invalid)?;
            let sql = format!(
                "SELECT pg_get_constraintdef(oid, true) FROM pg_constraint \
                 WHERE conrelid = {RELATION_OID} AND conname = $3"
            );
            let body = single_text(client, path, &sql, &[&schema, &relation, &name]).await?;
            Ok(format!(
                "ALTER TABLE {}.{} ADD CONSTRAINT {qualified} {body};",
                quote_identifier(schema),
                quote_identifier(relation)
            ))
        }
        NodeKind::Trigger => {
            let relation = relation_name(path).ok_or_else(invalid)?;
            let sql = format!(
                "SELECT pg_get_triggerdef(oid, true) FROM pg_trigger \
                 WHERE tgrelid = {RELATION_OID} AND tgname = $3"
            );
            let body = single_text(client, path, &sql, &[&schema, &relation, &name]).await?;
            Ok(format!("{body};"))
        }
        _ => Err(invalid()),
    }
}

/// The table or view a column-level node belongs to.
fn relation_name(path: &CatalogPath) -> Option<&str> {
    path.segments()
        .iter()
        .find(|segment| relation_folder(segment.kind).is_some())
        .map(|segment| segment.name.as_str())
}

/// Runs a query expected to return one text value. No row means the object is gone, which
/// is reported as an invalid path.
async fn single_text(
    client: &Client,
    path: &CatalogPath,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<String, DriverError> {
    let row = client
        .query_opt(sql, params)
        .await
        .map_err(connection_error)?
        .ok_or_else(|| DriverError::InvalidPath(path.clone()))?;
    text(&row, 0)
}
