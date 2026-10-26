//! What each dialect's DDL can do, for the object helpers: their menus and dialogs read these
//! facts instead of checking which engine they talk to, and the DDL builder refuses what a
//! dialect lacks.

use super::Dialect;
use crate::ddl::{IndexMethod, ReferentialAction};

/// What a dialect can create and change. See [`Dialect::capabilities`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Whether DDL runs inside a transaction, so that several statements are applied all or
    /// none. MySQL commits each one as it runs; ClickHouse has no transactions.
    pub transactional_ddl: bool,
    /// `DROP TABLE … CASCADE`, which also drops the views and keys that depend on the table.
    pub drop_cascade: bool,
    pub unique_keys: bool,
    pub foreign_keys: bool,
    /// What a foreign key may do when the row it points at is deleted or updated. Empty
    /// without foreign keys; otherwise [`ReferentialAction::NoAction`] comes first, as the
    /// default every dialect has.
    pub referential_actions: &'static [ReferentialAction],
    /// `CREATE INDEX` and `DROP INDEX`. ClickHouse's data-skipping indexes are another kind
    /// of object, with a type and a granularity, so it has none here.
    pub indexes: bool,
    /// The methods `CREATE INDEX` may name; empty where the dialect has only one.
    pub index_methods: &'static [IndexMethod],
    /// The changes an existing table takes in place.
    pub table_changes: &'static [TableChange],
    /// Whether Copy DDL rebuilds a table's `CREATE TABLE` from its description and the
    /// constraints in the catalog, because the database keeps no such statement. Elsewhere
    /// the catalog's table definition is the statement, or there is none to copy.
    pub rebuilds_table_ddl: bool,
    /// Column types the New table dialog suggests, in the order it lists them. Any other
    /// type can be typed in.
    pub common_types: &'static [&'static str],
}

/// A change to an existing table that the dialect makes without rebuilding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableChange {
    RenameTable,
    RenameColumn,
    AddColumn,
    DropColumn,
    ChangeColumnType,
    ChangeNullability,
    ChangeDefault,
    AddConstraint,
    DropConstraint,
}

const ALL_TABLE_CHANGES: &[TableChange] = &[
    TableChange::RenameTable,
    TableChange::RenameColumn,
    TableChange::AddColumn,
    TableChange::DropColumn,
    TableChange::ChangeColumnType,
    TableChange::ChangeNullability,
    TableChange::ChangeDefault,
    TableChange::AddConstraint,
    TableChange::DropConstraint,
];

const ALL_REFERENTIAL_ACTIONS: &[ReferentialAction] = &[
    ReferentialAction::NoAction,
    ReferentialAction::Restrict,
    ReferentialAction::Cascade,
    ReferentialAction::SetNull,
    ReferentialAction::SetDefault,
];

const POSTGRES: Capabilities = Capabilities {
    transactional_ddl: true,
    drop_cascade: true,
    unique_keys: true,
    foreign_keys: true,
    referential_actions: ALL_REFERENTIAL_ACTIONS,
    indexes: true,
    index_methods: &[
        IndexMethod::BTree,
        IndexMethod::Hash,
        IndexMethod::Gin,
        IndexMethod::Gist,
        IndexMethod::Brin,
    ],
    table_changes: ALL_TABLE_CHANGES,
    rebuilds_table_ddl: true,
    common_types: &[
        "integer",
        "bigint",
        "smallint",
        "serial",
        "bigserial",
        "numeric(12, 2)",
        "real",
        "double precision",
        "boolean",
        "text",
        "varchar(255)",
        "char(1)",
        "date",
        "time",
        "timestamp",
        "timestamptz",
        "interval",
        "uuid",
        "json",
        "jsonb",
        "bytea",
    ],
};

/// SQLite's `ALTER TABLE` renames tables and columns and adds or drops simple columns; any
/// other change rebuilds the table (E4). `TRUNCATE` is written as `DELETE FROM`.
const SQLITE: Capabilities = Capabilities {
    transactional_ddl: true,
    drop_cascade: false,
    unique_keys: true,
    foreign_keys: true,
    referential_actions: ALL_REFERENTIAL_ACTIONS,
    indexes: true,
    index_methods: &[],
    table_changes: &[
        TableChange::RenameTable,
        TableChange::RenameColumn,
        TableChange::AddColumn,
        TableChange::DropColumn,
    ],
    rebuilds_table_ddl: false,
    common_types: &["integer", "real", "text", "blob", "numeric"],
};

/// InnoDB parses `SET DEFAULT` but refuses the table, so it isn't offered. `CASCADE` after
/// `DROP TABLE` is accepted and does nothing.
const MYSQL: Capabilities = Capabilities {
    transactional_ddl: false,
    drop_cascade: false,
    unique_keys: true,
    foreign_keys: true,
    referential_actions: &[
        ReferentialAction::NoAction,
        ReferentialAction::Restrict,
        ReferentialAction::Cascade,
        ReferentialAction::SetNull,
    ],
    indexes: true,
    index_methods: &[IndexMethod::BTree, IndexMethod::Hash],
    table_changes: ALL_TABLE_CHANGES,
    rebuilds_table_ddl: false,
    common_types: &[
        "int",
        "bigint",
        "smallint",
        "tinyint",
        "decimal(12, 2)",
        "float",
        "double",
        "boolean",
        "varchar(255)",
        "char(1)",
        "text",
        "longtext",
        "date",
        "datetime",
        "timestamp",
        "time",
        "json",
        "blob",
        "binary(16)",
    ],
};

