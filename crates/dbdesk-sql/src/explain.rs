use crate::lexer::Dialect;

/// What to put before a statement to have the database describe how it would run it, without
/// running it: `EXPLAIN` on PostgreSQL, `EXPLAIN QUERY PLAN` on SQLite (whose plain `EXPLAIN`
/// lists virtual machine opcodes instead).
pub fn explain_prefix(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Postgres => "EXPLAIN ",
        Dialect::Sqlite => "EXPLAIN QUERY PLAN ",
    }
}
