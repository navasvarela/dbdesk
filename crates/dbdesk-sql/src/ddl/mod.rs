//! Statements that create and change database objects, written from plain descriptions of
//! what the user asked for: the object helpers' dialogs fill in a spec, and these pure
//! functions turn it into each dialect's SQL, so the SQL is tested without a database.
//!
//! Names in a spec are as the user typed them, never quoted. The builder quotes each one as
//! the dialect requires (see [`quote_name`]), and refuses what the dialect can't do, as its
//! [`Capabilities`](crate::Capabilities) say.

mod index;
mod reserved;
mod table;
#[cfg(test)]
mod tests;

use crate::Dialect;

/// A schema object's name. `schema` is `None` for an object in the connection's default
/// schema, which is what the database chooses for an unqualified name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectName {
    pub schema: Option<String>,
    pub name: String,
}

impl ObjectName {
    pub fn new(schema: Option<&str>, name: &str) -> Self {
        Self {
            schema: schema.map(str::to_owned),
            name: name.to_owned(),
        }
    }

    /// `schema.name`, quoted for `dialect`.
    pub fn to_sql(&self, dialect: Dialect) -> String {
        match &self.schema {
            Some(schema) => format!(
                "{}.{}",
                quote_name(dialect, schema),
                quote_name(dialect, &self.name)
            ),
            None => quote_name(dialect, &self.name),
        }
    }

    /// Another object in the same schema.
    fn sibling(&self, name: &str) -> Self {
        Self {
            schema: self.schema.clone(),
            name: name.to_owned(),
        }
    }
}

/// One column of a new table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnSpec {
    pub name: String,
    /// As the dialect spells it, e.g. `varchar(255)` or `Nullable(String)`: written as typed.
    pub data_type: String,
    pub nullable: bool,
    /// An SQL expression, written as typed after `DEFAULT`.
    pub default: Option<String>,
}

/// A new table: its columns and constraints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSpec {
    pub name: ObjectName,
    pub columns: Vec<ColumnSpec>,
    /// Column names, in key order. Empty for a table without a primary key.
    pub primary_key: Vec<String>,
    pub unique_keys: Vec<UniqueKey>,
    pub foreign_keys: Vec<ForeignKey>,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniqueKey {
    /// `None` lets the database name it.
    pub name: Option<String>,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKey {
    pub name: Option<String>,
    pub columns: Vec<String>,
    /// The table it points at, which may be the new table itself.
    pub target: ObjectName,
    /// The columns of `target` each of `columns` refers to, in the same order.
    pub target_columns: Vec<String>,
    pub on_delete: ReferentialAction,
    pub on_update: ReferentialAction,
}

/// What a foreign key does when the row it points at is deleted or its key updated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ReferentialAction {
    /// Refuse the change, checked at the end of the statement. Every dialect's default, so
    /// it is never written.
    #[default]
    NoAction,
    /// Refuse the change at once.
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ReferentialAction {
    pub fn sql(self) -> &'static str {
        match self {
            Self::NoAction => "NO ACTION",
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: Option<String>,
    /// An SQL condition, written as typed.
    pub expression: String,
}

/// A new index on a table, created in the table's schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSpec {
    pub name: String,
    pub table: ObjectName,
    /// Column names, in index order.
    pub columns: Vec<String>,
    pub unique: bool,
    /// `None` for the dialect's default method.
    pub method: Option<IndexMethod>,
}

/// How an index is organised, where a dialect offers a choice; see
/// [`Capabilities::index_methods`](crate::Capabilities::index_methods).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexMethod {
    BTree,
    Hash,
    /// PostgreSQL's inverted index, for arrays, `jsonb` and full-text search.
    Gin,
    /// PostgreSQL's generalised search tree, for geometry and ranges.
    Gist,
    /// PostgreSQL's block range index, small for large tables whose rows are stored in order.
    Brin,
}

impl IndexMethod {
    /// The method's name as the user picks it.
    pub fn label(self) -> &'static str {
        match self {
            Self::BTree => "B-tree",
            Self::Hash => "Hash",
            Self::Gin => "GIN",
            Self::Gist => "GiST",
            Self::Brin => "BRIN",
        }
    }

    fn keyword(self) -> &'static str {
        match self {
            Self::BTree => "btree",
            Self::Hash => "hash",
            Self::Gin => "gin",
            Self::Gist => "gist",
            Self::Brin => "brin",
        }
    }
}

