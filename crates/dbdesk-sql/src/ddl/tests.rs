//! Golden tests: what each dialect's DDL looks like, written out in full.

use std::collections::BTreeMap;

use dbdesk_core::RoleAttribute;

use super::*;
use crate::UserModel;

fn column(name: &str, data_type: &str, nullable: bool, default: Option<&str>) -> ColumnSpec {
    ColumnSpec {
        name: name.to_owned(),
        data_type: data_type.to_owned(),
        nullable,
        default: default.map(str::to_owned),
        identity: None,
    }
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// An order line with every kind of constraint, and names that need quoting: a keyword
/// (`order`), capitals (`Note`) and a space (`line items`).
fn order_lines(dialect: Dialect) -> TableSpec {
    let quantity = match dialect {
        Dialect::ClickHouse => "UInt32",
        _ => "integer",
    };
    TableSpec {
        name: ObjectName::new(Some("shop"), "line items"),
        columns: vec![
            column("order", quantity, false, None),
            column("line", quantity, false, None),
            column("sku", "varchar(20)", false, None),
            column("quantity", quantity, false, Some("1")),
            column("Note", "text", true, None),
        ],
        primary_key: strings(&["order", "line"]),
        unique_keys: vec![UniqueKey {
            name: Some("line_items_sku_key".to_owned()),
            columns: strings(&["order", "sku"]),
        }],
        foreign_keys: vec![ForeignKey {
            name: None,
            columns: strings(&["order"]),
            target: ObjectName::new(Some("shop"), "orders"),
            target_columns: strings(&["id"]),
            on_delete: ReferentialAction::Cascade,
            on_update: ReferentialAction::NoAction,
        }],
        checks: vec![Check {
            name: None,
            expression: "quantity > 0".to_owned(),
        }],
    }
}

#[test]
fn creates_a_table_with_every_kind_of_constraint() {
    let cases = [
        (
            Dialect::Postgres,
            r#"CREATE TABLE shop."line items" (
    "order" integer NOT NULL,
    line integer NOT NULL,
    sku varchar(20) NOT NULL,
    quantity integer DEFAULT 1 NOT NULL,
    "Note" text,
    PRIMARY KEY ("order", line),
    CONSTRAINT line_items_sku_key UNIQUE ("order", sku),
    FOREIGN KEY ("order") REFERENCES shop.orders (id) ON DELETE CASCADE,
    CHECK (quantity > 0)
)"#,
        ),
        (
            Dialect::Sqlite,
            r#"CREATE TABLE shop."line items" (
    "order" integer NOT NULL,
    line integer NOT NULL,
    sku varchar(20) NOT NULL,
    quantity integer DEFAULT 1 NOT NULL,
    "Note" text,
    PRIMARY KEY ("order", line),
    CONSTRAINT line_items_sku_key UNIQUE ("order", sku),
    FOREIGN KEY ("order") REFERENCES orders (id) ON DELETE CASCADE,
    CHECK (quantity > 0)
)"#,
        ),
        (
            Dialect::Mysql,
            "CREATE TABLE shop.`line items` (
    `order` integer NOT NULL,
    line integer NOT NULL,
    sku varchar(20) NOT NULL,
    quantity integer DEFAULT 1 NOT NULL,
    `Note` text,
    PRIMARY KEY (`order`, line),
    CONSTRAINT line_items_sku_key UNIQUE (`order`, sku),
    FOREIGN KEY (`order`) REFERENCES shop.orders (id) ON DELETE CASCADE,
    CHECK (quantity > 0)
)",
        ),
        (
            Dialect::SqlServer,
            "CREATE TABLE shop.[line items] (
    [order] integer NOT NULL,
    line integer NOT NULL,
    sku varchar(20) NOT NULL,
    quantity integer DEFAULT 1 NOT NULL,
    [Note] text NULL,
    PRIMARY KEY ([order], line),
    CONSTRAINT line_items_sku_key UNIQUE ([order], sku),
    FOREIGN KEY ([order]) REFERENCES shop.orders (id) ON DELETE CASCADE,
    CHECK (quantity > 0)
)",
        ),
        (
            Dialect::ClickHouse,
            "CREATE TABLE shop.`line items` (
    `order` UInt32,
    line UInt32,
    sku varchar(20),
    quantity UInt32 DEFAULT 1,
    `Note` Nullable(text),
    CONSTRAINT `line items_check_1` CHECK quantity > 0
)
ENGINE = MergeTree
ORDER BY (`order`, line)",
        ),
    ];
    for (dialect, expected) in cases {
        let mut spec = order_lines(dialect);
        if dialect == Dialect::ClickHouse {
            spec.unique_keys.clear();
            spec.foreign_keys.clear();
        }
        let statements = DdlOperation::CreateTable(spec).statements(dialect);
        assert_eq!(statements, Ok(vec![expected.to_owned()]), "{dialect:?}");
    }
}

#[test]
fn duckdb_writes_standard_sql_without_referential_actions() {
    let mut spec = order_lines(Dialect::DuckDb);
    spec.foreign_keys[0].on_delete = ReferentialAction::NoAction;
    let statements = DdlOperation::CreateTable(spec.clone()).statements(Dialect::DuckDb);
    assert_eq!(
        statements,
        Ok(vec![
            r#"CREATE TABLE shop."line items" (
    "order" integer NOT NULL,
    line integer NOT NULL,
    sku varchar(20) NOT NULL,
    quantity integer DEFAULT 1 NOT NULL,
    "Note" text,
    PRIMARY KEY ("order", line),
    CONSTRAINT line_items_sku_key UNIQUE ("order", sku),
    FOREIGN KEY ("order") REFERENCES shop.orders (id),
    CHECK (quantity > 0)
)"#
            .to_owned()
        ])
    );
    spec.foreign_keys[0].on_delete = ReferentialAction::Cascade;
    assert_eq!(
        DdlOperation::CreateTable(spec)
            .statements(Dialect::DuckDb)
            .map_err(|error| error.to_string()),
        Err("DuckDB has no CASCADE for foreign keys.".to_owned())
    );
}

#[test]
fn a_table_without_a_key_or_constraints_is_only_its_columns() {
    let spec = TableSpec {
        name: ObjectName::new(None, "notes"),
        columns: vec![column("body", "text", true, None)],
        primary_key: vec![],
        unique_keys: vec![],
        foreign_keys: vec![],
        checks: vec![],
    };
    let cases = [
        (Dialect::Postgres, "CREATE TABLE notes (\n    body text\n)"),
        (
            Dialect::ClickHouse,
            "CREATE TABLE notes (\n    body Nullable(text)\n)\nENGINE = MergeTree\nORDER BY tuple()",
        ),
    ];
    for (dialect, expected) in cases {
        let statements = DdlOperation::CreateTable(spec.clone()).statements(dialect);
        assert_eq!(statements, Ok(vec![expected.to_owned()]), "{dialect:?}");
    }
}

/// Breaks a valid spec in one way.
type Change = fn(&mut TableSpec);

