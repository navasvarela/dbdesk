use crate::dialect::Dialect;
use crate::lexer::first_word;

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
}
