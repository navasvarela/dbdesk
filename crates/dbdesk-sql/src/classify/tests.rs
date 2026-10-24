//! The rules, a few cases each. The per-dialect corpus in `tests/classify/` holds many more.

use std::collections::BTreeSet;

use super::*;

use StatementKind::{Admin, Read, Schema, Write};

#[test]
fn refuses_to_guess() {
    for dialect in Dialect::ALL {
        assert_eq!(classify("", dialect), Err(Unclassified::Empty));
        assert_eq!(
            classify("  -- nothing\n", dialect),
            Err(Unclassified::Empty)
        );
        assert_eq!(
            classify("SELECT 1; DELETE FROM t", dialect),
            Err(Unclassified::Several(2))
        );
        assert!(matches!(
            classify("SELEC * FROM t", dialect),
            Err(Unclassified::Unparseable(_))
        ));
    }
}

#[test]
fn the_most_demanding_part_decides() {
    let cases = [
        (
            "WITH gone AS (DELETE FROM t RETURNING *) SELECT * FROM gone",
            Write,
        ),
        ("EXPLAIN ANALYZE CREATE TABLE t AS SELECT 1", Schema),
        ("SELECT pg_terminate_backend(42)", Admin),
        (
            "SELECT * FROM t WHERE x = set_config('a', 'b', true)",
            Admin,
        ),
        ("SELECT pg_catalog.PG_CANCEL_BACKEND(42)", Admin),
        ("SELECT \"set_config\"('a', 'b', true)", Admin),
        ("SELECT \"SET_CONFIG\"('a', 'b', true)", Read),
        ("CALL tidy_up()", Admin),
    ];
    for (sql, expected) in cases {
        assert_eq!(classify(sql, Dialect::Postgres), Ok(expected), "{sql}");
    }
}

fn schemas(sql: &str, dialect: Dialect) -> NamedSchemas {
    analyze(sql, dialect)
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .schemas
}

fn known(qualified: &[(&str, bool)], unqualified: bool) -> NamedSchemas {
    NamedSchemas::Known {
        qualified: qualified
            .iter()
            .map(|(value, quoted)| NamePart {
                value: (*value).to_owned(),
                quoted: *quoted,
            })
            .collect::<BTreeSet<_>>(),
        unqualified,
    }
}

#[test]
fn finds_the_schemas_a_statement_names() {
    let cases = [
        ("SELECT 1", known(&[], false)),
        ("SELECT count(*) FROM orders", known(&[], true)),
        (
            "SELECT * FROM shop.orders o JOIN \"Billing\".invoices i ON o.id = i.order_id",
            known(&[("shop", false), ("Billing", true)], false),
        ),
        (
            "SELECT * FROM shop.orders WHERE id IN (SELECT id FROM audit.log)",
            known(&[("shop", false), ("audit", false)], false),
        ),
        ("SELECT other.f(1)", known(&[("other", false)], false)),
        (
            "SELECT * FROM other.series(1, 3)",
            known(&[("other", false)], false),
        ),
        (
            "WITH recent AS (SELECT * FROM shop.orders) SELECT * FROM recent",
            known(&[("shop", false)], false),
        ),
        (
            "WITH gone AS (DELETE FROM shop.orders RETURNING *) SELECT * FROM gone",
            known(&[("shop", false)], false),
        ),
        (
            "INSERT INTO archive.orders SELECT * FROM shop.orders",
            known(&[("archive", false), ("shop", false)], false),
        ),
        (
            "UPDATE shop.orders SET x = 1",
            known(&[("shop", false)], false),
        ),
        (
            "CREATE TABLE shop.t (id int)",
            known(&[("shop", false)], false),
        ),
        ("ALTER TABLE t ADD COLUMN c int", known(&[], true)),
        (
            "DROP TABLE shop.t, archive.t",
            known(&[("shop", false), ("archive", false)], false),
        ),
        ("DROP SCHEMA shop", known(&[("shop", false)], false)),
        (
            "COMMENT ON COLUMN shop.t.c IS 'x'",
            known(&[("shop", false)], false),
        ),
        ("TRUNCATE shop.t", known(&[("shop", false)], false)),
        ("CREATE DATABASE other", NamedSchemas::Unknown),
        ("GRANT SELECT ON shop.t TO someone", NamedSchemas::Unknown),
        ("CALL shop.tidy_up()", NamedSchemas::Unknown),
    ];
    for (sql, expected) in cases {
        assert_eq!(schemas(sql, Dialect::Postgres), expected, "{sql}");
    }
}

#[test]
fn reads_each_dialects_qualified_names() {
    assert_eq!(
        schemas("SELECT * FROM `shop`.orders", Dialect::Mysql),
        known(&[("shop", true)], false)
    );
    assert_eq!(
        schemas("SELECT * FROM erp.[dbo].orders", Dialect::SqlServer),
        known(&[("dbo", true)], false)
    );
    assert_eq!(
        schemas("SELECT * FROM main.orders", Dialect::Sqlite),
        known(&[("main", false)], false)
    );
}
