//! Tables as a statement names them, and what editing their rows needs to know.

/// A name as written in SQL. Whether it was quoted matters: PostgreSQL folds unquoted names
/// to lowercase, and SQLite matches them ignoring case.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identifier {
    pub name: String,
    pub quoted: bool,
}

impl Identifier {
    /// The identifier as SQL text: quoted ones in double quotes, with quotes doubled.
    pub fn to_sql(&self) -> String {
        if self.quoted {
            quote_always(&self.name)
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
    pub fn to_sql(&self) -> String {
        match &self.schema {
            Some(schema) => format!("{}.{}", schema.to_sql(), self.name.to_sql()),
            None => self.name.to_sql(),
        }
    }
}

/// A table resolved by the database: where it really lives, its columns, and its primary key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDescription {
    pub schema: String,
    pub name: String,
    /// In table order.
    pub columns: Vec<String>,
    /// In key order. Empty if the table has none.
    pub primary_key: Vec<String>,
}

impl TableDescription {
    /// The table's name for generated SQL: schema-qualified and always quoted, so it means
    /// exactly this table whatever the search path and whatever the names spell.
    pub fn qualified_name(&self) -> String {
        format!(
            "{}.{}",
            quote_always(&self.schema),
            quote_always(&self.name)
        )
    }
}

/// Double-quotes an identifier, doubling quotes inside, even where it needs no quoting, as
/// generated statements must not depend on whether a name happens to be a keyword.
pub fn quote_always(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
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
        assert_eq!(name.to_sql(), "public.\"Order \"\"Items\"\"\"");
        let described = TableDescription {
            schema: "public".to_owned(),
            name: "order".to_owned(),
            columns: vec![],
            primary_key: vec![],
        };
        assert_eq!(described.qualified_name(), "\"public\".\"order\"");
    }
}