#[test]
fn refuses_incomplete_or_inconsistent_tables() {
    let valid = || TableSpec {
        name: ObjectName::new(None, "orders"),
        columns: vec![column("id", "integer", false, None)],
        primary_key: strings(&["id"]),
        unique_keys: vec![],
        foreign_keys: vec![],
        checks: vec![],
    };
    let cases: [(Change, DdlError); 8] = [
        (
            |spec| spec.name.name = " ".to_owned(),
            DdlError::MissingName("table"),
        ),
        (|spec| spec.columns.clear(), DdlError::NoColumns),
        (
            |spec| spec.columns[0].name.clear(),
            DdlError::UnnamedColumn { position: 1 },
        ),
        (
            |spec| spec.columns[0].data_type.clear(),
            DdlError::UntypedColumn {
                column: "id".to_owned(),
            },
        ),
        (
            |spec| spec.columns.push(column("id", "text", true, None)),
            DdlError::DuplicateColumn("id".to_owned()),
        ),
        (
            |spec| spec.primary_key = strings(&["code"]),
            DdlError::UnknownColumn {
                column: "code".to_owned(),
                used_in: "primary key",
            },
        ),
        (
            |spec| {
                spec.foreign_keys.push(ForeignKey {
                    name: None,
                    columns: strings(&["id"]),
                    target: ObjectName::new(None, "customers"),
                    target_columns: strings(&["id", "region"]),
                    on_delete: ReferentialAction::NoAction,
                    on_update: ReferentialAction::NoAction,
                });
            },
            DdlError::ForeignKeyColumnCount,
        ),
        (
            |spec| {
                spec.checks.push(Check {
                    name: None,
                    expression: "  ".to_owned(),
                });
            },
            DdlError::EmptyCheck,
        ),
    ];
    for (change, expected) in cases {
        let mut spec = valid();
        change(&mut spec);
        assert_eq!(
            DdlOperation::CreateTable(spec).statements(Dialect::Postgres),
            Err(expected.clone()),
            "{expected}"
        );
    }
}

#[test]
fn refuses_what_the_dialect_lacks() {
    let keyed = |dialect| {
        let mut spec = order_lines(dialect);
        spec.foreign_keys.clear();
        spec
    };
    let cases = [
        (
            DdlOperation::CreateTable(keyed(Dialect::ClickHouse)),
            Dialect::ClickHouse,
            "ClickHouse has no unique keys.",
        ),
        (
            DdlOperation::CreateTable(order_lines(Dialect::Mysql))
                .with_foreign_key_action(ReferentialAction::SetDefault),
            Dialect::Mysql,
            "MySQL has no SET DEFAULT for foreign keys.",
        ),
        (
            DdlOperation::CreateTable(order_lines(Dialect::Sqlite))
                .with_foreign_key_target(ObjectName::new(Some("main"), "orders")),
            Dialect::Sqlite,
            "SQLite has no foreign keys to a table in another schema.",
        ),
        (
            DdlOperation::DropTable {
                table: ObjectName::new(None, "orders"),
                cascade: true,
            },
            Dialect::Sqlite,
            "SQLite has no CASCADE for dropping a table.",
        ),
        (
            DdlOperation::CreateIndex(IndexSpec {
                name: "events_at".to_owned(),
                table: ObjectName::new(None, "events"),
                columns: strings(&["at"]),
                unique: false,
                method: None,
            }),
            Dialect::ClickHouse,
            "ClickHouse has no indexes dbdesk can create.",
        ),
        (
            DdlOperation::CreateIndex(IndexSpec {
                name: "events_tags".to_owned(),
                table: ObjectName::new(None, "events"),
                columns: strings(&["tags"]),
                unique: false,
                method: Some(IndexMethod::Gin),
            }),
            Dialect::Mysql,
            "MySQL has no GIN.",
        ),
    ];
    for (operation, dialect, expected) in cases {
        assert_eq!(
            operation
                .statements(dialect)
                .map_err(|error| error.to_string()),
            Err(expected.to_owned())
        );
    }
}

impl DdlOperation {
    fn with_foreign_key_action(mut self, action: ReferentialAction) -> Self {
        if let Self::CreateTable(spec) = &mut self {
            spec.foreign_keys[0].on_update = action;
        }
        self
    }

    fn with_foreign_key_target(mut self, target: ObjectName) -> Self {
        if let Self::CreateTable(spec) = &mut self {
            spec.foreign_keys[0].target = target;
        }
        self
    }
}

#[test]
fn renames_empties_and_drops_tables() {
    let table = ObjectName::new(Some("shop"), "orders");
    let rename = DdlOperation::RenameTable {
        table: table.clone(),
        new_name: "Old Orders".to_owned(),
    };
    let truncate = DdlOperation::TruncateTable(table.clone());
    let drop = DdlOperation::DropTable {
        table: table.clone(),
        cascade: false,
    };
    let cases = [
        (
            Dialect::Postgres,
            [
                r#"ALTER TABLE shop.orders RENAME TO "Old Orders""#,
                "TRUNCATE TABLE shop.orders",
                "DROP TABLE shop.orders",
            ],
        ),
        (
            Dialect::Sqlite,
            [
                r#"ALTER TABLE shop.orders RENAME TO "Old Orders""#,
                "DELETE FROM shop.orders",
                "DROP TABLE shop.orders",
            ],
        ),
        (
            Dialect::Mysql,
            [
                "RENAME TABLE shop.orders TO shop.`Old Orders`",
                "TRUNCATE TABLE shop.orders",
                "DROP TABLE shop.orders",
            ],
        ),
        (
            Dialect::SqlServer,
            [
                "EXEC sp_rename N'shop.orders', N'Old Orders'",
                "TRUNCATE TABLE shop.orders",
                "DROP TABLE shop.orders",
            ],
        ),
        (
            Dialect::DuckDb,
            [
                r#"ALTER TABLE shop.orders RENAME TO "Old Orders""#,
                "TRUNCATE TABLE shop.orders",
                "DROP TABLE shop.orders",
            ],
        ),
        (
            Dialect::ClickHouse,
            [
                "RENAME TABLE shop.orders TO shop.`Old Orders`",
                "TRUNCATE TABLE shop.orders",
                "DROP TABLE shop.orders",
            ],
        ),
    ];
    for (dialect, expected) in cases {
        let statements = ddl_statements(&[rename.clone(), truncate.clone(), drop.clone()], dialect);
        assert_eq!(
            statements,
            Ok(expected.map(str::to_owned).to_vec()),
            "{dialect:?}"
        );
    }
    assert_eq!(
        DdlOperation::DropTable {
            table,
            cascade: true
        }
        .statements(Dialect::Postgres),
        Ok(vec!["DROP TABLE shop.orders CASCADE".to_owned()])
    );
}

#[test]
fn sql_server_renames_a_table_whose_names_need_brackets() {
    let rename = DdlOperation::RenameTable {
        table: ObjectName::new(Some("dbo"), "order's items"),
        new_name: "items".to_owned(),
    };
    assert_eq!(
        rename.statements(Dialect::SqlServer),
        Ok(vec![
            "EXEC sp_rename N'dbo.[order''s items]', N'items'".to_owned()
        ])
    );
}

