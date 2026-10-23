use std::ops::Range;

use crate::dialect::Dialect;
use crate::lexer::{TokenKind, tokenize};

/// Splits a console's text into statements at top-level semicolons.
///
/// Each range covers one statement from its first to its last meaningful token: surrounding
/// whitespace and comments, and the terminating semicolon, are left out. Statements with no
/// meaningful tokens (stray `;`, comment-only stretches) are dropped.
///
/// Semicolons inside strings, quoted identifiers, comments and dollar-quoted bodies don't
/// split. Neither do semicolons inside a `BEGIN … END` block of a `CREATE` statement (SQLite
/// trigger bodies, PostgreSQL `BEGIN ATOMIC` functions), or inside `CASE … END`. An
/// unclosed block therefore runs to the end of the text, as the database would read it.
pub fn split_statements(sql: &str, dialect: Dialect) -> Vec<Range<usize>> {
    let mut statements = Vec::new();
    let mut current: Option<Range<usize>> = None;
    let mut first_word: Option<String> = None;
    let mut block_depth = 0usize;

    for token in tokenize(sql, dialect) {
        match token.kind {
            kind if kind.is_trivia() => {}
            TokenKind::Semicolon if block_depth == 0 => {
                statements.extend(current.take());
                first_word = None;
            }
            kind => {
                if kind == TokenKind::Word {
                    let word = sql[token.range.clone()].to_ascii_lowercase();
                    let starts_block = word == "case"
                        || (word == "begin" && first_word.as_deref() == Some("create"));
                    if starts_block {
                        block_depth += 1;
                    } else if word == "end" {
                        block_depth = block_depth.saturating_sub(1);
                    }
                    first_word.get_or_insert(word);
                }
                current = Some(match current {
                    Some(range) => range.start..token.range.end,
                    None => token.range,
                });
            }
        }
    }
    statements.extend(current);
    statements
}

/// The index of the statement to run for a cursor at byte offset `cursor`: the statement
/// containing it, else the nearest one before it (the cursor usually sits just after the
/// statement's `;`), else the first one after it.
pub fn statement_at(statements: &[Range<usize>], cursor: usize) -> Option<usize> {
    statements
        .iter()
        .rposition(|statement| statement.start <= cursor)
        .or_else(|| (!statements.is_empty()).then_some(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statements(sql: &str, dialect: Dialect) -> Vec<&str> {
        split_statements(sql, dialect)
            .into_iter()
            .map(|range| &sql[range])
            .collect()
    }

    #[test]
    fn splits_only_at_top_level_semicolons() {
        let cases: [(&str, Dialect, &[&str]); 7] = [
            (
                "select 1; select 2",
                Dialect::Postgres,
                &["select 1", "select 2"],
            ),
            (
                "  -- leading\n select ';' ; ;; /* only a comment */ ;",
                Dialect::Postgres,
                &["select ';'"],
            ),
            (
                "create function f() returns int as $$ select 1; $$ language sql; select f()",
                Dialect::Postgres,
                &[
                    "create function f() returns int as $$ select 1; $$ language sql",
                    "select f()",
                ],
            ),
            (
                "CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE b SET n = 1; DELETE FROM c; END; SELECT 1",
                Dialect::Sqlite,
                &[
                    "CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE b SET n = 1; DELETE FROM c; END",
                    "SELECT 1",
                ],
            ),
            (
                "BEGIN; update t set a = case when b then 1 else 2 end; COMMIT",
                Dialect::Postgres,
                &[
                    "BEGIN",
                    "update t set a = case when b then 1 else 2 end",
                    "COMMIT",
                ],
            ),
            (
                "select 'unterminated; select 2",
                Dialect::Sqlite,
                &["select 'unterminated; select 2"],
            ),
            ("", Dialect::Sqlite, &[]),
        ];

        for (sql, dialect, expected) in cases {
            assert_eq!(statements(sql, dialect), expected, "{sql}");
        }
    }

    #[test]
    fn picks_the_statement_under_or_before_the_cursor() {
        let sql = "  select 1;\n\nselect 2;  ";
        let ranges = split_statements(sql, Dialect::Postgres);

        let cases = [(0, 0), (4, 0), (11, 0), (13, 1), (sql.len(), 1)];
        for (cursor, expected) in cases {
            assert_eq!(
                statement_at(&ranges, cursor),
                Some(expected),
                "cursor {cursor}"
            );
        }
        assert_eq!(statement_at(&[], 3), None);
    }
}
