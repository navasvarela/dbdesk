//! `CREATE VIEW` and materialized views: creating, replacing the query of, renaming,
//! refreshing and dropping them; and finding the query in a view's definition, for Edit view.

use super::{DdlError, ObjectName, ViewKind, ViewSpec, rename, require_name};
use crate::dialect::ViewReplacement;
use crate::{Dialect, TokenKind, tokenize};

pub(super) fn create(spec: &ViewSpec, dialect: Dialect) -> Result<String, DdlError> {
    validate(spec, dialect)?;
    Ok(create_statement("CREATE", spec, dialect))
}

/// The statements that give an existing view `spec`'s query, as the dialect allows. A
/// materialized view has no `CREATE OR REPLACE`, so it is dropped and created again.
pub(super) fn replace(spec: &ViewSpec, dialect: Dialect) -> Result<Vec<String>, DdlError> {
    validate(spec, dialect)?;
    let drop_and_create = || {
        vec![
            drop_statement(&spec.name, spec.kind, false, dialect),
            create_statement("CREATE", spec, dialect),
        ]
    };
    if spec.kind == ViewKind::Materialized {
        return Ok(drop_and_create());
    }
    Ok(match dialect.capabilities().view_replacement {
        ViewReplacement::OrReplace => vec![create_statement("CREATE OR REPLACE", spec, dialect)],
        ViewReplacement::Alter => vec![create_statement("ALTER", spec, dialect)],
        ViewReplacement::DropAndCreate => drop_and_create(),
    })
}

/// `{verb} VIEW name AS query`, the query on a line of its own as typed.
fn create_statement(verb: &str, spec: &ViewSpec, dialect: Dialect) -> String {
    let mut sql = format!(
        "{verb} {} {} AS\n{}",
        spec.kind.keyword(),
        spec.name.to_sql(dialect),
        query(spec)
    );
    if spec.kind == ViewKind::Materialized && !spec.with_data {
        sql.push_str("\nWITH NO DATA");
    }
    sql
}

/// The query without the spaces and `;` it may end with: the statement is ended by whoever
/// runs it.
fn query(spec: &ViewSpec) -> &str {
    spec.query.trim().trim_end_matches(';').trim_end()
}

fn validate(spec: &ViewSpec, dialect: Dialect) -> Result<(), DdlError> {
    require_name(&spec.name.name, spec.kind.noun())?;
    require_materialized_views(spec.kind, dialect)?;
    if query(spec).is_empty() {
        return Err(DdlError::EmptyQuery(spec.kind.noun()));
    }
    Ok(())
}

fn require_materialized_views(kind: ViewKind, dialect: Dialect) -> Result<(), DdlError> {
    if kind == ViewKind::Materialized && !dialect.capabilities().materialized_views {
        return Err(DdlError::Unsupported {
            what: "materialized views DbRover can create",
            dialect,
        });
    }
    Ok(())
}

pub(super) fn rename_view(
    view: &ObjectName,
    kind: ViewKind,
    new_name: &str,
    dialect: Dialect,
) -> Result<String, DdlError> {
    require_materialized_views(kind, dialect)?;
    if !dialect.capabilities().renames_views {
        return Err(DdlError::Unsupported {
            what: "way to rename a view",
            dialect,
        });
    }
    require_name(new_name, "view's new name")?;
    Ok(rename(kind.keyword(), view, new_name, dialect))
}

pub(super) fn refresh(
    view: &ObjectName,
    concurrently: bool,
    dialect: Dialect,
) -> Result<String, DdlError> {
    require_materialized_views(ViewKind::Materialized, dialect)?;
    let concurrently = if concurrently { "CONCURRENTLY " } else { "" };
    Ok(format!(
        "REFRESH MATERIALIZED VIEW {concurrently}{}",
        view.to_sql(dialect)
    ))
}

pub(super) fn drop(
    view: &ObjectName,
    kind: ViewKind,
    cascade: bool,
    dialect: Dialect,
) -> Result<String, DdlError> {
    require_materialized_views(kind, dialect)?;
    if cascade && !dialect.capabilities().drop_cascade {
        return Err(DdlError::Unsupported {
            what: "CASCADE for dropping a view",
            dialect,
        });
    }
    Ok(drop_statement(view, kind, cascade, dialect))
}

fn drop_statement(view: &ObjectName, kind: ViewKind, cascade: bool, dialect: Dialect) -> String {
    let cascade = if cascade { " CASCADE" } else { "" };
    format!("DROP {} {}{cascade}", kind.keyword(), view.to_sql(dialect))
}

/// The query of a view whose definition, as the catalog shows it, is `definition`: what
/// follows the first `AS` after `VIEW`, outside parentheses, without a closing `;`. `None` if
/// the definition isn't a `CREATE … VIEW … AS …` statement.
///
/// Every engine's definition has that shape, whatever comes between `VIEW` and `AS`: MySQL
/// writes `ALGORITHM=… DEFINER=… SQL SECURITY … VIEW`, SQLite and ClickHouse may list the
/// columns in parentheses after the name, and SQL Server may add `WITH SCHEMABINDING`. Those
/// options are not part of the query, so Edit view leaves them out.
pub fn view_query(definition: &str, dialect: Dialect) -> Option<String> {
    let tokens = tokenize(definition, dialect);
    let mut words = tokens
        .iter()
        .filter(|token| !token.kind.is_trivia())
        .scan(0usize, |depth, token| {
            let text = &definition[token.range.clone()];
            let outside = *depth == 0;
            match text {
                "(" => *depth += 1,
                ")" => *depth = depth.saturating_sub(1),
                _ => {}
            }
            Some((outside && token.kind == TokenKind::Word).then_some((text, token)))
        })
        .flatten();
    let (create, _) = words.next()?;
    if !create.eq_ignore_ascii_case("create") {
        return None;
    }
    words.find(|(word, _)| word.eq_ignore_ascii_case("view"))?;
    let (_, as_token) = words.find(|(word, _)| word.eq_ignore_ascii_case("as"))?;
    let body = definition[as_token.range.end..]
        .trim()
        .trim_end_matches(';')
        .trim_end();
    (!body.is_empty()).then(|| body.to_owned())
}
