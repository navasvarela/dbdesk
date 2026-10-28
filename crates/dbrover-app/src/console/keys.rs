//! Following foreign keys from a result's rows: which cells link to the row they refer to, and
//! which tables have rows that refer to a row. Pure; the grid draws the links and the app opens
//! the rows in a new console.

use dbrover_core::{Cell, ColumnInfo, ForeignKey, Row, TableDescription, TableName};
use dbrover_sql::{ColumnValue, Dialect, RowSql};

/// Rows of a table to open, chosen by the values of some of its columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowsToOpen {
    pub table: TableName,
    /// Each column and the value it must have; never `NULL`, which no key refers by.
    pub filter: Vec<(String, String)>,
}

impl RowsToOpen {
    /// The `SELECT` that finds the rows.
    pub fn sql(&self, dialect: Dialect) -> String {
        let values: Vec<(&str, Cell)> = self
            .filter
            .iter()
            .map(|(column, value)| (column.as_str(), Some(value.clone())))
            .collect();
        let filter: Vec<ColumnValue<'_>> = values
            .iter()
            .map(|(column, value)| ColumnValue { column, value })
            .collect();
        RowSql::new(dialect, &self.table).select(&filter)
    }
}

/// A key as the result holds it: where each of its columns on this side is among the result's
/// columns, if all of them are (each named once).
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeyInResult {
    key: ForeignKey,
    positions: Option<Vec<usize>>,
}

impl KeyInResult {
    fn new(key: ForeignKey, own_columns: &[String], columns: &[ColumnInfo]) -> Self {
        let positions = own_columns
            .iter()
            .map(|name| position(columns, name))
            .collect();
        Self { key, positions }
    }

    /// This row's values in the key's columns, unless one is `NULL` or missing.
    fn values(&self, row: &Row) -> Option<Vec<String>> {
        self.positions
            .as_ref()?
            .iter()
            .map(|&position| row.get(position)?.clone())
            .collect()
    }
}

/// The result column called `name`, if exactly one is.
fn position(columns: &[ColumnInfo], name: &str) -> Option<usize> {
    let mut named = columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.name == name);
    match (named.next(), named.next()) {
        (Some((position, _)), None) => Some(position),
        _ => None,
    }
}

/// The foreign keys of the table a result's rows come from, mapped onto the result's columns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyLinks {
    /// The table's own keys, by the columns that hold them.
    outgoing: Vec<KeyInResult>,
    /// Other tables' keys to this one, by the columns they refer to.
    incoming: Vec<KeyInResult>,
}

/// A table whose rows may refer to a row, and how to open them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referencing {
    /// `orders.customer_id`: the table and its key's columns.
    pub label: String,
    /// The rows to open, or why they can't be found from this result.
    pub rows: Result<RowsToOpen, String>,
}

impl KeyLinks {
    pub fn new(table: &TableDescription, columns: &[ColumnInfo]) -> Self {
        Self {
            outgoing: table
                .foreign_keys
                .iter()
                .map(|key| KeyInResult::new(key.clone(), &key.columns, columns))
                .collect(),
            incoming: table
                .referenced_by
                .iter()
                .map(|key| KeyInResult::new(key.clone(), &key.referenced_columns, columns))
                .collect(),
        }
    }

    /// The row a cell's value refers to: `column` is part of a key whose columns are all in
    /// the result and none `NULL` in this row. The first such key, if several share the column.
    pub fn referenced(&self, column: usize, row: &Row) -> Option<RowsToOpen> {
        self.outgoing.iter().find_map(|link| {
            if !link.positions.as_ref()?.contains(&column) {
                return None;
            }
            let values = link.values(row)?;
            Some(RowsToOpen {
                table: link.key.referenced.clone(),
                filter: link
                    .key
                    .referenced_columns
                    .iter()
                    .cloned()
                    .zip(values)
                    .collect(),
            })
        })
    }

