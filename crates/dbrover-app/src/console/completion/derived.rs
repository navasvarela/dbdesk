//! Tables a statement defines for itself: CTEs (`WITH name AS (…)`) and subqueries in `FROM`
//! (`(…) AS alias`), with the columns their queries return.
//!
//! Found with the lexer, so a half-typed statement still has them; each query's columns are
//! read by `dbrover_sql::projection`, which uses the parser when the query parses.

use dbrover_sql::{Dialect, ProjectionItem, Token, TokenKind, is_keyword, projection, tokenize};

use super::{CatalogSource, TableRef, resolve, table_references};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivedKind {
    Cte,
    Subquery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedTable {
    pub kind: DerivedKind,
    /// The CTE's name or the subquery's alias.
    pub name: String,
    pub columns: Vec<String>,
}

/// Words after which a subquery in parentheses is a table: `FROM (…)`, `JOIN (…)`,
/// `JOIN LATERAL (…)`, and `, (…)` in a `FROM` list.
const SUBQUERY_INTRODUCERS: &[&str] = &["from", "join", "lateral", ","];
/// What may open the query inside a CTE's or subquery's parentheses.
const QUERY_STARTS: &[&str] = &["select", "with", "values"];

pub fn derived_tables(
    statement: &str,
    dialect: Dialect,
    catalog: &dyn CatalogSource,
) -> Vec<DerivedTable> {
    let tokens: Vec<Token> = tokenize(statement, dialect)
        .into_iter()
        .filter(|token| !token.kind.is_trivia())
        .collect();
    let closes = matching_parentheses(statement, &tokens);
    let text = |index: usize| {
        tokens
            .get(index)
            .map(|token| &statement[token.range.clone()])
    };
    let lower = |index: usize| text(index).map(str::to_ascii_lowercase);
    let name = |index: usize| {
        tokens
            .get(index)
            .and_then(|token| identifier(statement, token))
    };
    let body = |open: usize| -> Option<&str> {
        let close = closes[open]?;
        let start = tokens.get(open + 1)?.range.start;
        let end = tokens[close].range.start;
        (start <= end).then(|| &statement[start..end])
    };
    let opens_query = |open: usize| {
        text(open) == Some("(")
            && lower(open + 1).is_some_and(|word| QUERY_STARTS.contains(&word.as_str()))
    };

    let mut found = Vec::new();
    for index in 0..tokens.len() {
        // `name [(columns)] AS [NOT] [MATERIALIZED] (query)` after `WITH`, `RECURSIVE` or `,`.
        let after_with = index
            .checked_sub(1)
            .and_then(lower)
            .is_some_and(|before| matches!(before.as_str(), "with" | "recursive" | ","));
        if let (true, Some(cte)) = (after_with, name(index)) {
            let mut next = index + 1;
            let mut columns = None;
            if text(next) == Some("(") && !opens_query(next) {
                columns = closes[next].map(|close| names_between(statement, &tokens, next, close));
                next = closes[next].map_or(next, |close| close + 1);
            }
            if lower(next).as_deref() == Some("as") {
                next += 1;
                while matches!(lower(next).as_deref(), Some("not" | "materialized")) {
                    next += 1;
                }
                if opens_query(next)
                    && let Some(query) = body(next)
                {
                    found.push(DerivedTable {
                        kind: DerivedKind::Cte,
                        name: cte,
                        columns: columns.unwrap_or_else(|| query_columns(query, dialect, catalog)),
                    });
                    continue;
                }
            }
        }
        // `FROM (query) [AS] alias [(columns)]`.
        let introduced = index
            .checked_sub(1)
            .and_then(lower)
            .is_some_and(|before| SUBQUERY_INTRODUCERS.contains(&before.as_str()));
        if introduced
            && opens_query(index)
            && let Some(close) = closes[index]
            && let Some(query) = body(index)
        {
            let mut next = close + 1;
            if lower(next).as_deref() == Some("as") {
                next += 1;
            }
            let Some(alias) = name(next).filter(|alias| {
                !is_keyword(alias) || text(next).is_some_and(|t| t.starts_with('"'))
            }) else {
                continue;
            };
            let columns = match closes.get(next + 1).copied().flatten() {
                Some(list_close) if text(next + 1) == Some("(") => {
                    names_between(statement, &tokens, next + 1, list_close)
                }
                _ => query_columns(query, dialect, catalog),
            };
            found.push(DerivedTable {
                kind: DerivedKind::Subquery,
                name: alias,
                columns,
            });
        }
    }
    found
}

/// The columns a query returns: its select list, with `*` and `t.*` expanded to the columns
/// of its tables that the catalog has loaded.
fn query_columns(query: &str, dialect: Dialect, catalog: &dyn CatalogSource) -> Vec<String> {
    let references = table_references(query, dialect);
    let mut columns = Vec::new();
    for item in projection(query, dialect) {
        match item {
            ProjectionItem::Named(name) => columns.push(name),
            ProjectionItem::Wildcard { qualifier } => {
                columns.extend(wildcard_columns(&references, qualifier.as_deref(), catalog));
            }
            ProjectionItem::Unnamed => {}
        }
    }
    columns.dedup();
    columns
}

fn wildcard_columns(
    references: &[TableRef],
    qualifier: Option<&str>,
    catalog: &dyn CatalogSource,
) -> Vec<String> {
    let tables = catalog.tables();
    references
        .iter()
        .filter(|reference| {
            qualifier.is_none_or(|qualifier| {
                reference
                    .alias
                    .as_deref()
                    .unwrap_or(&reference.table)
                    .eq_ignore_ascii_case(qualifier)
            })
        })
        .filter_map(|reference| {
            resolve(
                reference.schema.as_deref(),
                &reference.table,
                &tables,
                catalog,
            )
        })
        .filter_map(|table| catalog.columns(table))
        .flatten()
        .map(|column| column.name)
        .collect()
}

/// For each `(` token, the index of its `)`, if it closes.
fn matching_parentheses(text: &str, tokens: &[Token]) -> Vec<Option<usize>> {
    let mut closes = vec![None; tokens.len()];
    let mut open = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match &text[token.range.clone()] {
            "(" => open.push(index),
            ")" => {
                if let Some(start) = open.pop() {
                    closes[start] = Some(index);
                }
            }
            _ => {}
        }
    }
    closes
}

