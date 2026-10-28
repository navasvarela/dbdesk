//! Which kind of name belongs at the cursor, from the clause it is in. Read from the lexer's
//! tokens, so it works on half-typed SQL.

use dbrover_sql::{Dialect, Token, TokenKind, is_keyword, tokenize};

/// What the cursor's position in the statement calls for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Context {
    /// A column or an expression: after `SELECT`, `WHERE`, `ON`, `GROUP BY`, `ORDER BY`,
    /// `HAVING`, `SET`, `RETURNING`, and inside their parentheses.
    Columns,
    /// A table: after `FROM`, `JOIN`, `UPDATE`, `INTO`, and after a comma in a `FROM` list.
    Tables,
    /// The column list of `INSERT INTO table (…)`.
    InsertColumns {
        schema: Option<String>,
        table: String,
    },
    /// Just after a table name (and its alias), where a keyword follows: `WHERE`, `JOIN`, …
    AfterTable,
    /// Anywhere else, such as a statement's first word: everything is offered.
    Any,
}

/// The context for a word typed right after `before`, the statement's text up to the typed
/// prefix.
pub fn context_at(before: &str, dialect: Dialect) -> Context {
    let tokens: Vec<Token> = tokenize(before, dialect)
        .into_iter()
        .filter(|token| !token.kind.is_trivia())
        .collect();
    let text = |index: usize| tokens.get(index).map(|token| &before[token.range.clone()]);
    let word = |index: usize| {
        tokens
            .get(index)
            .filter(|token| token.kind == TokenKind::Word)
            .map(|token| before[token.range.clone()].to_ascii_lowercase())
    };
    // One context per open parenthesis, innermost last.
    let mut levels = vec![Context::Any];
    for index in 0..tokens.len() {
        let current = levels.last().cloned().unwrap_or(Context::Any);
        let next = match (text(index), word(index).as_deref()) {
            (Some("("), _) => {
                levels.push(match insert_target(before, &tokens, index) {
                    Some((schema, table)) => Context::InsertColumns { schema, table },
                    None if current == Context::Tables => Context::Any,
                    None => current,
                });
                continue;
            }
            (Some(")"), _) => {
                let closed = levels.pop();
                if levels.is_empty() {
                    levels.push(Context::Any);
                }
                // `FROM (SELECT …) s`: after the subquery comes its alias, then a keyword.
                let opened_in_tables = matches!(levels.last(), Some(Context::Tables));
                if opened_in_tables && closed.is_some() {
                    Context::AfterTable
                } else {
                    continue;
                }
            }
            (Some(","), _) if current == Context::AfterTable => Context::Tables,
            (Some("."), _) if current == Context::AfterTable => Context::Tables,
            (_, Some("select" | "where" | "on" | "having" | "set" | "returning")) => {
                Context::Columns
            }
            (_, Some("by")) => Context::Columns,
            (_, Some("from" | "join" | "update" | "into")) => Context::Tables,
            (_, Some("values")) => Context::Any,
            _ if current == Context::Tables && is_name(before, &tokens[index]) => {
                Context::AfterTable
            }
            _ => continue,
        };
        if let Some(level) = levels.last_mut() {
            *level = next;
        }
    }
    levels.pop().unwrap_or(Context::Any)
}

/// A table name rather than a keyword (`FROM ONLY t`, `JOIN LATERAL (…)` keep expecting one).
fn is_name(text: &str, token: &Token) -> bool {
    match token.kind {
        TokenKind::QuotedIdentifier => true,
        TokenKind::Word => !is_keyword(&text[token.range.clone()]),
        _ => false,
    }
}

/// For a `(` at `open` that follows `INSERT INTO [schema.]table` (or SQLite's `INSERT OR …
/// INTO`, `REPLACE INTO`), the table.
fn insert_target(text: &str, tokens: &[Token], open: usize) -> Option<(Option<String>, String)> {
    let name = |index: usize| {
        let token = tokens.get(index)?;
        let written = &text[token.range.clone()];
        match token.kind {
            TokenKind::Word => Some(written.to_owned()),
            TokenKind::QuotedIdentifier => written
                .get(1..written.len().saturating_sub(1))
                .map(|inner| inner.replace("\"\"", "\"")),
            _ => None,
        }
    };
    let is = |index: usize, expected: &str| {
        tokens
            .get(index)
            .is_some_and(|token| text[token.range.clone()].eq_ignore_ascii_case(expected))
    };
    let table_at = open.checked_sub(1)?;
    let table = name(table_at)?;
    let (schema, into_at) = match table_at.checked_sub(2) {
        Some(schema_at) if is(schema_at + 1, ".") => (name(schema_at), schema_at.checked_sub(1)?),
        _ => (None, table_at.checked_sub(1)?),
    };
    is(into_at, "into").then_some((schema, table))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(table: &str) -> Context {
        Context::InsertColumns {
            schema: None,
            table: table.to_owned(),
        }
    }

    #[test]
    fn reads_the_clause_the_cursor_is_in() {
        let cases = [
            ("", Context::Any),
            ("select ", Context::Columns),
            ("select a, count(", Context::Columns),
            ("select * from ", Context::Tables),
            ("select * from orders ", Context::AfterTable),
            ("select * from orders o ", Context::AfterTable),
            ("select * from orders o, ", Context::Tables),
            ("select * from public.", Context::Tables),
            ("select * from orders o join ", Context::Tables),
            (
                "select * from orders o join customers c on ",
                Context::Columns,
            ),
            ("select * from t where a = 1 and ", Context::Columns),
            ("select * from t group by ", Context::Columns),
            ("select * from t order by a, ", Context::Columns),
            ("update t set ", Context::Columns),
            ("update ", Context::Tables),
            ("delete from ", Context::Tables),
            ("insert into ", Context::Tables),
            ("insert into orders (id, ", insert("orders")),
            ("insert or replace into kv (", insert("kv")),
            ("insert into orders (id) values (", Context::Any),
            ("select * from (select ", Context::Columns),
            ("select * from (select a from b) s ", Context::AfterTable),
            (
                "select * from t where id in (select id from ",
                Context::Tables,
            ),
            ("select * from t where id in (", Context::Columns),
        ];
        for (before, expected) in cases {
            assert_eq!(
                context_at(before, Dialect::Postgres),
                expected,
                "{before:?}"
            );
        }
        assert_eq!(
            context_at("insert into sales.\"Orders\" (", Dialect::Postgres),
            Context::InsertColumns {
                schema: Some("sales".to_owned()),
                table: "Orders".to_owned()
            }
        );
    }
}
