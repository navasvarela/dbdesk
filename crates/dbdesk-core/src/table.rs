//! Tables as a statement names them, and what editing their rows needs to know.

use crate::Quoting;

/// A name as written in SQL. Whether it was quoted matters: PostgreSQL folds unquoted names
/// to lowercase, and SQLite matches them ignoring case.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identifier {
    pub name: String,
    pub quoted: bool,
}

impl Identifier {
    /// The identifier as SQL text, quoted as it was written.
    pub fn to_sql(&self, quoting: Quoting) -> String {
        if self.quoted {
            quoting.identifier(&self.name)
        } else {
            self.name.clone()
        }
    }
}

/// `[schema.]table`, as a statement names it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelationName {
    pub schema: Option<Identifier>,
    pub name: Identifier,
}

impl RelationName {
    pub fn to_sql(&self, quoting: Quoting) -> String {
        match &self.schema {
            Some(schema) => format!("{}.{}", schema.to_sql(quoting), self.name.to_sql(quoting)),
            None => self.name.to_sql(quoting),
        }
    }
}

/// A table resolved by the database: where it really lives, its columns, and its primary key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDescription {
    pub schema: String,
    pub name: String,
    /// In table order.
    pub columns: Vec<ColumnDescription>,
    /// In key order. Empty if the table has none.
    pub primary_key: Vec<String>,
}

/// One column of a described table, as the database reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDescription {
    pub name: String,
    /// As the database spells the type, which may differ from how it was written:
    /// `character varying(20)` for `varchar(20)` on PostgreSQL, `int` for `integer` on MySQL.
    pub data_type: String,
    pub nullable: bool,
    /// The default's expression as the database keeps it, e.g. `nextval('orders_id_seq'::regclass)`.
    pub default: Option<String>,
}

impl TableDescription {
    pub fn has_column(&self, name: &str) -> bool {
        self.columns.iter().any(|column| column.name == name)
    }

    pub fn column_names(&self) -> impl Iterator<Item = &str> {
        self.columns.iter().map(|column| column.name.as_str())
    }

    /// The table's name for generated SQL: schema-qualified and always quoted, so it means
    /// exactly this table whatever the search path and whatever the names spell.
    pub fn qualified_name(&self, quoting: Quoting) -> String {
        format!(
            "{}.{}",
            quoting.identifier(&self.schema),
            quoting.identifier(&self.name)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_names_as_they_were_written_and_quotes_generated_ones() {
        let name = RelationName {
            schema: Some(Identifier {
                name: "public".to_owned(),
                quoted: false,
            }),
            name: Identifier {
                name: "Order \"Items\"".to_owned(),
                quoted: true,
            },
        };
        assert_eq!(
            name.to_sql(Quoting::STANDARD),
            "public.\"Order \"\"Items\"\"\""
        );
        let described = TableDescription {
            schema: "public".to_owned(),
            name: "order".to_owned(),
            columns: vec![],
            primary_key: vec![],
        };
        assert_eq!(
            described.qualified_name(Quoting::STANDARD),
            "\"public\".\"order\""
        );
    }
}