#[test]
fn creates_and_drops_indexes() {
    let table = ObjectName::new(Some("shop"), "orders");
    let create = |method| {
        DdlOperation::CreateIndex(IndexSpec {
            name: "orders_customer_placed".to_owned(),
            table: table.clone(),
            columns: strings(&["customer", "placed at"]),
            unique: true,
            method,
        })
    };
    let drop = DdlOperation::DropIndex {
        table: table.clone(),
        index: "orders_customer_placed".to_owned(),
    };
    let cases = [
        (
            Dialect::Postgres,
            Some(IndexMethod::BTree),
            [
                r#"CREATE UNIQUE INDEX orders_customer_placed ON shop.orders USING btree (customer, "placed at")"#,
                "DROP INDEX shop.orders_customer_placed",
            ],
        ),
        (
            Dialect::Sqlite,
            None,
            [
                r#"CREATE UNIQUE INDEX shop.orders_customer_placed ON orders (customer, "placed at")"#,
                "DROP INDEX shop.orders_customer_placed",
            ],
        ),
        (
            Dialect::Mysql,
            Some(IndexMethod::BTree),
            [
                "CREATE UNIQUE INDEX orders_customer_placed ON shop.orders (customer, `placed at`) USING BTREE",
                "DROP INDEX orders_customer_placed ON shop.orders",
            ],
        ),
        (
            Dialect::SqlServer,
            None,
            [
                "CREATE UNIQUE INDEX orders_customer_placed ON shop.orders (customer, [placed at])",
                "DROP INDEX orders_customer_placed ON shop.orders",
            ],
        ),
        (
            Dialect::DuckDb,
            None,
            [
                r#"CREATE UNIQUE INDEX orders_customer_placed ON shop.orders (customer, "placed at")"#,
                "DROP INDEX shop.orders_customer_placed",
            ],
        ),
    ];
    for (dialect, method, expected) in cases {
        let statements = ddl_statements(&[create(method), drop.clone()], dialect);
        assert_eq!(
            statements,
            Ok(expected.map(str::to_owned).to_vec()),
            "{dialect:?}"
        );
    }
}

#[test]
fn refuses_an_index_without_columns_or_naming_one_twice() {
    let index = |columns: &[&str]| {
        DdlOperation::CreateIndex(IndexSpec {
            name: "orders_customer".to_owned(),
            table: ObjectName::new(None, "orders"),
            columns: strings(columns),
            unique: false,
            method: None,
        })
    };
    assert_eq!(
        index(&[]).statements(Dialect::Postgres),
        Err(DdlError::NoKeyColumns("index"))
    );
    assert_eq!(
        index(&["customer", "customer"]).statements(Dialect::Postgres),
        Err(DdlError::DuplicateColumn("customer".to_owned()))
    );
}

#[test]
fn quotes_only_names_that_need_it() {
    let cases = [
        (Dialect::Postgres, "orders", "orders"),
        (Dialect::Postgres, "order", "\"order\""),
        (Dialect::Postgres, "User", "\"User\""),
        (Dialect::Postgres, "user", "\"user\""),
        (Dialect::Postgres, "line_2", "line_2"),
        (Dialect::Postgres, "2nd", "\"2nd\""),
        (Dialect::Mysql, "key", "`key`"),
        (Dialect::SqlServer, "a]b", "[a]]b]"),
        (Dialect::ClickHouse, "Events", "`Events`"),
    ];
    for (dialect, name, expected) in cases {
        assert_eq!(quote_name(dialect, name), expected, "{dialect:?} {name}");
    }
}

fn big_orders(kind: ViewKind) -> ViewSpec {
    ViewSpec {
        name: ObjectName::new(Some("shop"), "Big Orders"),
        kind,
        query: "SELECT id, total\nFROM shop.orders\nWHERE total > 100;\n".to_owned(),
        with_data: true,
    }
}

#[test]
fn creates_replaces_renames_and_drops_views() {
    let view = big_orders(ViewKind::View);
    let operations = [
        DdlOperation::CreateView(view.clone()),
        DdlOperation::ReplaceView(view.clone()),
        DdlOperation::RenameView {
            view: view.name.clone(),
            kind: ViewKind::View,
            new_name: "large_orders".to_owned(),
        },
        DdlOperation::DropView {
            view: view.name.clone(),
            kind: ViewKind::View,
            cascade: false,
        },
    ];
    let query = "SELECT id, total\nFROM shop.orders\nWHERE total > 100";
    let cases: [(Dialect, &[&str]); 5] = [
        (
            Dialect::Postgres,
            &[
                r#"CREATE VIEW shop."Big Orders" AS"#,
                r#"CREATE OR REPLACE VIEW shop."Big Orders" AS"#,
                r#"ALTER VIEW shop."Big Orders" RENAME TO large_orders"#,
                r#"DROP VIEW shop."Big Orders""#,
            ],
        ),
        (
            Dialect::Mysql,
            &[
                "CREATE VIEW shop.`Big Orders` AS",
                "CREATE OR REPLACE VIEW shop.`Big Orders` AS",
                "RENAME TABLE shop.`Big Orders` TO shop.large_orders",
                "DROP VIEW shop.`Big Orders`",
            ],
        ),
        (
            Dialect::SqlServer,
            &[
                "CREATE VIEW shop.[Big Orders] AS",
                "ALTER VIEW shop.[Big Orders] AS",
                "EXEC sp_rename N'shop.[Big Orders]', N'large_orders'",
                "DROP VIEW shop.[Big Orders]",
            ],
        ),
        (
            Dialect::DuckDb,
            &[
                r#"CREATE VIEW shop."Big Orders" AS"#,
                r#"CREATE OR REPLACE VIEW shop."Big Orders" AS"#,
                r#"ALTER VIEW shop."Big Orders" RENAME TO large_orders"#,
                r#"DROP VIEW shop."Big Orders""#,
            ],
        ),
        (
            Dialect::ClickHouse,
            &[
                "CREATE VIEW shop.`Big Orders` AS",
                "CREATE OR REPLACE VIEW shop.`Big Orders` AS",
                "RENAME TABLE shop.`Big Orders` TO shop.large_orders",
                "DROP VIEW shop.`Big Orders`",
            ],
        ),
    ];
    for (dialect, expected) in cases {
        let with_query = |statement: &str| match statement.strip_suffix(" AS") {
            Some(_) => format!("{statement}\n{query}"),
            None => statement.to_owned(),
        };
        assert_eq!(
            ddl_statements(&operations, dialect),
            Ok(expected
                .iter()
                .map(|statement| with_query(statement))
                .collect()),
            "{dialect:?}"
        );
    }
}

#[test]
fn sqlite_replaces_a_view_by_dropping_and_creating_it_and_cannot_rename_one() {
    let view = big_orders(ViewKind::View);
    assert_eq!(
        DdlOperation::ReplaceView(view.clone()).statements(Dialect::Sqlite),
        Ok(vec![
            r#"DROP VIEW shop."Big Orders""#.to_owned(),
            "CREATE VIEW shop.\"Big Orders\" AS\nSELECT id, total\nFROM shop.orders\nWHERE total > 100"
                .to_owned(),
        ])
    );
    let rename = DdlOperation::RenameView {
        view: view.name,
        kind: ViewKind::View,
        new_name: "large_orders".to_owned(),
    };
    assert_eq!(
        rename.statements(Dialect::Sqlite),
        Err(DdlError::Unsupported {
            what: "way to rename a view",
            dialect: Dialect::Sqlite
        })
    );
}

#[test]
fn postgres_creates_refreshes_and_drops_materialized_views() {
    let view = big_orders(ViewKind::Materialized);
    let empty = ViewSpec {
        with_data: false,
        ..view.clone()
    };
    let operations = [
        DdlOperation::CreateView(empty.clone()),
        DdlOperation::RefreshMaterializedView {
            view: view.name.clone(),
            concurrently: true,
        },
        DdlOperation::ReplaceView(view.clone()),
        DdlOperation::RenameView {
            view: view.name.clone(),
            kind: ViewKind::Materialized,
            new_name: "large_orders".to_owned(),
        },
        DdlOperation::DropView {
            view: view.name.clone(),
            kind: ViewKind::Materialized,
            cascade: true,
        },
    ];
    let create = "CREATE MATERIALIZED VIEW shop.\"Big Orders\" AS\nSELECT id, total\nFROM \
                  shop.orders\nWHERE total > 100";
    assert_eq!(
        ddl_statements(&operations, Dialect::Postgres),
        Ok(strings(&[
            &format!("{create}\nWITH NO DATA"),
            r#"REFRESH MATERIALIZED VIEW CONCURRENTLY shop."Big Orders""#,
            r#"DROP MATERIALIZED VIEW shop."Big Orders""#,
            create,
            r#"ALTER MATERIALIZED VIEW shop."Big Orders" RENAME TO large_orders"#,
            r#"DROP MATERIALIZED VIEW shop."Big Orders" CASCADE"#,
        ]))
    );
}

