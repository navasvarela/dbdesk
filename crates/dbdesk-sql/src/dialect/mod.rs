//! Everything that differs between SQL dialects, in one place: the lexer's rules, the
//! `sqlparser` dialect, the built-in functions, the `EXPLAIN` prefix, quoting and the
//! catalog's system names.
//!
//! The rest of the crate asks a [`Dialect`] for these facts instead of matching on it, so a
//! new dialect is a new variant here, its rows in each method and, if its lexical rules are
//! new, the lexer code that applies them.

mod functions;

use dbdesk_core::{DriverKind, Quoting};
use sqlparser::dialect::{MySqlDialect, PostgreSqlDialect, SQLiteDialect};

/// The SQL flavour of a data source.
///
/// A closed enum, like `DriverKind`: see `docs/adr/0002-native-drivers-behind-a-trait.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    Sqlite,
    /// MySQL and MariaDB, which one driver serves; their differences depend on the server's
    /// version, which only a connection knows.
    Mysql,
}

/// The lexical rules that differ between dialects. Strings in `'…'`, `--` line comments and
/// `/* … */` block comments are common to every dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LexicalRules {
    /// `E'…'` strings, in which a backslash escapes the next character.
    pub escape_strings: bool,
    /// Whether a backslash escapes the next character in every `'…'` (and `"…"`) string, as
    /// in MySQL's default SQL mode.
    pub backslash_escapes: bool,
    /// Whether `"…"` is a string, as in MySQL without `ANSI_QUOTES`, rather than an
    /// identifier.
    pub double_quoted_strings: bool,
    /// `$tag$ … $tag$` strings.
    pub dollar_quoting: bool,
    /// `$` inside identifiers after their first character.
    pub dollar_in_identifiers: bool,
    /// Whether `/* /* */ */` is one comment, rather than ending at the first `*/`.
    pub nested_block_comments: bool,
    /// `# …` line comments.
    pub hash_comments: bool,
    /// Whether `--` starts a comment only before whitespace, as in MySQL, where `1--1` is
    /// arithmetic.
    pub dash_comments_need_space: bool,
    /// `` `backtick` `` identifiers.
    pub backtick_identifiers: bool,
    /// `[bracketed]` identifiers.
    pub bracket_identifiers: bool,
}

/// A construct `sqlparser` rejects although the dialect's database accepts it. A statement
/// with one isn't syntax-checked, because a false error is worse than a missed one; the
/// check that finds each kind is in `check::syntax`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParserGap {
    /// A sequence of words, compared lowercase.
    Words(&'static [&'static str]),
    /// PostgreSQL's XML functions, which take keyword arguments: `xmlelement(name foo)`.
    XmlFunctionArguments,
    /// `JOIN t USING (id) AS j` (PostgreSQL 14), which names the join's columns.
    JoinUsingAlias,
    /// `a IS b` with any expression after `IS`, which SQLite accepts.
    IsAnyExpression,
    /// A keyword used as an infix operator, `a MOD 3`, rather than as the function of the
    /// same name, `MOD(a, 3)`.
    InfixOperator(&'static str),
}

impl Dialect {
    pub const ALL: [Self; 3] = [Self::Postgres, Self::Sqlite, Self::Mysql];

    pub(crate) fn lexical_rules(self) -> LexicalRules {
        match self {
            Self::Postgres => LexicalRules {
                escape_strings: true,
                backslash_escapes: false,
                double_quoted_strings: false,
                dollar_quoting: true,
                dollar_in_identifiers: true,
                nested_block_comments: true,
                hash_comments: false,
                dash_comments_need_space: false,
                backtick_identifiers: false,
                bracket_identifiers: false,
            },
            Self::Sqlite => LexicalRules {
                escape_strings: false,
                backslash_escapes: false,
                double_quoted_strings: false,
                dollar_quoting: false,
                dollar_in_identifiers: false,
                nested_block_comments: false,
                hash_comments: false,
                dash_comments_need_space: false,
                backtick_identifiers: true,
                bracket_identifiers: true,
            },
            Self::Mysql => LexicalRules {
                escape_strings: false,
                backslash_escapes: true,
                double_quoted_strings: true,
                dollar_quoting: false,
                dollar_in_identifiers: true,
                nested_block_comments: false,
                hash_comments: true,
                dash_comments_need_space: true,
                backtick_identifiers: true,
                bracket_identifiers: false,
            },
        }
    }

    /// Whether scripts may change the statement terminator with the client-side
    /// `DELIMITER //` command, as the `mysql` client and every MySQL tool accept, so that a
    /// procedure's body can hold semicolons.
    pub(crate) fn has_delimiter_command(self) -> bool {
        match self {
            Self::Postgres | Self::Sqlite => false,
            Self::Mysql => true,
        }
    }

