//! How SQL that dbdesk writes itself quotes identifiers and strings, which differs between
//! dialects: MySQL reads `"x"` as a string and `\` inside a string as an escape, and SQL
//! Server reads `"x"` as an identifier only while the session's `QUOTED_IDENTIFIER` is on.

/// The quoting rules of one dialect, for generated SQL. `dbdesk-sql`'s `Dialect::quoting`
/// says which applies to a data source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quoting {
    /// Opens a quoted identifier.
    pub identifier_open: char,
    /// Closes a quoted identifier; doubled inside one to stand for itself. The same as
    /// `identifier_open` except for SQL Server's `[brackets]`.
    pub identifier_close: char,
    /// Whether a backslash inside a string literal escapes the next character, so a literal
    /// backslash has to be written twice.
    pub backslash_escapes: bool,
    /// Written before a string literal's opening quote: `N` on SQL Server, where a plain
    /// `'…'` is read in the database's code page and loses the characters it lacks.
    pub string_prefix: &'static str,
}

impl Quoting {
    /// Standard SQL, as PostgreSQL and SQLite read it: `"identifier"`, `'it''s'`.
    pub const STANDARD: Self = Self {
        identifier_open: '"',
        identifier_close: '"',
        backslash_escapes: false,
        string_prefix: "",
    };

    /// Quotes `name` even where it needs no quoting, as generated statements must not depend
    /// on whether a name happens to be a keyword.
    pub fn identifier(self, name: &str) -> String {
        let (open, close) = (self.identifier_open, self.identifier_close);
        let doubled = format!("{close}{close}");
        format!("{open}{}{close}", name.replace(close, &doubled))
    }

    /// Quotes `name` unless it is a plain lowercase identifier, which reads the same quoted or
    /// not in every dialect (PostgreSQL folds unquoted names to lowercase). For text a person
    /// reads and edits, such as completions.
    pub fn identifier_if_needed(self, name: &str) -> String {
        let mut chars = name.chars();
        let plain = chars
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if plain {
            name.to_owned()
        } else {
            self.identifier(name)
        }
    }

    /// `text` as a string literal.
    pub fn string_literal(self, text: &str) -> String {
        let escaped = if self.backslash_escapes {
            text.replace('\\', "\\\\")
        } else {
            text.to_owned()
        };
        format!("{}'{}'", self.string_prefix, escaped.replace('\'', "''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BACKTICKS: Quoting = Quoting {
        identifier_open: '`',
        identifier_close: '`',
        backslash_escapes: true,
        string_prefix: "",
    };

    const BRACKETS: Quoting = Quoting {
        identifier_open: '[',
        identifier_close: ']',
        backslash_escapes: false,
        string_prefix: "N",
    };

    #[test]
    fn quotes_identifiers_doubling_the_quote_inside() {
        let cases = [
            (Quoting::STANDARD, "users", "\"users\"", "users"),
            (
                Quoting::STANDARD,
                "Order \"Items\"",
                "\"Order \"\"Items\"\"\"",
                "\"Order \"\"Items\"\"\"",
            ),
            (BACKTICKS, "order`s", "`order``s`", "`order``s`"),
            (BACKTICKS, "Users", "`Users`", "`Users`"),
            (BACKTICKS, "users_2", "`users_2`", "users_2"),
            (BRACKETS, "a]b[c", "[a]]b[c]", "[a]]b[c]"),
            (BRACKETS, "order", "[order]", "order"),
        ];
        for (quoting, name, always, if_needed) in cases {
            assert_eq!(quoting.identifier(name), always, "{name}");
            assert_eq!(quoting.identifier_if_needed(name), if_needed, "{name}");
        }
    }

    #[test]
    fn escapes_backslashes_only_where_they_are_escapes() {
        let text = r"it's C:\temp";

        assert_eq!(Quoting::STANDARD.string_literal(text), r"'it''s C:\temp'");
        assert_eq!(BACKTICKS.string_literal(text), r"'it''s C:\\temp'");
        assert_eq!(BRACKETS.string_literal("José"), "N'José'");
    }
}