#[test]
fn refuses_views_without_a_name_or_query_or_that_the_dialect_lacks() {
    let unnamed = ViewSpec {
        name: ObjectName::new(None, " "),
        ..big_orders(ViewKind::View)
    };
    let empty = ViewSpec {
        query: " ;\n".to_owned(),
        ..big_orders(ViewKind::Materialized)
    };
    let cases = [
        (
            DdlOperation::CreateView(unnamed),
            Dialect::Postgres,
            DdlError::MissingName("view"),
        ),
        (
            DdlOperation::CreateView(empty),
            Dialect::Postgres,
            DdlError::EmptyQuery("materialized view"),
        ),
        (
            DdlOperation::CreateView(big_orders(ViewKind::Materialized)),
            Dialect::ClickHouse,
            DdlError::Unsupported {
                what: "materialized views dbdesk can create",
                dialect: Dialect::ClickHouse,
            },
        ),
        (
            DdlOperation::RefreshMaterializedView {
                view: ObjectName::new(None, "big_orders"),
                concurrently: false,
            },
            Dialect::DuckDb,
            DdlError::Unsupported {
                what: "materialized views dbdesk can create",
                dialect: Dialect::DuckDb,
            },
        ),
        (
            DdlOperation::DropView {
                view: ObjectName::new(None, "big_orders"),
                kind: ViewKind::View,
                cascade: true,
            },
            Dialect::Mysql,
            DdlError::Unsupported {
                what: "CASCADE for dropping a view",
                dialect: Dialect::Mysql,
            },
        ),
    ];
    for (operation, dialect, expected) in cases {
        assert_eq!(
            operation.statements(dialect),
            Err(expected),
            "{operation:?}"
        );
    }
}

#[test]
fn finds_the_query_in_each_engines_view_definition() {
    let cases = [
        (
            Dialect::Postgres,
            "CREATE VIEW public.big_orders AS\n SELECT id,\n    total\n   FROM orders\n  WHERE \
             total > 100::numeric;",
            " SELECT id,\n    total\n   FROM orders\n  WHERE total > 100::numeric".trim_start(),
        ),
        (
            Dialect::Sqlite,
            "CREATE VIEW big_orders(id, total) AS SELECT id, total FROM orders WHERE total > 100",
            "SELECT id, total FROM orders WHERE total > 100",
        ),
        (
            Dialect::Mysql,
            "CREATE ALGORITHM=UNDEFINED DEFINER=`dbdesk`@`%` SQL SECURITY DEFINER VIEW \
             `big_orders` AS select `orders`.`id` AS `id` from `orders` where (`orders`.`total` > \
             100)",
            "select `orders`.`id` AS `id` from `orders` where (`orders`.`total` > 100)",
        ),
        (
            Dialect::SqlServer,
            "-- the big ones\nCREATE VIEW dbo.big_orders\nWITH SCHEMABINDING\nAS\nSELECT id \
             FROM dbo.orders WHERE total > 100\n",
            "SELECT id FROM dbo.orders WHERE total > 100",
        ),
        (
            Dialect::DuckDb,
            "CREATE VIEW big_orders AS WITH big AS (SELECT * FROM orders WHERE total > 100) \
             SELECT id FROM big;",
            "WITH big AS (SELECT * FROM orders WHERE total > 100) SELECT id FROM big",
        ),
        (
            Dialect::ClickHouse,
            "CREATE VIEW dbdesk.big_orders\n(\n    `id` Int32\n)\nAS SELECT id\nFROM \
             dbdesk.orders\nWHERE total > 100",
            "SELECT id\nFROM dbdesk.orders\nWHERE total > 100",
        ),
    ];
    for (dialect, definition, expected) in cases {
        assert_eq!(
            view_query(definition, dialect).as_deref(),
            Some(expected),
            "{dialect:?}"
        );
    }
    for not_a_view in [
        "SELECT 1",
        "CREATE TABLE t AS SELECT 1",
        "CREATE VIEW v AS",
        "",
    ] {
        assert_eq!(
            view_query(not_a_view, Dialect::Postgres),
            None,
            "{not_a_view}"
        );
    }
}

