//! What a statement does, read from its parse tree: whether it only reads, or changes data,
//! the schema or the server. The MCP server refuses what a project doesn't allow before it
//! reaches the database (see `docs/mcp-server.md`).
//!
//! Unlike the rest of this crate, this is strict: a statement the parser can't read is not
//! guessed at, it is [`Unclassified`], because what it does can't be known.

use std::ops::ControlFlow;

use sqlparser::ast::{Query, SetExpr, Statement, Visit, Visitor};
use sqlparser::parser::Parser;

use crate::dialect::Dialect;

/// What a statement may change, from least to most. A statement holding others (a `WITH`
/// clause with a `DELETE`, an `EXPLAIN ANALYZE`) is the most demanding of them all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StatementKind {
    /// Browsing the catalog, `SELECT`, `EXPLAIN`, `SHOW`.
    Read,
    /// `INSERT`, `UPDATE`, `DELETE`, `MERGE`, `TRUNCATE`, `CALL`, and reads that lock rows.
    Write,
    /// `CREATE`, `ALTER`, `DROP`, `COMMENT ON`, `SELECT … INTO` a new table.
    Schema,
    /// `BEGIN`, `COMMIT`, `ROLLBACK`, savepoints: whoever runs the statement owns the
    /// transaction, so an MCP client may never send these.
    Transaction,
    /// Everything else: grants, roles, `SET`, `COPY`, `VACUUM`, `PRAGMA`, `ATTACH`, and any
    /// statement this classifier doesn't know.
    Admin,
}

impl StatementKind {
    pub fn description(self) -> &'static str {
        match self {
            Self::Read => "reads",
            Self::Write => "changes data",
            Self::Schema => "changes the schema",
            Self::Transaction => "controls the transaction",
            Self::Admin => "administers the database",
        }
    }
}

/// Why a statement has no kind.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unclassified {
    #[error("there is no statement")]
    Empty,
    #[error("send one statement at a time; this text holds {0}")]
    Several(usize),
    #[error("the statement can't be parsed, so what it does is unknown: {0}")]
    Unparseable(String),
}

/// Classifies `sql`, which must hold exactly one statement.
///
/// # Errors
///
/// [`Unclassified`] when the text holds no statement, several, or one the dialect's parser
/// can't read.
pub fn classify(sql: &str, dialect: Dialect) -> Result<StatementKind, Unclassified> {
    let statements = Parser::parse_sql(dialect.sqlparser_dialect(), sql).map_err(|error| {
        let message = error.to_string();
        let message = message
            .strip_prefix("sql parser error: ")
            .unwrap_or(&message);
        Unclassified::Unparseable(message.to_owned())
    })?;
    let statement = match statements.as_slice() {
        [] => return Err(Unclassified::Empty),
        [statement] => statement,
        several => return Err(Unclassified::Several(several.len())),
    };
    let mut classifier = Classifier {
        kind: StatementKind::Read,
    };
    // The visitor never breaks, so the flow it returns says nothing.
    let _ = statement.visit(&mut classifier);
    Ok(classifier.kind)
}

/// Walks every statement and query in the tree, nested ones included, keeping the most
/// demanding kind seen. Nesting is how writes hide: `WITH d AS (DELETE …) SELECT …` is a
/// query whose `WITH` holds a `DELETE` statement.
struct Classifier {
    kind: StatementKind,
}

impl Classifier {
    fn raise(&mut self, kind: StatementKind) {
        self.kind = self.kind.max(kind);
    }
}

impl Visitor for Classifier {
    type Break = ();

    fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<()> {
        self.raise(statement_kind(statement));
        ControlFlow::Continue(())
    }

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        // `FOR UPDATE` and its kin take row locks, which only writers need.
        if !query.locks.is_empty() {
            self.raise(StatementKind::Write);
        }
        if selects_into(&query.body) {
            self.raise(StatementKind::Schema);
        }
        ControlFlow::Continue(())
    }
}

/// `SELECT … INTO new_table` (PostgreSQL) creates a table; MySQL's `INTO @variable` and
/// `INTO OUTFILE` set variables or write files. None of them only reads.
fn selects_into(body: &SetExpr) -> bool {
    match body {
        SetExpr::Select(select) => select.into.is_some(),
        SetExpr::SetOperation { left, right, .. } => selects_into(left) || selects_into(right),
        _ => false,
    }
}

