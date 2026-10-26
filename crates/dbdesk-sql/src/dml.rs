//! The statements the grid writes to change a table's rows: one `UPDATE`, `INSERT` or `DELETE`
//! per row, naming the row by its primary key as it was loaded.
//!
//! Names are always quoted, so a statement means exactly that table and column whatever the
//! names spell. Values are string literals or `NULL`: the engines turn a string literal into
//! the column's type (PostgreSQL from an untyped literal, SQLite by type affinity, the others by
//! implicit conversion), which is how text from the grid (ADR-0004) goes back in without
//! dbdesk knowing the types.

use dbdesk_core::{Cell, Quoting, TableDescription};

use crate::Dialect;

/// A table as the database resolved it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Table<'a> {
    pub schema: &'a str,
    pub name: &'a str,
}

impl<'a> From<&'a TableDescription> for Table<'a> {
    fn from(table: &'a TableDescription) -> Self {
        Self {
            schema: &table.schema,
            name: &table.name,
        }
    }
}

/// A column and the value it has, or is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnValue<'a> {
    pub column: &'a str,
    pub value: &'a Cell,
}

/// Writes the statements that change one table's rows, in one dialect.
#[derive(Debug, Clone, Copy)]
pub struct RowSql<'a> {
    dialect: Dialect,
    table: Table<'a>,
}

impl<'a> RowSql<'a> {
    pub fn new(dialect: Dialect, table: impl Into<Table<'a>>) -> Self {
        Self {
            dialect,
            table: table.into(),
        }
    }

    /// `UPDATE … SET … WHERE <key>`.
    pub fn update(&self, set: &[ColumnValue<'_>], key: &[ColumnValue<'_>]) -> String {
        let assignments: Vec<String> = set
            .iter()
            .map(|assigned| {
                format!(
                    "{} = {}",
                    self.name(assigned.column),
                    self.literal(assigned.value)
                )
            })
            .collect();
        format!(
            "UPDATE {} SET {} WHERE {}",
            self.table_name(),
            assignments.join(", "),
            self.condition(key)
        )
    }

    /// `INSERT` of one row with these values; the columns not given get their defaults. With
    /// no values at all, every column gets its default.
    pub fn insert(&self, values: &[ColumnValue<'_>]) -> String {
        if values.is_empty() {
            return format!(
                "INSERT INTO {} {}",
                self.table_name(),
                self.dialect.default_row()
            );
        }
        let columns: Vec<String> = values.iter().map(|value| self.name(value.column)).collect();
        let literals: Vec<String> = values
            .iter()
            .map(|value| self.literal(value.value))
            .collect();
        format!(
            "INSERT INTO {} ({}) VALUES ({})",
            self.table_name(),
            columns.join(", "),
            literals.join(", ")
        )
    }

    /// `DELETE … WHERE <key>`.
    pub fn delete(&self, key: &[ColumnValue<'_>]) -> String {
        format!(
            "DELETE FROM {} WHERE {}",
            self.table_name(),
            self.condition(key)
        )
    }

    fn quoting(&self) -> Quoting {
        self.dialect.quoting()
    }

    /// Schema-qualified and always quoted, so it means exactly this table whatever the search
    /// path and whatever the names spell.
    fn table_name(&self) -> String {
        format!(
            "{}.{}",
            self.name(self.table.schema),
            self.name(self.table.name)
        )
    }

    fn name(&self, name: &str) -> String {
        self.quoting().identifier(name)
    }

    fn literal(&self, value: &Cell) -> String {
        match value {
            None => "NULL".to_owned(),
            Some(text) => self.quoting().string_literal(text),
        }
    }

    /// Every key column equal to its value; `IS NULL` for a `NULL`, which `=` never matches.
    fn condition(&self, key: &[ColumnValue<'_>]) -> String {
        key.iter()
            .map(|part| match part.value {
                None => format!("{} IS NULL", self.name(part.column)),
                value => format!("{} = {}", self.name(part.column), self.literal(value)),
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: Table<'static> = Table {
        schema: "shop",
        name: "order \"items\"",
    };

    fn text(value: &str) -> Cell {
        Some(value.to_owned())
    }

    /// The statements for one row of `TABLE`, in each dialect: an update, an insert of two
    /// values, an insert of nothing but defaults, and a delete.
    #[test]
    fn writes_each_statement_in_every_dialect() {
        let (id, note, empty) = (text("7"), text(r"it's C:\x"), None);
        let key = [ColumnValue {
            column: "id",
            value: &id,
        }];
        let values = [
            ColumnValue {
                column: "note",
                value: &note,
            },
            ColumnValue {
                column: "gone",
                value: &empty,
            },
        ];
        let expected = [
            (
                Dialect::Postgres,
                [
                    r#"UPDATE "shop"."order ""items""" SET "note" = 'it''s C:\x', "gone" = NULL WHERE "id" = '7'"#,
                    r#"INSERT INTO "shop"."order ""items""" ("note", "gone") VALUES ('it''s C:\x', NULL)"#,
                    r#"INSERT INTO "shop"."order ""items""" DEFAULT VALUES"#,
                    r#"DELETE FROM "shop"."order ""items""" WHERE "id" = '7'"#,
                ],
            ),
            (
                Dialect::Mysql,
                [
                    r#"UPDATE `shop`.`order "items"` SET `note` = 'it''s C:\\x', `gone` = NULL WHERE `id` = '7'"#,
                    r#"INSERT INTO `shop`.`order "items"` (`note`, `gone`) VALUES ('it''s C:\\x', NULL)"#,
                    r#"INSERT INTO `shop`.`order "items"` () VALUES ()"#,
                    r#"DELETE FROM `shop`.`order "items"` WHERE `id` = '7'"#,
                ],
            ),
            (
                Dialect::SqlServer,
                [
                    r#"UPDATE [shop].[order "items"] SET [note] = N'it''s C:\x', [gone] = NULL WHERE [id] = N'7'"#,
                    r#"INSERT INTO [shop].[order "items"] ([note], [gone]) VALUES (N'it''s C:\x', NULL)"#,
                    r#"INSERT INTO [shop].[order "items"] DEFAULT VALUES"#,
                    r#"DELETE FROM [shop].[order "items"] WHERE [id] = N'7'"#,
                ],
            ),
        ];
        for (dialect, [update, insert, defaults, delete]) in expected {
            let sql = RowSql::new(dialect, TABLE);
            assert_eq!(sql.update(&values, &key), update, "{dialect:?}");
            assert_eq!(sql.insert(&values), insert, "{dialect:?}");
            assert_eq!(sql.insert(&[]), defaults, "{dialect:?}");
            assert_eq!(sql.delete(&key), delete, "{dialect:?}");
        }
        for dialect in [Dialect::Sqlite, Dialect::DuckDb] {
            assert_eq!(
                RowSql::new(dialect, TABLE).insert(&[]),
                r#"INSERT INTO "shop"."order ""items""" DEFAULT VALUES"#
            );
        }
    }

    #[test]
    fn names_a_null_key_part_with_is_null() {
        let (id, part) = (text("1"), None);
        let key = [
            ColumnValue {
                column: "id",
                value: &id,
            },
            ColumnValue {
                column: "part",
                value: &part,
            },
        ];

        assert_eq!(
            RowSql::new(Dialect::Postgres, TABLE).delete(&key),
            r#"DELETE FROM "shop"."order ""items""" WHERE "id" = '1' AND "part" IS NULL"#
        );
    }
}
