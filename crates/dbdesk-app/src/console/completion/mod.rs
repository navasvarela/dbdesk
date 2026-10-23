//! What to suggest at the cursor: SQL keywords, and the schemas, tables, views and columns
//! the cached catalog knows. Pure; the catalog comes in through [`CatalogSource`].

use std::ops::Range;

use dbdesk_core::quote_identifier;
use dbdesk_sql::{Dialect, Token, TokenKind, is_keyword, keywords, tokenize};

/// More would be a list nobody scrolls; typing narrows it.
const MAX_CANDIDATES: usize = 100;

/// Keywords after which a table name follows.
const TABLE_INTRODUCERS: &[&str] = &["from", "join", "update", "into"];

/// What the catalog knows, as far as it has been loaded.
pub trait CatalogSource {
    /// Where unqualified names resolve: `public` on PostgreSQL, `main` on SQLite.
    fn default_schema(&self) -> &str;
    fn schemas(&self) -> Vec<String>;
    fn tables(&self) -> Vec<TableEntry>;
    /// `None` if the table's columns haven't been loaded.
    fn columns(&self, table: &TableEntry) -> Option<Vec<ColumnEntry>>;
    /// Whether every table and view of `schema` has been loaded, or with `None`, of every
    /// schema (and the list of schemas itself): only then is a name missing from
    /// [`CatalogSource::tables`] really missing.
    fn tables_loaded(&self, schema: Option<&str>) -> bool;
}

/// A data source with nothing cached: completion offers keywords only.
pub struct NoCatalog;

impl CatalogSource for NoCatalog {
    fn default_schema(&self) -> &str {
        ""
    }

    fn schemas(&self) -> Vec<String> {
        Vec::new()
    }

    fn tables(&self) -> Vec<TableEntry> {
        Vec::new()
    }

    fn columns(&self, _: &TableEntry) -> Option<Vec<ColumnEntry>> {
        None
    }

    fn tables_loaded(&self, _: Option<&str>) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableEntry {
    pub schema: String,
    pub name: String,
    pub is_view: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnEntry {
    pub name: String,
    /// The type, as the explorer shows it.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateKind {
    Column,
    Table,
    View,
    Schema,
    Keyword,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub kind: CandidateKind,
    /// What the list shows.
    pub label: String,
    /// Shown dimmed after the label: a column's table and type, a table's schema.
    pub detail: Option<String>,
    /// What replaces the typed prefix.
    pub insert: String,
}

/// Where completion happens: the word being typed and what qualifies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionPoint {
    /// The typed prefix's bytes, which an accepted candidate replaces.
    pub replace: Range<usize>,
    pub prefix: String,
    /// The name before a `.` directly before the prefix: a schema, table or alias.
    pub qualifier: Option<String>,
}

/// A table the statement names, as `[schema.]table [[AS] alias]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRef {
    pub schema: Option<String>,
    pub table: String,
    pub alias: Option<String>,
    /// The bytes of `[schema.]table` in the statement.
    pub range: Range<usize>,
}

/// The completion point for a cursor at byte `cursor` of `text`, or `None` where nothing
/// should be suggested: inside a string, comment or quoted identifier, or after a number.
pub fn completion_point(text: &str, cursor: usize, dialect: Dialect) -> Option<CompletionPoint> {
    let before = text.get(..cursor)?;
    let tokens = tokenize(before, dialect);
    let last = tokens.last();
    let (replace, prefix_tokens) = match last {
        Some(token) if token.kind == TokenKind::Word => (token.range.clone(), tokens.len() - 1),
        Some(token)
            if matches!(
                token.kind,
                TokenKind::String
                    | TokenKind::LineComment
                    | TokenKind::BlockComment
                    | TokenKind::QuotedIdentifier
                    | TokenKind::Number
            ) =>
        {
            return None;
        }
        _ => (cursor..cursor, tokens.len()),
    };
    let qualifier = match tokens[..prefix_tokens] {
        [.., ref name, ref dot] if is_dot(before, dot) => identifier(before, name),
        _ => None,
    };
    Some(CompletionPoint {
        prefix: before[replace.clone()].to_owned(),
        replace,
        qualifier,
    })
}

fn is_dot(text: &str, token: &Token) -> bool {
    token.kind == TokenKind::Punctuation && &text[token.range.clone()] == "."
}