/// One change to the database, which may take several statements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DdlOperation {
    CreateTable(TableSpec),
    RenameTable {
        table: ObjectName,
        /// The table stays in its schema.
        new_name: String,
    },
    /// Deletes every row, keeping the table.
    TruncateTable(ObjectName),
    DropTable {
        table: ObjectName,
        /// Also drop what depends on the table, where the dialect can.
        cascade: bool,
    },
    CreateIndex(IndexSpec),
    DropIndex {
        /// The table the index is on, which MySQL and SQL Server name in `DROP INDEX`.
        table: ObjectName,
        /// In the table's schema.
        index: String,
    },
}

/// Why a spec can't be written as SQL. The message is shown in the dialog, which stays open to
/// fix it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DdlError {
    #[error("The {0} needs a name.")]
    MissingName(&'static str),
    #[error("A table needs at least one column.")]
    NoColumns,
    #[error("Column {position} needs a name.")]
    UnnamedColumn { position: usize },
    #[error("Column \u{201c}{column}\u{201d} needs a type.")]
    UntypedColumn { column: String },
    #[error("There are two columns named \u{201c}{0}\u{201d}.")]
    DuplicateColumn(String),
    #[error("The {used_in} names \u{201c}{column}\u{201d}, which isn't a column of the table.")]
    UnknownColumn {
        column: String,
        used_in: &'static str,
    },
    #[error("The {0} needs at least one column.")]
    NoKeyColumns(&'static str),
    #[error("A foreign key needs as many target columns as columns.")]
    ForeignKeyColumnCount,
    #[error("A check needs a condition.")]
    EmptyCheck,
    #[error("{dialect} has no {what}.", dialect = .dialect.display_name())]
    Unsupported {
        what: &'static str,
        dialect: Dialect,
    },
}

impl DdlOperation {
    /// The statements that make this change on `dialect`, in the order to run them, without
    /// a terminating `;`.
    ///
    /// # Errors
    ///
    /// [`DdlError`] for a spec that is incomplete, names a column that isn't there, or asks
    /// for something the dialect can't do.
    pub fn statements(&self, dialect: Dialect) -> Result<Vec<String>, DdlError> {
        match self {
            Self::CreateTable(spec) => table::create(spec, dialect).map(|sql| vec![sql]),
            Self::RenameTable { table, new_name } => {
                table::rename(table, new_name, dialect).map(|sql| vec![sql])
            }
            Self::TruncateTable(table) => Ok(vec![table::truncate(table, dialect)]),
            Self::DropTable { table, cascade } => {
                table::drop(table, *cascade, dialect).map(|sql| vec![sql])
            }
            Self::CreateIndex(spec) => index::create(spec, dialect).map(|sql| vec![sql]),
            Self::DropIndex { table, index } => {
                index::drop(table, index, dialect).map(|sql| vec![sql])
            }
        }
    }
}

/// The statements for `operations`, one after the other.
///
/// # Errors
///
/// The first operation's [`DdlError`].
pub fn ddl_statements(
    operations: &[DdlOperation],
    dialect: Dialect,
) -> Result<Vec<String>, DdlError> {
    let mut statements = Vec::new();
    for operation in operations {
        statements.extend(operation.statements(dialect)?);
    }
    Ok(statements)
}

/// A name as generated DDL writes it: bare when it reads the same bare in every dialect
/// (lowercase, and reserved in none of them), quoted otherwise. Readable SQL matters here,
/// since the review panel teaches it; quoting every name would hide the statement's shape.
pub fn quote_name(dialect: Dialect, name: &str) -> String {
    let quoting = dialect.quoting();
    let bare = quoting.identifier_if_needed(name);
    if bare == name && !reserved::is_reserved(name) {
        bare
    } else {
        quoting.identifier(name)
    }
}

fn require_name(name: &str, what: &'static str) -> Result<(), DdlError> {
    if name.trim().is_empty() {
        Err(DdlError::MissingName(what))
    } else {
        Ok(())
    }
}

fn column_list(dialect: Dialect, columns: &[String]) -> String {
    let names: Vec<String> = columns
        .iter()
        .map(|column| quote_name(dialect, column))
        .collect();
    format!("({})", names.join(", "))
}