#[test]
fn creates_and_drops_schemas_or_databases() {
    let schema = |owner: Option<&str>| {
        DdlOperation::CreateSchema(SchemaSpec {
            name: "Sales".to_owned(),
            owner: owner.map(str::to_owned),
        })
    };
    let drop = |cascade| DdlOperation::DropSchema {
        schema: "Sales".to_owned(),
        cascade,
    };
    let cases = [
        (
            Dialect::Postgres,
            [schema(Some("reporting")), drop(true)],
            [
                r#"CREATE SCHEMA "Sales" AUTHORIZATION reporting"#,
                r#"DROP SCHEMA "Sales" CASCADE"#,
            ],
        ),
        (
            Dialect::SqlServer,
            [schema(Some("dbo")), drop(false)],
            [
                "CREATE SCHEMA [Sales] AUTHORIZATION dbo",
                "DROP SCHEMA [Sales]",
            ],
        ),
        (
            Dialect::DuckDb,
            [schema(None), drop(true)],
            [r#"CREATE SCHEMA "Sales""#, r#"DROP SCHEMA "Sales" CASCADE"#],
        ),
        (
            Dialect::Mysql,
            [schema(None), drop(false)],
            ["CREATE DATABASE `Sales`", "DROP DATABASE `Sales`"],
        ),
        (
            Dialect::ClickHouse,
            [schema(None), drop(true)],
            ["CREATE DATABASE `Sales`", "DROP DATABASE `Sales`"],
        ),
    ];
    for (dialect, operations, expected) in cases {
        assert_eq!(
            ddl_statements(&operations, dialect),
            Ok(strings(&expected)),
            "{dialect:?}"
        );
    }
}

#[test]
fn refuses_schemas_the_dialect_cannot_create_or_drop() {
    let cases = [
        (
            DdlOperation::CreateSchema(SchemaSpec {
                name: "sales".to_owned(),
                owner: None,
            }),
            Dialect::Sqlite,
            "schemas dbdesk can create or drop",
        ),
        (
            DdlOperation::DropSchema {
                schema: "sales".to_owned(),
                cascade: false,
            },
            Dialect::Sqlite,
            "schemas dbdesk can create or drop",
        ),
        (
            DdlOperation::CreateSchema(SchemaSpec {
                name: "sales".to_owned(),
                owner: Some("reporting".to_owned()),
            }),
            Dialect::DuckDb,
            "schema owners",
        ),
        (
            DdlOperation::DropSchema {
                schema: "sales".to_owned(),
                cascade: true,
            },
            Dialect::SqlServer,
            "CASCADE for dropping a schema",
        ),
    ];
    for (operation, dialect, what) in cases {
        assert_eq!(
            operation.statements(dialect),
            Err(DdlError::Unsupported { what, dialect }),
            "{operation:?}"
        );
    }
    let unnamed = DdlOperation::CreateSchema(SchemaSpec {
        name: String::new(),
        owner: None,
    });
    assert_eq!(
        unnamed.statements(Dialect::Postgres),
        Err(DdlError::MissingName("schema"))
    );
}

/// A verifier as `ScramVerifier::hash` writes one, fixed so the statements are the same every
/// run.
const VERIFIER: &str = "SCRAM-SHA-256$4096:c2FsdHNhbHRzYWx0c2FsdA==$\
    c3RvcmVkc3RvcmVkc3RvcmVkc3RvcmVkc3RvcmVkMTI=:c2VydmVyc2VydmVyc2VydmVyc2VydmVyc2VydmVyMTI=";

fn role_spec(attributes: &[RoleAttribute], member_of: &[&str]) -> RoleSpec {
    RoleSpec {
        attributes: attributes.iter().copied().collect(),
        connection_limit: None,
        valid_until: None,
        member_of: member_of.iter().map(|role| (*role).to_owned()).collect(),
    }
}

#[test]
fn creates_a_login_role_with_its_password_verifier_and_memberships() {
    let spec = RoleSpec {
        connection_limit: Some(5),
        valid_until: Some(" 2027-01-01 ".to_owned()),
        ..role_spec(
            &[RoleAttribute::Login, RoleAttribute::CreateDb],
            &["readers", "Report Writers"],
        )
    };
    let operation = DdlOperation::CreateRole {
        name: "alice".to_owned(),
        spec,
        password: Some(ScramVerifier::fixed(VERIFIER)),
    };
    assert_eq!(
        operation.statements(Dialect::Postgres).unwrap(),
        [format!(
            "CREATE ROLE alice WITH LOGIN CREATEDB NOINHERIT CONNECTION LIMIT 5 \
             VALID UNTIL '2027-01-01' IN ROLE \"Report Writers\", readers PASSWORD '{VERIFIER}'"
        )]
    );
}

#[test]
fn a_role_with_postgres_defaults_needs_no_options() {
    let operation = DdlOperation::CreateRole {
        name: "user".to_owned(),
        spec: role_spec(&[RoleAttribute::Inherit], &[]),
        password: None,
    };
    assert_eq!(
        operation.statements(Dialect::Postgres).unwrap(),
        ["CREATE ROLE \"user\""]
    );
}

#[test]
fn alters_only_what_changed_and_grants_or_revokes_memberships() {
    let before = RoleSpec {
        connection_limit: Some(5),
        valid_until: Some("2027-01-01 00:00:00+00".to_owned()),
        ..role_spec(
            &[RoleAttribute::Login, RoleAttribute::Inherit],
            &["readers", "auditors"],
        )
    };
    let after = RoleSpec {
        connection_limit: None,
        valid_until: Some(String::new()),
        ..role_spec(
            &[RoleAttribute::Login, RoleAttribute::CreateRole],
            &["readers", "writers"],
        )
    };
    let operation = DdlOperation::AlterRole {
        name: "alice".to_owned(),
        before,
        after,
        password: PasswordChange::Set(ScramVerifier::fixed(VERIFIER)),
    };
    assert_eq!(
        operation.statements(Dialect::Postgres).unwrap(),
        [
            format!(
                "ALTER ROLE alice WITH CREATEROLE NOINHERIT CONNECTION LIMIT -1 \
                 VALID UNTIL 'infinity' PASSWORD '{VERIFIER}'"
            ),
            "REVOKE auditors FROM alice".to_owned(),
            "GRANT writers TO alice".to_owned(),
        ]
    );
}

#[test]
fn alters_a_password_alone_or_removes_it() {
    let spec = role_spec(&[RoleAttribute::Login], &[]);
    let alter = |password| DdlOperation::AlterRole {
        name: "alice".to_owned(),
        before: spec.clone(),
        after: spec.clone(),
        password,
    };
    assert_eq!(
        alter(PasswordChange::Remove)
            .statements(Dialect::Postgres)
            .unwrap(),
        ["ALTER ROLE alice WITH PASSWORD NULL"]
    );
    assert_eq!(
        alter(PasswordChange::Keep).statements(Dialect::Postgres),
        Err(DdlError::NothingChanged)
    );
}

#[test]
fn renames_and_drops_roles_with_what_they_own() {
    let rename = DdlOperation::RenameRole {
        name: "alice".to_owned(),
        new_name: "Alice Smith".to_owned(),
    };
    assert_eq!(
        rename.statements(Dialect::Postgres).unwrap(),
        ["ALTER ROLE alice RENAME TO \"Alice Smith\""]
    );
    let drop = |owned| DdlOperation::DropRole {
        name: "alice".to_owned(),
        owned,
    };
    let cases = [
        (OwnedObjects::Keep, vec!["DROP ROLE alice"]),
        (
            OwnedObjects::ReassignTo("bob".to_owned()),
            vec![
                "REASSIGN OWNED BY alice TO bob",
                "DROP OWNED BY alice",
                "DROP ROLE alice",
            ],
        ),
        (
            OwnedObjects::Drop,
            vec!["DROP OWNED BY alice", "DROP ROLE alice"],
        ),
    ];
    for (owned, statements) in cases {
        assert_eq!(
            drop(owned).statements(Dialect::Postgres).unwrap(),
            statements
        );
    }
    assert_eq!(
        drop(OwnedObjects::ReassignTo(" ".to_owned())).statements(Dialect::Postgres),
        Err(DdlError::NoNewOwner)
    );
    assert_eq!(
        drop(OwnedObjects::ReassignTo("alice".to_owned())).statements(Dialect::Postgres),
        Err(DdlError::ReassignToItself)
    );
}

#[test]
fn roles_need_a_name_and_an_engine_whose_users_are_roles() {
    let create = |name: &str| DdlOperation::CreateRole {
        name: name.to_owned(),
        spec: role_spec(&[RoleAttribute::Inherit], &[]),
        password: None,
    };
    assert_eq!(
        create(" ").statements(Dialect::Postgres),
        Err(DdlError::MissingName("role"))
    );
    for dialect in Dialect::ALL {
        let supported = dialect.capabilities().users == UserModel::Roles;
        assert_eq!(
            create("alice").statements(dialect).is_ok(),
            supported,
            "{dialect:?}"
        );
    }
    assert_eq!(Dialect::Postgres.capabilities().users, UserModel::Roles);
}

/// A table numbered by the database, and how each dialect says so.
fn numbered(identity: Identity) -> TableSpec {
    TableSpec {
        name: ObjectName::new(None, "tickets"),
        columns: vec![
            ColumnSpec {
                identity: Some(identity),
                ..column("id", "integer", false, None)
            },
            column("title", "text", false, None),
        ],
        primary_key: strings(&["id"]),
        unique_keys: vec![],
        foreign_keys: vec![],
        checks: vec![],
    }
}

#[test]
fn writes_identity_columns() {
    use dbdesk_core::IdentityGeneration::{Always, ByDefault};
    let from_100_by_5 = Identity {
        generation: Always,
        start: 100,
        increment: 5,
    };
    let cases = [
        (
            Dialect::Postgres,
            Identity::counting(ByDefault),
            "id integer GENERATED BY DEFAULT AS IDENTITY NOT NULL",
        ),
        (
            Dialect::Postgres,
            from_100_by_5,
            "id integer GENERATED ALWAYS AS IDENTITY (START WITH 100 INCREMENT BY 5) NOT NULL",
        ),
        (
            Dialect::SqlServer,
            from_100_by_5,
            "id integer IDENTITY(100, 5) NOT NULL",
        ),
        (
            Dialect::Mysql,
            Identity::counting(ByDefault),
            "id integer AUTO_INCREMENT NOT NULL",
        ),
    ];
    for (dialect, identity, column) in cases {
        let sql = DdlOperation::CreateTable(numbered(identity))
            .statements(dialect)
            .unwrap();
        assert!(sql[0].contains(column), "{dialect:?}: {}", sql[0]);
        assert!(
            sql[0].contains("PRIMARY KEY (id)"),
            "{dialect:?}: {}",
            sql[0]
        );
    }
    let sqlite = DdlOperation::CreateTable(numbered(Identity::counting(ByDefault)))
        .statements(Dialect::Sqlite)
        .unwrap();
    assert_eq!(
        sqlite[0],
        "CREATE TABLE tickets (
    id integer PRIMARY KEY AUTOINCREMENT NOT NULL,
    title text NOT NULL
)"
    );
}