/// An identifier's name: a word as written, a quoted identifier unquoted.
fn identifier(text: &str, token: &Token) -> Option<String> {
    let written = &text[token.range.clone()];
    match token.kind {
        TokenKind::Word => Some(written.to_owned()),
        TokenKind::QuotedIdentifier => {
            let inner = written.get(1..written.len().checked_sub(1)?)?;
            Some(inner.replace("\"\"", "\""))
        }
        _ => None,
    }
}

/// The tables `statement` names after `FROM` (including comma-separated lists), `JOIN`,
/// `UPDATE` and `INTO`, with their aliases.
pub fn table_references(statement: &str, dialect: Dialect) -> Vec<TableRef> {
    let tokens: Vec<Token> = tokenize(statement, dialect)
        .into_iter()
        .filter(|token| !token.kind.is_trivia())
        .collect();
    let word = |index: usize| {
        tokens
            .get(index)
            .filter(|token| token.kind == TokenKind::Word)
            .map(|token| statement[token.range.clone()].to_ascii_lowercase())
    };
    let mut references = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        // `DO UPDATE SET` (an upsert) and `FOR [NO KEY] UPDATE` (locking) name no table.
        let not_a_statement = index
            .checked_sub(1)
            .and_then(word)
            .is_some_and(|before| matches!(before.as_str(), "do" | "for" | "key"));
        let introduces = word(index).is_some_and(|w| TABLE_INTRODUCERS.contains(&w.as_str()))
            && !(not_a_statement && word(index).as_deref() == Some("update"));
        index += 1;
        if !introduces {
            continue;
        }
        while let Some((reference, next)) = read_reference(statement, &tokens, index) {
            references.push(reference);
            index = next;
            let comma = tokens
                .get(index)
                .is_some_and(|token| &statement[token.range.clone()] == ",");
            if !comma {
                break;
            }
            index += 1;
        }
    }
    references
}

/// Reads `[schema.]table [[AS] alias]` at `index`; returns it and the index after it.
fn read_reference(text: &str, tokens: &[Token], index: usize) -> Option<(TableRef, usize)> {
    let first = identifier(text, tokens.get(index)?)?;
    let (schema, table, mut next) = match tokens.get(index + 1) {
        Some(dot) if is_dot(text, dot) => {
            let table = identifier(text, tokens.get(index + 2)?)?;
            (Some(first), table, index + 3)
        }
        _ => (None, first, index + 1),
    };
    let lowercase_word = |at: usize| {
        tokens
            .get(at)
            .filter(|token| token.kind == TokenKind::Word)
            .map(|token| text[token.range.clone()].to_ascii_lowercase())
    };
    if lowercase_word(next).as_deref() == Some("as") {
        next += 1;
    }
    let alias = tokens
        .get(next)
        .and_then(|token| identifier(text, token))
        .filter(|name| !is_keyword(name));
    if alias.is_some() {
        next += 1;
    }
    let name_end = tokens[if schema.is_some() { index + 2 } else { index }]
        .range
        .end;
    Some((
        TableRef {
            schema,
            table,
            alias,
            range: tokens[index].range.start..name_end,
        },
        next,
    ))
}

/// The candidates for `point`, best first: with a qualifier, the columns of the table or
/// alias it names, or the tables of the schema it names; without one, the columns of the
/// statement's tables, then tables and views, schemas and keywords.
pub fn candidates(
    point: &CompletionPoint,
    references: &[TableRef],
    catalog: &dyn CatalogSource,
) -> Vec<Candidate> {
    let mut found = match &point.qualifier {
        Some(qualifier) => qualified(qualifier, references, catalog),
        None => unqualified(point, references, catalog),
    };
    // A name typed in full has nothing left to complete; offering it again would only make
    // Enter accept it instead of breaking the line.
    found.retain(|candidate| {
        starts_with_ignoring_case(&candidate.label, &point.prefix)
            && !candidate.label.eq_ignore_ascii_case(&point.prefix)
    });
    found.truncate(MAX_CANDIDATES);
    found
}