/// The names in a column list between the parentheses at `open` and `close`.
fn names_between(text: &str, tokens: &[Token], open: usize, close: usize) -> Vec<String> {
    tokens[open + 1..close]
        .iter()
        .filter_map(|token| identifier(text, token))
        .collect()
}

fn identifier(text: &str, token: &Token) -> Option<String> {
    let written = &text[token.range.clone()];
    match token.kind {
        TokenKind::Word => Some(written.to_owned()),
        TokenKind::QuotedIdentifier => written
            .get(1..written.len().saturating_sub(1))
            .map(|inner| inner.replace("\"\"", "\"")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::NoCatalog;
    use super::*;

    fn derived(statement: &str) -> Vec<(DerivedKind, String, Vec<String>)> {
        derived_tables(statement, Dialect::Postgres, &NoCatalog)
            .into_iter()
            .map(|table| (table.kind, table.name, table.columns))
            .collect()
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn finds_ctes_and_subqueries_with_their_columns() {
        assert_eq!(
            derived(
                "with recent as (select id, total * 2 as doubled from orders), \
                 named (a, b) as materialized (select 1, 2) \
                 select * from recent r join (select id as order_id from items where) i on "
            ),
            [
                (
                    DerivedKind::Cte,
                    "recent".to_owned(),
                    names(&["id", "doubled"])
                ),
                (DerivedKind::Cte, "named".to_owned(), names(&["a", "b"])),
                (DerivedKind::Subquery, "i".to_owned(), names(&["order_id"])),
            ]
        );
        assert_eq!(
            derived("select * from (values (1, 'a')) as v (n, label)"),
            [(
                DerivedKind::Subquery,
                "v".to_owned(),
                names(&["n", "label"])
            )]
        );
        assert!(derived("select * from t where id in (select id from u) x").is_empty());
    }
}
