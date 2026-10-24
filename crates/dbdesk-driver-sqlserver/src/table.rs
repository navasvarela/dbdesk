//! Describing a table for editing its rows.
//!
//! `OBJECT_ID` resolves a name as a statement on this connection would: the login's default
//! schema for an unqualified name, the database's collation for case. A `#temporary` table
//! lives in `tempdb` under a name padded with underscores and a suffix that tells sessions'
//! tables apart, so it is looked up there and reported under the name statements use.

use dbdesk_core::{DriverError, RelationName, TableDescription};

use crate::{SQL_SERVER_QUOTING, SqlClient, column, connection_error};

/// How long `tempdb` stores a local temporary table's name, before its 12-digit suffix.
const TEMPORARY_NAME_PADDED_LENGTH: usize = 116;

pub(crate) async fn describe_table(
    client: &mut SqlClient,
    table: &RelationName,
) -> Result<Option<TableDescription>, DriverError> {
    let written = table.to_sql(SQL_SERVER_QUOTING);
    let temporary = table.name.name.starts_with('#');
    let (catalog, object) = if temporary {
        ("tempdb.sys", format!("tempdb..{written}"))
    } else {
        ("sys", written)
    };
    let found = client
        .query(
            format!(
                "SELECT s.name, o.name FROM {catalog}.objects o \
                 JOIN {catalog}.schemas s ON s.schema_id = o.schema_id \
                 WHERE o.object_id = OBJECT_ID(@P1, N'U')"
            ),
            &[&object],
        )
        .await
        .map_err(connection_error)?
        .into_row()
        .await
        .map_err(connection_error)?;
    let names = found.as_ref().and_then(|row| {
        let schema: &str = column(row, 0)?;
        let name: &str = column(row, 1)?;
        Some((schema.to_owned(), name.to_owned()))
    });
    let Some((schema, stored_name)) = names else {
        return Ok(None);
    };
    let columns = names_of(
        client,
        &format!(
            "SELECT name FROM {catalog}.columns \
             WHERE object_id = OBJECT_ID(@P1, N'U') ORDER BY column_id"
        ),
        &object,
    )
    .await?;
    let primary_key = names_of(
        client,
        &format!(
            "SELECT c.name FROM {catalog}.indexes i \
             JOIN {catalog}.index_columns ic \
             ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
             JOIN {catalog}.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
             WHERE i.object_id = OBJECT_ID(@P1, N'U') AND i.is_primary_key = 1 \
             ORDER BY ic.key_ordinal"
        ),
        &object,
    )
    .await?;
    Ok(Some(TableDescription {
        schema,
        name: statement_name(&stored_name),
        columns,
        primary_key,
    }))
}

/// The name statements use for a table: what `tempdb` stores for a local temporary table,
/// without the padding and suffix.
fn statement_name(stored: &str) -> String {
    match stored.get(..TEMPORARY_NAME_PADDED_LENGTH) {
        Some(padded) if stored.starts_with('#') && !stored.starts_with("##") => {
            padded.trim_end_matches('_').to_owned()
        }
        _ => stored.to_owned(),
    }
}

async fn names_of(
    client: &mut SqlClient,
    sql: &str,
    object: &str,
) -> Result<Vec<String>, DriverError> {
    let rows = client
        .query(sql, &[&object])
        .await
        .map_err(connection_error)?
        .into_first_result()
        .await
        .map_err(connection_error)?;
    rows.iter()
        .map(|row| {
            column::<&str>(row, 0)
                .map(str::to_owned)
                .ok_or_else(|| connection_error(format!("{sql} returned no name")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_temporary_tables_as_statements_do() {
        let stored = format!("{:_<116}000000000123", "#orders");
        assert_eq!(statement_name(&stored), "#orders");
        assert_eq!(statement_name("##shared"), "##shared");
        assert_eq!(statement_name("orders"), "orders");
    }
}
