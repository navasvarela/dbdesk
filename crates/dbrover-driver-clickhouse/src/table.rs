//! Describing a table for editing its rows.
//!
//! A ClickHouse table has no key that tells its rows apart: its `PRIMARY KEY` is its sort
//! order and may repeat. So a description never has a primary key, and the grid keeps the
//! table's rows read-only.

use clickhouse::Client;
use dbrover_core::{
    CheckConstraint, ColumnDescription, DriverError, RelationName, TableConstraints,
    TableDescription, TableName,
};

use crate::CLICKHOUSE_QUOTING;
use crate::execution::rows;

/// Finds the table as a statement on the session would: a temporary table (database `''`)
/// first, since it hides a table of the same name in the current database. Names match
/// exactly, quoted or not.
const RESOLVE: &str = "SELECT database FROM system.tables \
     WHERE name = {name:String} AND engine NOT IN ('View', 'MaterializedView', 'Dictionary') \
     AND (database = {schema:String} OR (is_temporary AND {unqualified:Bool})) \
     ORDER BY is_temporary DESC LIMIT 1";

pub(crate) async fn describe_table(
    session: &Client,
    table: &RelationName,
) -> Result<Option<TableDescription>, DriverError> {
    let current;
    let (schema, unqualified) = match &table.schema {
        Some(schema) => (schema.name.as_str(), "false"),
        None => {
            current = crate::execution::single_value(session, "SELECT currentDatabase()")
                .await
                .map_err(crate::connection_error)?;
            (current.as_str(), "true")
        }
    };
    let name = table.name.name.as_str();
    let params = [
        ("name", name),
        ("schema", schema),
        ("unqualified", unqualified),
    ];
    let found = rows(session, RESOLVE, &params).await?;
    let Some(database) = found.first().and_then(|row| row.first().cloned().flatten()) else {
        return Ok(None);
    };
    let quoted = if database.is_empty() {
        CLICKHOUSE_QUOTING.identifier(name)
    } else {
        format!(
            "{}.{}",
            CLICKHOUSE_QUOTING.identifier(&database),
            CLICKHOUSE_QUOTING.identifier(name)
        )
    };
    let columns = rows(session, &format!("DESCRIBE TABLE {quoted}"), &[])
        .await?
        .into_iter()
        .filter_map(described_column)
        .collect();
    Ok(Some(TableDescription {
        schema: database,
        name: name.to_owned(),
        columns,
        primary_key: Vec::new(),
        // ClickHouse has no foreign keys.
        foreign_keys: Vec::new(),
        referenced_by: Vec::new(),
    }))
}

/// A row of `DESCRIBE TABLE`: the name, type, kind of default (`DEFAULT`, `MATERIALIZED`,
/// `ALIAS` or empty) and its expression, then comments and codecs. A column computed by
/// `MATERIALIZED` or `ALIAS` has no default to report.
fn described_column(row: Vec<Option<String>>) -> Option<ColumnDescription> {
    let mut fields = row.into_iter();
    let name = fields.next().flatten()?;
    let data_type = fields.next().flatten().unwrap_or_default();
    let default_kind = fields.next().flatten();
    let expression = fields.next().flatten();
    Some(ColumnDescription {
        nullable: is_nullable(&data_type),
        default: expression.filter(|_| default_kind.as_deref() == Some("DEFAULT")),
        identity: None,
        name,
        data_type,
    })
}

/// ClickHouse's types hold NULL only when wrapped in `Nullable`, which `LowCardinality` may
/// wrap in turn.
fn is_nullable(data_type: &str) -> bool {
    data_type.starts_with("Nullable(") || data_type.starts_with("LowCardinality(Nullable(")
}

/// The table's checks, from the statement `SHOW CREATE TABLE` writes, one constraint per line:
/// ClickHouse has no catalog table of them. Its other constraints (`ASSUME`) aren't checks,
/// and it has no unique or foreign keys.
pub(crate) async fn describe_constraints(
    session: &Client,
    table: &TableName,
) -> Result<TableConstraints, DriverError> {
    let statement = crate::execution::single_value(
        session,
        &format!(
            "SHOW CREATE TABLE {}.{}",
            CLICKHOUSE_QUOTING.identifier(&table.schema),
            CLICKHOUSE_QUOTING.identifier(&table.name)
        ),
    )
    .await
    .map_err(crate::connection_error)?;
    Ok(TableConstraints {
        checks: statement.lines().filter_map(check).collect(),
        ..TableConstraints::default()
    })
}

/// A line `CONSTRAINT name CHECK condition,` of `SHOW CREATE TABLE`, the name backquoted when
/// it needs to be.
fn check(line: &str) -> Option<CheckConstraint> {
    let rest = line.trim().strip_prefix("CONSTRAINT ")?;
    let (name, rest) = match rest.strip_prefix('`') {
        Some(quoted) => {
            let end = quoted.find('`')?;
            (quoted[..end].to_owned(), &quoted[end + 1..])
        }
        None => {
            let (name, rest) = rest.split_once(' ')?;
            (name.to_owned(), rest)
        }
    };
    let expression = rest.trim_start().strip_prefix("CHECK ")?;
    Some(CheckConstraint {
        name: Some(name),
        expression: expression.trim_end().trim_end_matches(',').to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_checks_from_the_create_statement() {
        let lines = [
            "    `id` Int32,",
            "    CONSTRAINT positive CHECK `Total Sum` > 0,",
            "    CONSTRAINT `Long name` CHECK id IN (1, 2),",
            "    CONSTRAINT a1 ASSUME id > 0",
        ];
        let checks: Vec<(String, String)> = lines
            .into_iter()
            .filter_map(check)
            .map(|check| (check.name.unwrap_or_default(), check.expression))
            .collect();
        assert_eq!(
            checks,
            [
                ("positive".to_owned(), "`Total Sum` > 0".to_owned()),
                ("Long name".to_owned(), "id IN (1, 2)".to_owned()),
            ]
        );
    }
}
