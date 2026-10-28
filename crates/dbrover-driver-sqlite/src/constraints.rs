//! A table's unique keys, foreign keys and checks, for changing the table. SQLite names none
//! of them in its catalog, and keeps checks only in the `CREATE TABLE` statement.

use dbrover_core::{
    CheckConstraint, DriverError, ForeignKeyConstraint, Quoting, ReferentialAction,
    TableConstraints, TableName, UniqueConstraint,
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::connection_error;

pub(crate) fn describe_constraints(
    connection: &Connection,
    table: &TableName,
) -> Result<TableConstraints, DriverError> {
    let definition: Option<String> = connection
        .query_row(
            &format!(
                "SELECT sql FROM {}.sqlite_schema WHERE type = 'table' AND name = ?1",
                Quoting::STANDARD.identifier(&table.schema)
            ),
            [&table.name],
            |row| row.get(0),
        )
        .optional()
        .map_err(connection_error)?
        .flatten();
    Ok(TableConstraints {
        primary_key_name: None,
        unique_keys: unique_keys(connection, table)?,
        foreign_keys: foreign_keys(connection, table)?,
        checks: definition.as_deref().map(checks).unwrap_or_default(),
        default_names: Default::default(),
    })
}

/// The indexes SQLite made for the table's `UNIQUE` constraints (origin `u`), each column list
/// in key order.
fn unique_keys(
    connection: &Connection,
    table: &TableName,
) -> Result<Vec<UniqueConstraint>, DriverError> {
    let mut statement = connection
        .prepare(
            "SELECT il.name, ii.name FROM pragma_index_list(?1, ?2) il \
             JOIN pragma_index_info(il.name, ?2) ii \
             WHERE il.origin = 'u' ORDER BY il.seq DESC, ii.seqno",
        )
        .map_err(connection_error)?;
    let columns = statement
        .query_map(params![table.name, table.schema], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(connection_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(connection_error)?;
    let mut keys: Vec<(String, UniqueConstraint)> = Vec::new();
    for (index, column) in columns {
        match keys.last_mut() {
            Some((last, key)) if *last == index => key.columns.push(column),
            _ => keys.push((
                index,
                UniqueConstraint {
                    name: None,
                    columns: vec![column],
                },
            )),
        }
    }
    Ok(keys.into_iter().map(|(_, key)| key).collect())
}

fn foreign_keys(
    connection: &Connection,
    table: &TableName,
) -> Result<Vec<ForeignKeyConstraint>, DriverError> {
    let mut statement = connection
        .prepare(
            "SELECT id, \"table\", \"from\", \"to\", on_update, on_delete \
             FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq",
        )
        .map_err(connection_error)?;
    type KeyColumn = (i64, String, String, Option<String>, String, String);
    let columns = statement
        .query_map(params![table.name, table.schema], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .map_err(connection_error)?
        .collect::<Result<Vec<KeyColumn>, _>>()
        .map_err(connection_error)?;
    let mut keys: Vec<(i64, ForeignKeyConstraint, bool)> = Vec::new();
    for (id, parent, from, to, on_update, on_delete) in columns {
        let implicit = to.is_none();
        match keys.last_mut() {
            Some((last, key, _)) if *last == id => {
                key.columns.push(from);
                key.referenced_columns.extend(to);
            }
            _ => keys.push((
                id,
                ForeignKeyConstraint {
                    name: None,
                    columns: vec![from],
                    referenced: TableName {
                        schema: table.schema.clone(),
                        name: parent,
                    },
                    referenced_columns: to.into_iter().collect(),
                    on_delete: ReferentialAction::from_catalog(&on_delete).unwrap_or_default(),
                    on_update: ReferentialAction::from_catalog(&on_update).unwrap_or_default(),
                },
                implicit,
            )),
        }
    }
    keys.into_iter()
        .map(|(_, mut key, implicit)| {
            // A key that names no columns refers to the parent's primary key.
            if implicit {
                key.referenced_columns = primary_key(connection, &key.referenced)?;
            }
            Ok(key)
        })
        .collect()
}

fn primary_key(connection: &Connection, table: &TableName) -> Result<Vec<String>, DriverError> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1, ?2) WHERE pk > 0 ORDER BY pk")
        .map_err(connection_error)?;
    statement
        .query_map(params![table.name, table.schema], |row| row.get(0))
        .map_err(connection_error)?
        .collect::<Result<_, _>>()
        .map_err(connection_error)
}

/// What the scanner sees of a `CREATE TABLE` statement: words (names unquoted), parentheses
/// at their byte offsets, and anything else (strings, operators) as one kind.
#[derive(Debug, PartialEq, Eq)]
enum Piece {
    Word(String),
    Open(usize),
    Close(usize),
    Other,
}

/// Every `CHECK (…)` in the table's column list, of a column or of the table, with the name
/// `CONSTRAINT` gives it.
fn checks(sql: &str) -> Vec<CheckConstraint> {
    let pieces = pieces(sql);
    let mut checks = Vec::new();
    let mut depth = 0usize;
    for (index, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Open(_) => depth += 1,
            Piece::Close(_) => depth = depth.saturating_sub(1),
            Piece::Word(word) if depth == 1 && word.eq_ignore_ascii_case("CHECK") => {
                let Some(Piece::Open(start)) = pieces.get(index + 1) else {
                    continue;
                };
                let Some(end) = matching_close(&pieces[index + 1..]) else {
                    continue;
                };
                let name = index
                    .checked_sub(2)
                    .and_then(|before| match &pieces[before..index] {
                        [Piece::Word(keyword), Piece::Word(name)]
                            if keyword.eq_ignore_ascii_case("CONSTRAINT") =>
                        {
                            Some(name.clone())
                        }
                        _ => None,
                    });
                checks.push(CheckConstraint {
                    name,
                    expression: sql[start + 1..end].trim().to_owned(),
                });
            }
            _ => {}
        }
    }
    checks
}

/// The byte offset of the parenthesis that closes the one `pieces` starts with.
fn matching_close(pieces: &[Piece]) -> Option<usize> {
    let mut depth = 0usize;
    for piece in pieces {
        match piece {
            Piece::Open(_) => depth += 1,
            Piece::Close(at) => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(*at);
                }
            }
            _ => {}
        }
    }
    None
}