fn qualified(
    qualifier: &str,
    references: &[TableRef],
    catalog: &dyn CatalogSource,
) -> Vec<Candidate> {
    let tables = catalog.tables();
    let referenced = references.iter().find(|reference| {
        reference
            .alias
            .as_deref()
            .unwrap_or(&reference.table)
            .eq_ignore_ascii_case(qualifier)
    });
    let table = match referenced {
        Some(reference) => resolve(
            reference.schema.as_deref(),
            &reference.table,
            &tables,
            catalog,
        ),
        None => resolve(None, qualifier, &tables, catalog),
    };
    if let Some(table) = table {
        return column_candidates(table, catalog);
    }
    let schema_named = catalog
        .schemas()
        .into_iter()
        .find(|schema| schema.eq_ignore_ascii_case(qualifier));
    match schema_named {
        Some(schema) => tables
            .iter()
            .filter(|table| table.schema == schema)
            .map(|table| table_candidate(table, quote_identifier(&table.name)))
            .collect(),
        None => Vec::new(),
    }
}

fn unqualified(
    point: &CompletionPoint,
    references: &[TableRef],
    catalog: &dyn CatalogSource,
) -> Vec<Candidate> {
    let tables = catalog.tables();
    let mut found = Vec::new();
    for reference in references {
        if let Some(table) = resolve(
            reference.schema.as_deref(),
            &reference.table,
            &tables,
            catalog,
        ) {
            for column in column_candidates(table, catalog) {
                if !found
                    .iter()
                    .any(|seen: &Candidate| seen.label == column.label)
                {
                    found.push(column);
                }
            }
        }
    }
    let default_schema = catalog.default_schema();
    found.extend(tables.iter().map(|table| {
        let insert = if table.schema == default_schema {
            quote_identifier(&table.name)
        } else {
            format!(
                "{}.{}",
                quote_identifier(&table.schema),
                quote_identifier(&table.name)
            )
        };
        table_candidate(table, insert)
    }));
    found.extend(catalog.schemas().into_iter().map(|schema| Candidate {
        kind: CandidateKind::Schema,
        insert: quote_identifier(&schema),
        label: schema,
        detail: None,
    }));
    // Every keyword matches an empty prefix, which would bury the names.
    if !point.prefix.is_empty() {
        let uppercase = point.prefix.chars().any(|c| c.is_ascii_uppercase());
        found.extend(keywords().iter().map(|keyword| {
            let insert = if uppercase {
                keyword.to_ascii_uppercase()
            } else {
                (*keyword).to_owned()
            };
            Candidate {
                kind: CandidateKind::Keyword,
                label: insert.clone(),
                detail: None,
                insert,
            }
        }));
    }
    found
}

/// The catalog's table for a reference: in its schema if it names one, else in the default
/// schema, else in whichever schema has it.
pub(super) fn resolve<'a>(
    schema: Option<&str>,
    name: &str,
    tables: &'a [TableEntry],
    catalog: &dyn CatalogSource,
) -> Option<&'a TableEntry> {
    let named = |table: &&TableEntry| table.name.eq_ignore_ascii_case(name);
    match schema {
        Some(schema) => tables
            .iter()
            .filter(named)
            .find(|table| table.schema.eq_ignore_ascii_case(schema)),
        None => tables
            .iter()
            .filter(named)
            .find(|table| table.schema == catalog.default_schema())
            .or_else(|| tables.iter().find(named)),
    }
}

fn column_candidates(table: &TableEntry, catalog: &dyn CatalogSource) -> Vec<Candidate> {
    catalog
        .columns(table)
        .unwrap_or_default()
        .into_iter()
        .map(|column| Candidate {
            kind: CandidateKind::Column,
            insert: quote_identifier(&column.name),
            detail: Some(match column.detail {
                Some(detail) => format!("{} · {detail}", table.name),
                None => table.name.clone(),
            }),
            label: column.name,
        })
        .collect()
}

fn table_candidate(table: &TableEntry, insert: String) -> Candidate {
    Candidate {
        kind: if table.is_view {
            CandidateKind::View
        } else {
            CandidateKind::Table
        },
        label: table.name.clone(),
        detail: Some(table.schema.clone()),
        insert,
    }
}

