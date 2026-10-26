//! Golden tests: what each dialect's DDL looks like, written out in full.

use super::*;

fn column(name: &str, data_type: &str, nullable: bool, default: Option<&str>) -> ColumnSpec {
    ColumnSpec {
        name: name.to_owned(),
        data_type: data_type.to_owned(),
        nullable,
        default: default.map(str::to_owned),
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