#[test]
fn refuses_identity_columns_where_the_dialect_has_none() {
    let identity = Identity::counting(dbdesk_core::IdentityGeneration::ByDefault);
    for dialect in [Dialect::DuckDb, Dialect::ClickHouse] {
        let refused = DdlOperation::CreateTable(numbered(identity)).statements(dialect);
        assert_eq!(
            refused,
            Err(DdlError::Unsupported {
                what: "identity columns",
                dialect
            })
        );
    }
    let mut keyless = numbered(identity);
    keyless.primary_key.clear();
    assert!(matches!(
        DdlOperation::CreateTable(keyless).statements(Dialect::Sqlite),
        Err(DdlError::Unsupported { .. })
    ));
}

/// `shop.orders` as a database describes it: a named key, a default, and a column that goes.
fn existing_orders(dialect: Dialect) -> ExistingTable {
    let (integer, text) = match dialect {
        Dialect::ClickHouse => ("Int32", "String"),
        _ => ("integer", "varchar(20)"),
    };
    let mut default_names = BTreeMap::new();
    if dialect == Dialect::SqlServer {
        default_names.insert("status".to_owned(), "DF_orders_status".to_owned());
    }
    ExistingTable {
        spec: TableSpec {
            name: ObjectName::new(Some("shop"), "orders"),
            columns: vec![
                column("id", integer, false, None),
                column("status", text, false, Some("'new'")),
                column("note", text, true, None),
                column("legacy", integer, true, None),
            ],
            primary_key: strings(&["id"]),
            unique_keys: vec![],
            foreign_keys: vec![],
            checks: vec![],
        },
        primary_key_name: Some("orders_pkey".to_owned()),
        default_names,
    }
}

/// Renames `status` to `state`, widening it and changing its default; makes `note` required;
/// drops `legacy` and adds `shipped_on`.
fn changed_columns(dialect: Dialect) -> TableAlteration {
    let table = existing_orders(dialect);
    let mut after = table.spec.clone();
    let wide = match dialect {
        Dialect::ClickHouse => "LowCardinality(String)",
        _ => "varchar(40)",
    };
    after.columns = vec![
        after.columns[0].clone(),
        column("state", wide, false, Some("'open'")),
        column("note", &after.columns[2].data_type, false, None),
        column("shipped_on", "date", true, None),
    ];
    TableAlteration {
        table,
        after,
        renamed: BTreeMap::from([("state".to_owned(), "status".to_owned())]),
        conversions: BTreeMap::new(),
    }
}

#[test]
fn alters_columns_in_each_dialect() {
    let cases = [
        (
            Dialect::Postgres,
            vec![
                "ALTER TABLE shop.orders DROP COLUMN legacy",
                "ALTER TABLE shop.orders RENAME COLUMN status TO state",
                "ALTER TABLE shop.orders ADD COLUMN shipped_on date",
                "ALTER TABLE shop.orders ALTER COLUMN state TYPE varchar(40)",
                "ALTER TABLE shop.orders ALTER COLUMN state SET DEFAULT 'open'",
                "ALTER TABLE shop.orders ALTER COLUMN note SET NOT NULL",
            ],
        ),
        (
            Dialect::Mysql,
            vec![
                "ALTER TABLE shop.orders DROP COLUMN legacy",
                "ALTER TABLE shop.orders RENAME COLUMN status TO state",
                "ALTER TABLE shop.orders ADD COLUMN shipped_on date",
                "ALTER TABLE shop.orders MODIFY COLUMN state varchar(40) DEFAULT 'open' NOT NULL",
                "ALTER TABLE shop.orders MODIFY COLUMN note varchar(20) NOT NULL",
            ],
        ),
        (
            Dialect::SqlServer,
            vec![
                "ALTER TABLE shop.orders DROP COLUMN legacy",
                "EXEC sp_rename N'shop.orders.status', N'state', 'COLUMN'",
                "ALTER TABLE shop.orders ADD shipped_on date NULL",
                "ALTER TABLE shop.orders DROP CONSTRAINT [DF_orders_status]",
                "ALTER TABLE shop.orders ALTER COLUMN state varchar(40) NOT NULL",
                "ALTER TABLE shop.orders ADD CONSTRAINT [DF_orders_status] DEFAULT 'open' FOR state",
                "ALTER TABLE shop.orders ALTER COLUMN note varchar(20) NOT NULL",
            ],
        ),
        (
            Dialect::DuckDb,
            vec![
                "ALTER TABLE shop.orders DROP COLUMN legacy",
                "ALTER TABLE shop.orders RENAME COLUMN status TO state",
                "ALTER TABLE shop.orders ADD COLUMN shipped_on date",
                "ALTER TABLE shop.orders ALTER COLUMN state TYPE varchar(40)",
                "ALTER TABLE shop.orders ALTER COLUMN state SET DEFAULT 'open'",
                "ALTER TABLE shop.orders ALTER COLUMN note SET NOT NULL",
            ],
        ),
        (
            Dialect::ClickHouse,
            vec![
                "ALTER TABLE shop.orders DROP COLUMN legacy",
                "ALTER TABLE shop.orders RENAME COLUMN status TO state",
                "ALTER TABLE shop.orders ADD COLUMN shipped_on Nullable(date)",
                "ALTER TABLE shop.orders MODIFY COLUMN state LowCardinality(String) DEFAULT 'open'",
                "ALTER TABLE shop.orders MODIFY COLUMN note String",
            ],
        ),
    ];
    for (dialect, expected) in cases {
        let statements =
            DdlOperation::AlterTable(Box::new(changed_columns(dialect))).statements(dialect);
        assert_eq!(statements, Ok(strings(&expected)), "{dialect:?}");
    }
}

#[test]
fn converts_a_retyped_column_with_using_and_drops_a_default_before_retyping() {
    let mut alteration = changed_columns(Dialect::Postgres);
    alteration.after.columns[1].default = None;
    alteration.after.columns[1].data_type = "integer".to_owned();
    alteration.conversions.insert(
        "state".to_owned(),
        "CASE status WHEN 'new' THEN 0 ELSE 1 END".to_owned(),
    );
    let statements = DdlOperation::AlterTable(Box::new(alteration))
        .statements(Dialect::Postgres)
        .unwrap();
    assert_eq!(
        statements[3..5],
        [
            "ALTER TABLE shop.orders ALTER COLUMN state DROP DEFAULT",
            "ALTER TABLE shop.orders ALTER COLUMN state TYPE integer \
             USING CASE status WHEN 'new' THEN 0 ELSE 1 END",
        ]
    );
}