/// SQL Server has no `RESTRICT`: `NO ACTION` is what refuses. Its renames are `sp_rename`
/// calls rather than `ALTER TABLE`, but they change the table in place all the same.
const SQL_SERVER: Capabilities = Capabilities {
    transactional_ddl: true,
    drop_cascade: false,
    unique_keys: true,
    foreign_keys: true,
    referential_actions: &[
        ReferentialAction::NoAction,
        ReferentialAction::Cascade,
        ReferentialAction::SetNull,
        ReferentialAction::SetDefault,
    ],
    indexes: true,
    index_methods: &[],
    table_changes: ALL_TABLE_CHANGES,
    rebuilds_table_ddl: false,
    common_types: &[
        "int",
        "bigint",
        "smallint",
        "tinyint",
        "bit",
        "decimal(12, 2)",
        "float",
        "real",
        "money",
        "nvarchar(255)",
        "nvarchar(max)",
        "varchar(255)",
        "char(1)",
        "date",
        "datetime2",
        "datetimeoffset",
        "time",
        "uniqueidentifier",
        "varbinary(max)",
    ],
};

/// DuckDB's foreign keys only refuse: it has no `ON DELETE` or `ON UPDATE` actions. Its
/// `ALTER TABLE` can't add or drop constraints.
const DUCKDB: Capabilities = Capabilities {
    transactional_ddl: true,
    drop_cascade: false,
    unique_keys: true,
    foreign_keys: true,
    referential_actions: &[ReferentialAction::NoAction],
    indexes: true,
    index_methods: &[],
    table_changes: &[
        TableChange::RenameTable,
        TableChange::RenameColumn,
        TableChange::AddColumn,
        TableChange::DropColumn,
        TableChange::ChangeColumnType,
        TableChange::ChangeNullability,
        TableChange::ChangeDefault,
    ],
    rebuilds_table_ddl: false,
    common_types: &[
        "integer",
        "bigint",
        "smallint",
        "hugeint",
        "decimal(18, 3)",
        "double",
        "real",
        "boolean",
        "varchar",
        "date",
        "time",
        "timestamp",
        "timestamptz",
        "interval",
        "uuid",
        "json",
        "blob",
    ],
};

/// ClickHouse has no unique or foreign keys, only checks; its `PRIMARY KEY` is the sort
/// order, which the builder writes as `ORDER BY`.
const CLICKHOUSE: Capabilities = Capabilities {
    transactional_ddl: false,
    drop_cascade: false,
    unique_keys: false,
    foreign_keys: false,
    referential_actions: &[],
    indexes: false,
    index_methods: &[],
    table_changes: ALL_TABLE_CHANGES,
    rebuilds_table_ddl: false,
    common_types: &[
        "UInt8",
        "UInt32",
        "UInt64",
        "Int32",
        "Int64",
        "Float32",
        "Float64",
        "Decimal(18, 4)",
        "Bool",
        "String",
        "LowCardinality(String)",
        "FixedString(16)",
        "UUID",
        "Date",
        "Date32",
        "DateTime",
        "DateTime64(3)",
        "Array(String)",
        "Map(String, String)",
    ],
};

impl Dialect {
    /// What the dialect's DDL can do.
    pub fn capabilities(self) -> &'static Capabilities {
        match self {
            Self::Postgres => &POSTGRES,
            Self::Sqlite => &SQLITE,
            Self::Mysql => &MYSQL,
            Self::SqlServer => &SQL_SERVER,
            Self::DuckDb => &DUCKDB,
            Self::ClickHouse => &CLICKHOUSE,
        }
    }
}

impl Capabilities {
    pub fn changes_in_place(&self, change: TableChange) -> bool {
        self.table_changes.contains(&change)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_keys_offer_no_action_first_and_only_where_they_exist() {
        for dialect in Dialect::ALL {
            let capabilities = dialect.capabilities();
            if capabilities.foreign_keys {
                assert_eq!(
                    capabilities.referential_actions.first(),
                    Some(&ReferentialAction::NoAction),
                    "{dialect:?}"
                );
            } else {
                assert!(capabilities.referential_actions.is_empty(), "{dialect:?}");
            }
        }
    }

    #[test]
    fn index_methods_need_indexes() {
        for dialect in Dialect::ALL {
            let capabilities = dialect.capabilities();
            assert!(
                capabilities.indexes || capabilities.index_methods.is_empty(),
                "{dialect:?}"
            );
            assert!(!capabilities.common_types.is_empty(), "{dialect:?}");
        }
    }
}