    /// The tables whose keys refer to the table, and the rows of each that refer to `row`.
    pub fn referencing(&self, row: &Row) -> Vec<Referencing> {
        self.incoming
            .iter()
            .map(|link| {
                let key = &link.key;
                let rows = link
                    .values(row)
                    .map(|values| RowsToOpen {
                        table: key.table.clone(),
                        filter: key.columns.iter().cloned().zip(values).collect(),
                    })
                    .ok_or_else(|| {
                        format!(
                            "Select {} to find the rows that refer to this one",
                            key.referenced_columns.join(", ")
                        )
                    });
                Referencing {
                    label: format!("{}.{}", key.table.name, key.columns.join(", ")),
                    rows,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use dbrover_core::ColumnDescription;

    use super::*;

    fn name(table: &str) -> TableName {
        TableName {
            schema: "public".to_owned(),
            name: table.to_owned(),
        }
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn result(names: &[&str]) -> Vec<ColumnInfo> {
        names
            .iter()
            .map(|name| ColumnInfo {
                name: (*name).to_owned(),
                type_name: None,
            })
            .collect()
    }

    fn row(values: &[Option<&str>]) -> Row {
        values
            .iter()
            .map(|value| value.map(str::to_owned))
            .collect()
    }

    /// `orders`, whose `(region, customer)` refers to `customers (region, id)`, and to which
    /// `items.order_id` refers.
    fn orders() -> TableDescription {
        TableDescription {
            schema: "public".to_owned(),
            name: "orders".to_owned(),
            columns: ["id", "region", "customer"]
                .into_iter()
                .map(|column| ColumnDescription {
                    name: column.to_owned(),
                    data_type: "integer".to_owned(),
                    nullable: true,
                    default: None,
                    identity: None,
                })
                .collect(),
            primary_key: strings(&["id"]),
            foreign_keys: vec![ForeignKey {
                table: name("orders"),
                columns: strings(&["region", "customer"]),
                referenced: name("customers"),
                referenced_columns: strings(&["region", "id"]),
            }],
            referenced_by: vec![ForeignKey {
                table: name("items"),
                columns: strings(&["order_id"]),
                referenced: name("orders"),
                referenced_columns: strings(&["id"]),
            }],
        }
    }

    #[test]
    fn links_each_column_of_a_key_to_the_row_it_refers_to() {
        let links = KeyLinks::new(&orders(), &result(&["customer", "id", "region"]));
        let customer = RowsToOpen {
            table: name("customers"),
            filter: vec![
                ("region".to_owned(), "eu".to_owned()),
                ("id".to_owned(), "7".to_owned()),
            ],
        };
        let values = row(&[Some("7"), Some("1"), Some("eu")]);

        assert_eq!(links.referenced(0, &values), Some(customer.clone()));
        assert_eq!(links.referenced(2, &values), Some(customer.clone()));
        assert_eq!(links.referenced(1, &values), None, "id is no foreign key");
        assert_eq!(
            links.referenced(0, &row(&[Some("7"), Some("1"), None])),
            None,
            "a NULL refers to nothing"
        );
        assert_eq!(
            customer.sql(Dialect::Postgres),
            r#"SELECT * FROM "public"."customers" WHERE "region" = 'eu' AND "id" = '7'"#
        );
    }

    #[test]
    fn needs_every_column_of_a_key_in_the_result() {
        let links = KeyLinks::new(&orders(), &result(&["customer", "id"]));

        assert_eq!(links.referenced(0, &row(&[Some("7"), Some("1")])), None);
    }

    #[test]
    fn finds_the_rows_that_refer_to_a_row() {
        let values = row(&[Some("1"), Some("eu")]);
        let with_id = KeyLinks::new(&orders(), &result(&["id", "region"]));
        assert_eq!(
            with_id.referencing(&values),
            [Referencing {
                label: "items.order_id".to_owned(),
                rows: Ok(RowsToOpen {
                    table: name("items"),
                    filter: vec![("order_id".to_owned(), "1".to_owned())],
                }),
            }]
        );

        let without_id = KeyLinks::new(&orders(), &result(&["region"]));
        let referencing = without_id.referencing(&row(&[Some("eu")]));
        assert_eq!(
            referencing[0].rows,
            Err("Select id to find the rows that refer to this one".to_owned())
        );
    }
}