#[test]
fn a_renamed_column_is_never_dropped_and_added() {
    let table = existing_orders(Dialect::Postgres);
    let mut after = table.spec.clone();
    after.columns[3].name = "archived".to_owned();
    let renamed = TableAlteration {
        table: table.clone(),
        after: after.clone(),
        renamed: BTreeMap::from([("archived".to_owned(), "legacy".to_owned())]),
        conversions: BTreeMap::new(),
    };
    assert_eq!(
        DdlOperation::AlterTable(Box::new(renamed)).statements(Dialect::Postgres),
        Ok(strings(&[
            "ALTER TABLE shop.orders RENAME COLUMN legacy TO archived"
        ]))
    );
    // Without the rename, the same names mean a new column and a dropped one.
    let replaced = TableAlteration {
        table,
        after,
        renamed: BTreeMap::new(),
        conversions: BTreeMap::new(),
    };
    assert_eq!(
        DdlOperation::AlterTable(Box::new(replaced)).statements(Dialect::Postgres),
        Ok(strings(&[
            "ALTER TABLE shop.orders DROP COLUMN legacy",
            "ALTER TABLE shop.orders ADD COLUMN archived integer",
        ]))
    );
}

#[test]
fn swapping_two_columns_names_is_two_renames() {
    let table = existing_orders(Dialect::Mysql);
    let mut after = table.spec.clone();
    after.columns.truncate(3);
    after.columns.push(column("legacy", "integer", true, None));
    after.columns[2].name = "remark".to_owned();
    let alteration = TableAlteration {
        table,
        after,
        renamed: BTreeMap::from([("remark".to_owned(), "note".to_owned())]),
        conversions: BTreeMap::new(),
    };
    assert_eq!(
        DdlOperation::AlterTable(Box::new(alteration)).statements(Dialect::Mysql),
        Ok(strings(&[
            "ALTER TABLE shop.orders RENAME COLUMN note TO remark"
        ]))
    );
}

/// `shop.orders` with every kind of constraint, named as PostgreSQL names them.
fn constrained_orders() -> ExistingTable {
    let mut table = existing_orders(Dialect::Postgres);
    table.spec.columns.truncate(2);
    table.spec.columns.extend([
        column("code", "varchar(20)", false, None),
        column("customer", "integer", true, None),
        column("total", "integer", false, None),
        column("region", "integer", false, Some("1")),
    ]);
    table.spec.unique_keys = vec![UniqueKey {
        name: Some("orders_code_key".to_owned()),
        columns: strings(&["code"]),
    }];
    table.spec.foreign_keys = vec![ForeignKey {
        name: Some("orders_customer_fkey".to_owned()),
        columns: strings(&["customer"]),
        target: ObjectName::new(Some("shop"), "customers"),
        target_columns: strings(&["id"]),
        on_delete: ReferentialAction::NoAction,
        on_update: ReferentialAction::NoAction,
    }];
    table.spec.checks = vec![Check {
        name: Some("orders_total_check".to_owned()),
        expression: "total >= 0".to_owned(),
    }];
    table
}

/// Keeps the unique key (its column renamed), makes the foreign key cascade, replaces the
/// check, adds `region` to the primary key, and renames the table.
fn changed_constraints() -> TableAlteration {
    let table = constrained_orders();
    let mut after = table.spec.clone();
    after.name.name = "purchases".to_owned();
    after.columns[2].name = "reference".to_owned();
    after.unique_keys[0].columns = strings(&["reference"]);
    after.foreign_keys[0].on_delete = ReferentialAction::Cascade;
    after.checks = vec![Check {
        name: None,
        expression: "total > 0".to_owned(),
    }];
    after.primary_key = strings(&["id", "region"]);
    TableAlteration {
        table,
        after,
        renamed: BTreeMap::from([("reference".to_owned(), "code".to_owned())]),
        conversions: BTreeMap::new(),
    }
}

#[test]
fn alters_constraints_in_each_dialect() {
    let cases = [
        (
            Dialect::Postgres,
            vec![
                "ALTER TABLE shop.orders DROP CONSTRAINT orders_customer_fkey",
                "ALTER TABLE shop.orders DROP CONSTRAINT orders_total_check",
                "ALTER TABLE shop.orders DROP CONSTRAINT orders_pkey",
                "ALTER TABLE shop.orders RENAME COLUMN code TO reference",
                "ALTER TABLE shop.orders ADD CONSTRAINT orders_pkey PRIMARY KEY (id, region)",
                "ALTER TABLE shop.orders ADD CHECK (total > 0)",
                "ALTER TABLE shop.orders ADD CONSTRAINT orders_customer_fkey FOREIGN KEY \
                 (customer) REFERENCES shop.customers (id) ON DELETE CASCADE",
                "ALTER TABLE shop.orders RENAME TO purchases",
            ],
        ),
        (
            Dialect::Mysql,
            vec![
                "ALTER TABLE shop.orders DROP FOREIGN KEY orders_customer_fkey",
                "ALTER TABLE shop.orders DROP CONSTRAINT orders_total_check",
                "ALTER TABLE shop.orders DROP PRIMARY KEY",
                "ALTER TABLE shop.orders RENAME COLUMN code TO reference",
                "ALTER TABLE shop.orders ADD PRIMARY KEY (id, region)",
                "ALTER TABLE shop.orders ADD CHECK (total > 0)",
                "ALTER TABLE shop.orders ADD CONSTRAINT orders_customer_fkey FOREIGN KEY \
                 (customer) REFERENCES shop.customers (id) ON DELETE CASCADE",
                "RENAME TABLE shop.orders TO shop.purchases",
            ],
        ),
        (
            Dialect::SqlServer,
            vec![
                "ALTER TABLE shop.orders DROP CONSTRAINT orders_customer_fkey",
                "ALTER TABLE shop.orders DROP CONSTRAINT orders_total_check",
                "ALTER TABLE shop.orders DROP CONSTRAINT orders_pkey",
                "EXEC sp_rename N'shop.orders.code', N'reference', 'COLUMN'",
                "ALTER TABLE shop.orders ADD CONSTRAINT orders_pkey PRIMARY KEY (id, region)",
                "ALTER TABLE shop.orders ADD CHECK (total > 0)",
                "ALTER TABLE shop.orders ADD CONSTRAINT orders_customer_fkey FOREIGN KEY \
                 (customer) REFERENCES shop.customers (id) ON DELETE CASCADE",
                "EXEC sp_rename N'shop.orders', N'purchases'",
            ],
        ),
    ];
    for (dialect, expected) in cases {
        let statements =
            DdlOperation::AlterTable(Box::new(changed_constraints())).statements(dialect);
        assert_eq!(statements, Ok(strings(&expected)), "{dialect:?}");
    }
}

#[test]
fn drops_a_unique_key_by_its_index_on_mysql_and_adds_one_everywhere() {
    let table = constrained_orders();
    let mut after = table.spec.clone();
    after.unique_keys = vec![UniqueKey {
        name: None,
        columns: strings(&["code", "region"]),
    }];
    let alteration = TableAlteration {
        table,
        after,
        renamed: BTreeMap::new(),
        conversions: BTreeMap::new(),
    };
    assert_eq!(
        DdlOperation::AlterTable(Box::new(alteration.clone())).statements(Dialect::Mysql),
        Ok(strings(&[
            "ALTER TABLE shop.orders DROP INDEX orders_code_key",
            "ALTER TABLE shop.orders ADD UNIQUE (code, region)",
        ]))
    );
    assert_eq!(
        DdlOperation::AlterTable(Box::new(alteration)).statements(Dialect::Postgres),
        Ok(strings(&[
            "ALTER TABLE shop.orders DROP CONSTRAINT orders_code_key",
            "ALTER TABLE shop.orders ADD UNIQUE (code, region)",
        ]))
    );
}