fn starts_with_ignoring_case(label: &str, prefix: &str) -> bool {
    label
        .get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeCatalog;

    impl CatalogSource for FakeCatalog {
        fn default_schema(&self) -> &str {
            "public"
        }

        fn schemas(&self) -> Vec<String> {
            vec!["public".to_owned(), "Sales".to_owned()]
        }

        fn tables(&self) -> Vec<TableEntry> {
            let table = |schema: &str, name: &str, is_view| TableEntry {
                schema: schema.to_owned(),
                name: name.to_owned(),
                is_view,
            };
            vec![
                table("public", "orders", false),
                table("public", "order_totals", true),
                table("Sales", "Order Items", false),
            ]
        }

        fn columns(&self, table: &TableEntry) -> Option<Vec<ColumnEntry>> {
            let column = |name: &str| ColumnEntry {
                name: name.to_owned(),
                detail: Some("integer".to_owned()),
            };
            match table.name.as_str() {
                "orders" => Some(vec![column("id"), column("customer_id")]),
                "Order Items" => Some(vec![column("order_id"), column("Quantity")]),
                _ => None,
            }
        }

        fn tables_loaded(&self, _: Option<&str>) -> bool {
            true
        }
    }

    fn point_at_end(text: &str) -> Option<CompletionPoint> {
        completion_point(text, text.len(), Dialect::Postgres)
    }

    fn suggest(text: &str) -> Vec<(CandidateKind, String)> {
        let point = point_at_end(text).unwrap();
        let references = table_references(text, Dialect::Postgres);
        candidates(&point, &references, &FakeCatalog)
            .into_iter()
            .map(|candidate| (candidate.kind, candidate.insert))
            .collect()
    }

    #[test]
    fn finds_the_prefix_and_its_qualifier() {
        let cases = [
            ("select ord", "ord", None),
            ("select o.", "", Some("o")),
            ("select o.cu", "cu", Some("o")),
            ("select \"Sales\".Ord", "Ord", Some("Sales")),
            ("select ", "", None),
        ];
        for (text, prefix, qualifier) in cases {
            let point = point_at_end(text).unwrap();
            assert_eq!(point.prefix, prefix, "{text}");
            assert_eq!(point.qualifier.as_deref(), qualifier, "{text}");
            assert_eq!(&text[point.replace], prefix, "{text}");
        }
        for text in ["select 'ord", "-- ord", "select 12", "select \"ord\""] {
            assert_eq!(point_at_end(text), None, "{text}");
        }
    }

    #[test]
    fn reads_tables_and_aliases_from_the_statement() {
        let references = table_references(
            "SELECT * FROM orders o, \"Sales\".\"Order Items\" AS items \
             JOIN customers ON true WHERE x",
            Dialect::Postgres,
        );
        let read: Vec<(Option<&str>, &str, Option<&str>)> = references
            .iter()
            .map(|r| (r.schema.as_deref(), r.table.as_str(), r.alias.as_deref()))
            .collect();
        assert_eq!(
            read,
            [
                (None, "orders", Some("o")),
                (Some("Sales"), "Order Items", Some("items")),
                (None, "customers", None),
            ]
        );
    }

    #[test]
    fn suggests_columns_of_an_alias_or_table_after_a_dot() {
        // The cursor sits after `o.c`; the alias is defined later in the statement.
        let point = completion_point("select o.c from orders o", 10, Dialect::Postgres).unwrap();
        let references = table_references("select o.c from orders o", Dialect::Postgres);
        let found: Vec<String> = candidates(&point, &references, &FakeCatalog)
            .into_iter()
            .map(|candidate| candidate.insert)
            .collect();
        assert_eq!(found, ["customer_id"]);
        assert_eq!(
            suggest("select * from \"Sales\".\"Order Items\" i where i."),
            [
                (CandidateKind::Column, "order_id".to_owned()),
                (CandidateKind::Column, "\"Quantity\"".to_owned())
            ]
        );
    }

    #[test]
    fn suggests_a_schemas_tables_after_its_name() {
        assert_eq!(
            suggest("select * from sales."),
            [(CandidateKind::Table, "\"Order Items\"".to_owned())]
        );
    }

    #[test]
    fn suggests_columns_then_tables_schemas_and_keywords_matching_the_prefix() {
        assert_eq!(
            suggest("select * from orders where cus"),
            [(CandidateKind::Column, "customer_id".to_owned())]
        );
        assert_eq!(
            suggest("select * from Ord"),
            [
                (CandidateKind::Table, "orders".to_owned()),
                (CandidateKind::View, "order_totals".to_owned()),
                (CandidateKind::Table, "\"Sales\".\"Order Items\"".to_owned()),
                (CandidateKind::Keyword, "ORDER".to_owned()),
            ]
        );
        assert!(
            suggest("select * from ")
                .iter()
                .all(|(kind, _)| *kind != CandidateKind::Keyword)
        );
    }
}