/// The kind of one statement on its own; what it holds is visited separately.
fn statement_kind(statement: &Statement) -> StatementKind {
    use StatementKind::{Admin, Read, Schema, Transaction, Write};
    match statement {
        // EXPLAIN's own kind is reading; the statement it explains is visited too, which is
        // what makes `EXPLAIN ANALYZE DELETE …` (which runs the DELETE) a write.
        Statement::Query(_)
        | Statement::Explain { .. }
        | Statement::ExplainTable { .. }
        | Statement::ShowFunctions { .. }
        | Statement::ShowVariable { .. }
        | Statement::ShowStatus { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowCreate { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowCatalogs { .. }
        | Statement::ShowDatabases { .. }
        | Statement::ShowSchemas { .. }
        | Statement::ShowCharset(_)
        | Statement::ShowObjects(_)
        | Statement::ShowTables { .. }
        | Statement::ShowViews { .. }
        | Statement::ShowCollation { .. } => Read,

        Statement::Insert(_)
        | Statement::Update(_)
        | Statement::Delete(_)
        | Statement::Merge(_)
        | Statement::Truncate(_)
        | Statement::Call(_)
        | Statement::LoadData { .. }
        | Statement::Lock(_)
        | Statement::LockTables { .. }
        | Statement::UnlockTables => Write,

        Statement::CreateView(_)
        | Statement::CreateTable(_)
        | Statement::CreateVirtualTable { .. }
        | Statement::CreateIndex(_)
        | Statement::CreateSchema { .. }
        | Statement::CreateFunction(_)
        | Statement::CreateTrigger(_)
        | Statement::CreateProcedure { .. }
        | Statement::CreateSequence { .. }
        | Statement::CreateDomain(_)
        | Statement::CreateType { .. }
        | Statement::CreatePolicy(_)
        | Statement::CreateExtension(_)
        | Statement::CreateCollation(_)
        | Statement::CreateOperator(_)
        | Statement::CreateOperatorFamily(_)
        | Statement::CreateOperatorClass(_)
        | Statement::CreateTextSearch(_)
        | Statement::AlterTable(_)
        | Statement::AlterSchema(_)
        | Statement::AlterIndex { .. }
        | Statement::AlterView { .. }
        | Statement::AlterFunction(_)
        | Statement::AlterType(_)
        | Statement::AlterCollation(_)
        | Statement::AlterOperator(_)
        | Statement::AlterOperatorFamily(_)
        | Statement::AlterOperatorClass(_)
        | Statement::AlterTextSearch(_)
        | Statement::AlterPolicy(_)
        | Statement::Drop { .. }
        | Statement::DropFunction(_)
        | Statement::DropDomain(_)
        | Statement::DropProcedure { .. }
        | Statement::DropPolicy(_)
        | Statement::DropExtension(_)
        | Statement::DropOperator(_)
        | Statement::DropOperatorFamily(_)
        | Statement::DropOperatorClass(_)
        | Statement::DropTrigger(_)
        | Statement::Comment { .. }
        | Statement::RenameTable(_) => Schema,

        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. }
        | Statement::Savepoint { .. }
        | Statement::ReleaseSavepoint { .. } => Transaction,

        // Deliberately not listed one by one: a statement added to sqlparser later, or one
        // overlooked here, must not slip through as anything less.
        _ => Admin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use StatementKind::{Admin, Read, Schema, Transaction, Write};

    fn check(dialect: Dialect, cases: &[(&str, StatementKind)]) {
        for (sql, expected) in cases {
            assert_eq!(classify(sql, dialect), Ok(*expected), "{dialect:?}: {sql}");
        }
    }

    /// Statements every dialect reads the same way.
    const SHARED: &[(&str, StatementKind)] = &[
        ("SELECT 1", Read),
        (
            "select * from orders where id = 3 order by id limit 5",
            Read,
        ),
        ("SELECT a FROM t UNION ALL SELECT b FROM u", Read),
        (
            "WITH recent AS (SELECT * FROM orders) SELECT count(*) FROM recent",
            Read,
        ),
        ("SELECT * FROM t WHERE id IN (SELECT id FROM u)", Read),
        ("VALUES (1, 2)", Read),
        ("EXPLAIN SELECT * FROM t", Read),
        ("INSERT INTO t (a) VALUES (1)", Write),
        ("UPDATE t SET a = 1 WHERE id = 2", Write),
        ("DELETE FROM t", Write),
        ("INSERT INTO t SELECT * FROM u", Write),
        ("CREATE TABLE t (id INTEGER PRIMARY KEY)", Schema),
        ("CREATE INDEX i ON t (a)", Schema),
        ("CREATE VIEW v AS SELECT 1", Schema),
        ("ALTER TABLE t ADD COLUMN b TEXT", Schema),
        ("DROP TABLE t", Schema),
        ("COMMIT", Transaction),
        ("ROLLBACK", Transaction),
        ("SAVEPOINT a", Transaction),
    ];

    #[test]
    fn classifies_postgres_statements() {
        check(Dialect::Postgres, SHARED);
        check(
            Dialect::Postgres,
            &[
                ("SHOW search_path", Read),
                (
                    "WITH gone AS (DELETE FROM t WHERE id = 1 RETURNING *) SELECT * FROM gone",
                    Write,
                ),
                (
                    "WITH a AS (SELECT 1), b AS (UPDATE t SET x = 1 RETURNING x) SELECT * FROM b",
                    Write,
                ),
                (
                    "WITH moved AS (INSERT INTO log SELECT * FROM t RETURNING *) SELECT 1",
                    Write,
                ),
                ("SELECT * INTO archive FROM orders", Schema),
                ("SELECT * FROM t FOR UPDATE", Write),
                ("SELECT * FROM t FOR SHARE", Write),
                ("EXPLAIN ANALYZE DELETE FROM t", Write),
                ("EXPLAIN DELETE FROM t", Write),
                ("TRUNCATE t", Write),
                ("CALL do_things()", Write),
                (
                    "MERGE INTO t USING u ON t.id = u.id WHEN MATCHED THEN DELETE",
                    Write,
                ),
                ("COMMENT ON TABLE t IS 'x'", Schema),
                ("BEGIN READ WRITE", Transaction),
                ("START TRANSACTION", Transaction),
                ("SET default_transaction_read_only = off", Admin),
                ("SET TRANSACTION READ WRITE", Admin),
                ("COPY t TO '/tmp/out.csv'", Admin),
                ("COPY t FROM '/tmp/in.csv'", Admin),
                ("GRANT SELECT ON t TO someone", Admin),
                ("VACUUM", Admin),
                ("CREATE DATABASE other", Admin),
                ("CREATE ROLE intruder", Admin),
            ],
        );
    }

    #[test]
    fn classifies_mysql_statements() {
        check(Dialect::Mysql, SHARED);
        check(
            Dialect::Mysql,
            &[
                ("SHOW TABLES", Read),
                ("SHOW CREATE TABLE t", Read),
                ("SHOW COLUMNS FROM t", Read),
                ("DESCRIBE t", Read),
                ("SELECT `order` FROM `t`", Read),
                ("REPLACE INTO t (a) VALUES (1)", Write),
                ("SELECT * FROM t FOR UPDATE", Write),
                ("SELECT a INTO @x FROM t", Schema),
                ("START TRANSACTION READ WRITE", Transaction),
                ("LOCK TABLES t WRITE", Write),
                ("SET autocommit = 0", Admin),
                ("USE other", Admin),
                ("CREATE USER intruder", Admin),
                ("KILL 42", Admin),
            ],
        );
    }

    #[test]
    fn classifies_sqlite_statements() {
        check(Dialect::Sqlite, SHARED);
        check(
            Dialect::Sqlite,
            &[
                ("SELECT [order] FROM \"t\"", Read),
                ("REPLACE INTO t (a) VALUES (1)", Write),
                (
                    "WITH gone AS (SELECT 1) DELETE FROM t WHERE id IN (SELECT * FROM gone)",
                    Write,
                ),
                ("BEGIN", Transaction),
                ("PRAGMA journal_mode = 'DELETE'", Admin),
                ("ATTACH DATABASE '/tmp/other.db' AS other", Admin),
                ("VACUUM", Admin),
            ],
        );
    }

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

    /// Statements the parser can't read, or reads as something other than reading: either
    /// way they are refused where only reading is allowed.
    #[test]
    fn never_mistakes_these_for_reading() {
        let cases = [
            (Dialect::Postgres, "DO $$ BEGIN DELETE FROM t; END $$"),
            (Dialect::Mysql, "SELECT * FROM t INTO OUTFILE '/tmp/out'"),
            (Dialect::Mysql, "SELECT * FROM t INTO DUMPFILE '/tmp/out'"),
            (Dialect::Sqlite, "PRAGMA table_info(t)"),
            (Dialect::Sqlite, "PRAGMA journal_mode = DELETE"),
        ];
        for (dialect, sql) in cases {
            assert_ne!(classify(sql, dialect), Ok(Read), "{dialect:?}: {sql}");
        }
    }
}