#[test]
fn keeps_a_foreign_key_to_the_table_itself_through_a_rename() {
    let mut table = constrained_orders();
    table.spec.foreign_keys[0].target = table.spec.name.clone();
    let mut after = table.spec.clone();
    after.name.name = "purchases".to_owned();
    after.foreign_keys[0].target = after.name.clone();
    let alteration = TableAlteration {
        table,
        after,
        renamed: BTreeMap::new(),
        conversions: BTreeMap::new(),
    };
    assert_eq!(
        DdlOperation::AlterTable(Box::new(alteration)).statements(Dialect::Postgres),
        Ok(strings(&["ALTER TABLE shop.orders RENAME TO purchases"]))
    );
}

#[test]
fn says_when_nothing_changed() {
    let table = constrained_orders();
    let alteration = TableAlteration {
        after: table.spec.clone(),
        table,
        renamed: BTreeMap::new(),
        conversions: BTreeMap::new(),
    };
    assert_eq!(
        DdlOperation::AlterTable(Box::new(alteration)).statements(Dialect::Postgres),
        Err(DdlError::NothingChanged)
    );
}

#[test]
fn refuses_what_a_dialect_cant_change_in_place() {
    let cant = |what, dialect| Err(DdlError::CantAlter { what, dialect });
    let new_check = |dialect| {
        let table = existing_orders(dialect);
        let mut after = table.spec.clone();
        after.checks.push(Check {
            name: None,
            expression: "id > 0".to_owned(),
        });
        TableAlteration {
            table,
            after,
            renamed: BTreeMap::new(),
            conversions: BTreeMap::new(),
        }
    };
    assert_eq!(
        DdlOperation::AlterTable(Box::new(new_check(Dialect::DuckDb))).statements(Dialect::DuckDb),
        cant("add keys or checks to an existing table", Dialect::DuckDb)
    );
    assert_eq!(
        DdlOperation::AlterTable(Box::new(changed_columns(Dialect::Sqlite)))
            .statements(Dialect::Sqlite),
        cant("change an existing column's type", Dialect::Sqlite)
    );
    let mut new_key = new_check(Dialect::ClickHouse);
    new_key.after.checks.clear();
    new_key.after.primary_key = strings(&["id", "status"]);
    assert_eq!(
        DdlOperation::AlterTable(Box::new(new_key)).statements(Dialect::ClickHouse),
        cant(
            "change an existing table's primary key",
            Dialect::ClickHouse
        )
    );
    let mut moved = new_check(Dialect::Postgres);
    moved.after.checks.clear();
    moved.after.name.schema = Some("archive".to_owned());
    assert_eq!(
        DdlOperation::AlterTable(Box::new(moved)).statements(Dialect::Postgres),
        cant("move a table to another schema", Dialect::Postgres)
    );
    let mut renumbered = new_check(Dialect::Postgres);
    renumbered.after.checks.clear();
    renumbered.after.columns[0].identity = Some(Identity::counting(
        dbdesk_core::IdentityGeneration::ByDefault,
    ));
    assert!(matches!(
        DdlOperation::AlterTable(Box::new(renumbered)).statements(Dialect::Postgres),
        Err(DdlError::CantAlter { .. })
    ));
}

#[test]
fn sqlite_changes_simple_columns_in_place_and_refuses_the_rest() {
    let table = existing_orders(Dialect::Sqlite);
    let mut after = table.spec.clone();
    after.columns[1].name = "state".to_owned();
    after.columns.remove(3);
    after.columns.push(column("shipped_on", "text", true, None));
    let mut alteration = TableAlteration {
        table,
        after,
        renamed: BTreeMap::from([("state".to_owned(), "status".to_owned())]),
        conversions: BTreeMap::new(),
    };
    assert_eq!(
        DdlOperation::AlterTable(Box::new(alteration.clone())).statements(Dialect::Sqlite),
        Ok(strings(&[
            "ALTER TABLE shop.orders DROP COLUMN legacy",
            "ALTER TABLE shop.orders RENAME COLUMN status TO state",
            "ALTER TABLE shop.orders ADD COLUMN shipped_on text",
        ]))
    );
    alteration.after.columns[3].nullable = false;
    assert!(matches!(
        DdlOperation::AlterTable(Box::new(alteration)).statements(Dialect::Sqlite),
        Err(DdlError::CantAlter { .. })
    ));
}

#[test]
fn duckdb_adds_a_required_column_then_requires_it() {
    let table = existing_orders(Dialect::DuckDb);
    let mut after = table.spec.clone();
    after
        .columns
        .push(column("priority", "integer", false, Some("0")));
    let alteration = TableAlteration {
        table,
        after,
        renamed: BTreeMap::new(),
        conversions: BTreeMap::new(),
    };
    assert_eq!(
        DdlOperation::AlterTable(Box::new(alteration)).statements(Dialect::DuckDb),
        Ok(strings(&[
            "ALTER TABLE shop.orders ADD COLUMN priority integer DEFAULT 0",
            "ALTER TABLE shop.orders ALTER COLUMN priority SET NOT NULL",
        ]))
    );
}

#[test]
fn reads_an_existing_table_from_its_catalog_description() {
    use dbdesk_core::{
        CheckConstraint, ColumnDescription, ForeignKeyConstraint, TableConstraints,
        TableDescription, TableName,
    };
    let description = TableDescription {
        schema: "shop".to_owned(),
        name: "orders".to_owned(),
        columns: vec![
            ColumnDescription {
                name: "id".to_owned(),
                data_type: "Int32".to_owned(),
                // As SQLite reports an INTEGER PRIMARY KEY.
                nullable: true,
                default: None,
                identity: None,
            },
            ColumnDescription {
                name: "note".to_owned(),
                data_type: "Nullable(String)".to_owned(),
                nullable: true,
                default: Some("'none'".to_owned()),
                identity: None,
            },
        ],
        primary_key: strings(&["id"]),
        foreign_keys: vec![],
        referenced_by: vec![],
    };
    let constraints = TableConstraints {
        primary_key_name: Some("orders_pkey".to_owned()),
        foreign_keys: vec![ForeignKeyConstraint {
            name: Some("orders_note_fkey".to_owned()),
            columns: strings(&["note"]),
            referenced: TableName {
                schema: "shop".to_owned(),
                name: "notes".to_owned(),
            },
            referenced_columns: strings(&["text"]),
            on_delete: ReferentialAction::Cascade,
            on_update: ReferentialAction::NoAction,
        }],
        checks: vec![CheckConstraint {
            name: Some("positive".to_owned()),
            expression: "id > 0".to_owned(),
        }],
        ..TableConstraints::default()
    };
    let clickhouse = ExistingTable::from_catalog(
        description.clone(),
        constraints.clone(),
        Dialect::ClickHouse,
    );
    assert_eq!(clickhouse.spec.columns[1].data_type, "String");
    assert!(!clickhouse.spec.columns[0].nullable, "a key column");
    assert_eq!(
        clickhouse.spec.name,
        ObjectName::new(Some("shop"), "orders")
    );
    assert_eq!(clickhouse.primary_key_name.as_deref(), Some("orders_pkey"));
    assert_eq!(
        clickhouse.spec.foreign_keys[0].target,
        ObjectName::new(Some("shop"), "notes")
    );
    assert_eq!(clickhouse.spec.checks[0].name.as_deref(), Some("positive"));
    let postgres = ExistingTable::from_catalog(description, constraints, Dialect::Postgres);
    assert_eq!(postgres.spec.columns[1].data_type, "Nullable(String)");
}