    /// The dialect `sqlparser` parses this one with, for the syntax check and for reading the
    /// columns of a query.
    pub(crate) fn sqlparser_dialect(self) -> &'static dyn sqlparser::dialect::Dialect {
        match self {
            Self::Postgres => &PostgreSqlDialect {},
            Self::Sqlite => &SQLiteDialect {},
            Self::Mysql => &MySqlDialect {},
        }
    }

    /// What `sqlparser` 0.63 rejects in valid statements of this dialect. The list came from
    /// running a corpus of valid statements through the parser (`check::tests`); a statement
    /// the parser wrongly rejects is a bug to add here.
    pub(crate) fn parser_gaps(self) -> &'static [ParserGap] {
        match self {
            Self::Postgres => &[
                // `SELECT * FROM ROWS FROM (f(), g())`
                ParserGap::Words(&["rows", "from"]),
                // `FOR NO KEY UPDATE`, `FOR KEY SHARE`
                ParserGap::Words(&["no", "key"]),
                ParserGap::Words(&["key", "share"]),
                ParserGap::Words(&["between", "symmetric"]),
                ParserGap::XmlFunctionArguments,
                ParserGap::JoinUsingAlias,
            ],
            Self::Sqlite => &[
                // `FROM t INDEXED BY i`, `FROM t NOT INDEXED`
                ParserGap::Words(&["indexed", "by"]),
                ParserGap::Words(&["not", "indexed"]),
                ParserGap::IsAnyExpression,
            ],
            Self::Mysql => &[
                ParserGap::Words(&["sounds", "like"]),
                ParserGap::Words(&["lock", "in", "share", "mode"]),
                // `UPDATE LOW_PRIORITY IGNORE t`, `DELETE QUICK IGNORE FROM t`
                ParserGap::Words(&["low_priority", "ignore"]),
                ParserGap::Words(&["quick", "ignore"]),
                ParserGap::InfixOperator("mod"),
            ],
        }
    }

    /// First words of statements that insert rows: `insert`, and SQLite's and MySQL's
    /// `replace` (which deletes a row with the same key first).
    pub(crate) fn insert_verbs(self) -> &'static [&'static str] {
        match self {
            Self::Postgres => &["insert"],
            Self::Sqlite | Self::Mysql => &["insert", "replace"],
        }
    }

    /// What to put before a statement to have the database describe how it would run it,
    /// without running it: `EXPLAIN` on PostgreSQL, `EXPLAIN QUERY PLAN` on SQLite (whose
    /// plain `EXPLAIN` lists virtual machine opcodes instead), and `EXPLAIN` on MySQL and
    /// MariaDB, which both accept it.
    pub fn explain_prefix(self) -> &'static str {
        match self {
            Self::Postgres | Self::Mysql => "EXPLAIN ",
            Self::Sqlite => "EXPLAIN QUERY PLAN ",
        }
    }

    /// The built-in functions completion offers: the common ones, not every function the
    /// database has (the explorer's catalog adds the database's own). Lowercase and sorted.
    pub fn builtin_functions(self) -> &'static [&'static str] {
        match self {
            Self::Postgres => functions::POSTGRES,
            Self::Sqlite => functions::SQLITE,
            Self::Mysql => functions::MYSQL,
        }
    }

    /// How SQL that dbdesk writes (qualified names, completions, grid edits) quotes
    /// identifiers and strings.
    pub fn quoting(self) -> Quoting {
        match self {
            Self::Postgres | Self::Sqlite => Quoting::STANDARD,
            Self::Mysql => Quoting {
                identifier_quote: '`',
                backslash_escapes: true,
            },
        }
    }

    /// Whether a table name belongs to the system catalog, which the explorer doesn't list:
    /// PostgreSQL's `pg_*` tables (always on the search path) and its catalog schemas,
    /// SQLite's `sqlite_*` tables, `pragma_*` functions and `temp` schema, MySQL's own
    /// databases. Names are compared case-insensitively.
    pub fn is_system_name(self, schema: Option<&str>, table: &str) -> bool {
        let (table_prefixes, schemas): (&[&str], &[&str]) = match self {
            Self::Postgres => (&["pg_"], &["pg_catalog", "information_schema", "pg_toast"]),
            Self::Sqlite => (&["sqlite_", "pragma_"], &["temp"]),
            Self::Mysql => (
                &[],
                &["mysql", "information_schema", "performance_schema", "sys"],
            ),
        };
        let table = table.to_ascii_lowercase();
        table_prefixes
            .iter()
            .any(|prefix| table.starts_with(prefix))
            || schema.is_some_and(|schema| {
                schemas
                    .iter()
                    .any(|system| schema.eq_ignore_ascii_case(system))
            })
    }
}

/// The dialect each engine speaks.
impl From<DriverKind> for Dialect {
    fn from(driver: DriverKind) -> Self {
        match driver {
            DriverKind::Postgres => Self::Postgres,
            DriverKind::Sqlite => Self::Sqlite,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_system_names_case_insensitively() {
        let cases = [
            (Dialect::Postgres, None, "pg_class", true),
            (
                Dialect::Postgres,
                Some("Information_Schema"),
                "tables",
                true,
            ),
            (Dialect::Postgres, Some("public"), "orders", false),
            (Dialect::Sqlite, None, "SQLITE_master", true),
            (Dialect::Sqlite, None, "pragma_table_info", true),
            (Dialect::Sqlite, Some("temp"), "scratch", true),
            (Dialect::Sqlite, Some("main"), "orders", false),
            (Dialect::Mysql, Some("Performance_Schema"), "threads", true),
            (Dialect::Mysql, Some("shop"), "orders", false),
            (Dialect::Mysql, None, "orders", false),
        ];
        for (dialect, schema, table, expected) in cases {
            assert_eq!(
                dialect.is_system_name(schema, table),
                expected,
                "{dialect:?} {schema:?}.{table}"
            );
        }
    }

    #[test]
    fn every_driver_has_a_dialect() {
        let dialects: Vec<Dialect> = DriverKind::ALL.into_iter().map(Dialect::from).collect();
        assert_eq!(dialects, [Dialect::Postgres, Dialect::Sqlite]);
    }
}