fn pieces(sql: &str) -> Vec<Piece> {
    let bytes = sql.as_bytes();
    let mut pieces = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        let (piece, next) = match byte {
            b'(' => (Piece::Open(at), at + 1),
            b')' => (Piece::Close(at), at + 1),
            b'\'' => (Piece::Other, quoted_end(bytes, at, b'\'')),
            b'"' | b'`' => {
                let end = quoted_end(bytes, at, byte);
                let doubled = (byte as char).to_string().repeat(2);
                let text = sql[at + 1..end.saturating_sub(1).max(at + 1)]
                    .replace(&doubled, &(byte as char).to_string());
                (Piece::Word(text), end)
            }
            b'[' => {
                let end = sql[at..].find(']').map_or(bytes.len(), |end| at + end + 1);
                (Piece::Word(sql[at + 1..end - 1].to_owned()), end)
            }
            b'-' if bytes.get(at + 1) == Some(&b'-') => {
                let end = sql[at..].find('\n').map_or(bytes.len(), |end| at + end);
                at = end;
                continue;
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                at = sql[at + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |end| at + end + 4);
                continue;
            }
            _ if byte.is_ascii_whitespace() => {
                at += 1;
                continue;
            }
            _ if byte.is_ascii_alphanumeric() || byte == b'_' || !byte.is_ascii() => {
                let end = sql[at..]
                    .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                    .map_or(bytes.len(), |end| at + end);
                (Piece::Word(sql[at..end].to_owned()), end)
            }
            _ => (Piece::Other, at + 1),
        };
        pieces.push(piece);
        at = next;
    }
    pieces
}

/// The offset just past the quote that closes the one at `start`, a doubled quote being one
/// inside the text.
fn quoted_end(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut at = start + 1;
    while at < bytes.len() {
        if bytes[at] == quote {
            if bytes.get(at + 1) == Some(&quote) {
                at += 2;
                continue;
            }
            return at + 1;
        }
        at += 1;
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_column_and_table_checks_with_their_names() {
        let sql = "CREATE TABLE \"t\" (\n\
                   id INTEGER PRIMARY KEY, -- a comment with CHECK (no)\n\
                   status TEXT CHECK (status IN ('a', 'b)')),\n\
                   total INT CONSTRAINT [positive total] CHECK(total >= 0),\n\
                   /* CHECK (hidden) */ CONSTRAINT \"Both\" CHECK ((total > 0) OR status = 'a')\n\
                   )";
        let checks = checks(sql);
        let found: Vec<(Option<&str>, &str)> = checks
            .iter()
            .map(|check| (check.name.as_deref(), check.expression.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (None, "status IN ('a', 'b)')"),
                (Some("positive total"), "total >= 0"),
                (Some("Both"), "(total > 0) OR status = 'a'"),
            ]
        );
    }
}
