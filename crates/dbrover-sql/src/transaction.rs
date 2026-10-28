use crate::dialect::Dialect;
use crate::lexer::{TokenKind, first_word, tokenize};

/// First words of statements that start, end or partly undo a transaction. `prepare` also
/// starts ordinary prepared statements; treating those as transaction control only costs a
/// status check.
const TRANSACTION_CONTROL: &[&str] = &[
    "begin",
    "start",
    "commit",
    "end",
    "rollback",
    "abort",
    "savepoint",
    "release",
    "prepare",
];

/// Whether the statement may change the connection's transaction state, so it is checked
/// afterwards and isn't wrapped in a transaction of its own in manual mode.
pub fn controls_transactions(statement: &str, dialect: Dialect) -> bool {
    first_word(statement, dialect).is_some_and(|word| TRANSACTION_CONTROL.contains(&word.as_str()))
}

/// Whether running `statement` commits the open transaction first and itself at once, so
/// `ROLLBACK` can't undo either: DDL on MySQL. Temporary tables are the exception MySQL makes,
/// `CREATE TEMPORARY TABLE` and `DROP TEMPORARY TABLE` staying inside the transaction.
pub fn commits_implicitly(statement: &str, dialect: Dialect) -> bool {
    let mut words = tokenize(statement, dialect)
        .into_iter()
        .filter(|token| !token.kind.is_trivia())
        .map(|token| {
            (token.kind == TokenKind::Word).then(|| statement[token.range].to_ascii_lowercase())
        });
    let Some(Some(verb)) = words.next() else {
        return false;
    };
    dialect.implicit_commit_verbs().contains(&verb.as_str())
        && words.next().flatten().as_deref() != Some("temporary")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_transaction_control_by_its_first_word() {
        let cases = [
            ("BEGIN", true),
            (
                "  -- comment\n start transaction isolation level serializable",
                true,
            ),
            ("COMMIT AND CHAIN", true),
            ("rollback to savepoint a", true),
            ("END", true),
            ("SELECT 'begin'", false),
            (
                "CREATE TRIGGER t AFTER INSERT ON a BEGIN SELECT 1; END",
                false,
            ),
            ("", false),
        ];
        for (sql, expected) in cases {
            assert_eq!(
                controls_transactions(sql, Dialect::Postgres),
                expected,
                "{sql}"
            );
        }
    }

    #[test]
    fn knows_which_statements_commit_on_their_own() {
        let cases = [
            ("CREATE TABLE t (n int)", Dialect::Mysql, true),
            ("  -- why\n alter table t add c int", Dialect::Mysql, true),
            ("TRUNCATE t", Dialect::Mysql, true),
            ("CREATE TEMPORARY TABLE t (n int)", Dialect::Mysql, false),
            ("drop temporary table t", Dialect::Mysql, false),
            ("INSERT INTO t VALUES (1)", Dialect::Mysql, false),
            ("SELECT 'create'", Dialect::Mysql, false),
            ("CREATE TABLE t (n int)", Dialect::Postgres, false),
            ("DROP TABLE t", Dialect::Sqlite, false),
            ("CREATE TABLE t (n int)", Dialect::SqlServer, false),
        ];
        for (sql, dialect, expected) in cases {
            assert_eq!(
                commits_implicitly(sql, dialect),
                expected,
                "{dialect:?} {sql}"
            );
        }
    }
}
