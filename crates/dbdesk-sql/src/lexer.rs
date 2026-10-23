use std::ops::Range;

/// The SQL flavour, which decides a few lexical rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// Dollar-quoted strings (`$body$ … $body$`), nested block comments, `E'…'` strings.
    Postgres,
    /// `[bracketed]` and `` `backtick` `` identifiers.
    Sqlite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Whitespace,
    LineComment,
    BlockComment,
    /// A string literal, including dollar-quoted bodies.
    String,
    QuotedIdentifier,
    Number,
    /// A keyword or an unquoted identifier; see [`is_keyword`](crate::is_keyword).
    Word,
    Semicolon,
    /// Any other single character: operators, parentheses, commas, `$1` markers…
    Punctuation,
}

impl TokenKind {
    /// Whitespace and comments: tokens with no meaning to the database.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Whitespace | Self::LineComment | Self::BlockComment
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub range: Range<usize>,
}

/// Splits `sql` into tokens that together cover every byte of it, in order.
pub fn tokenize(sql: &str, dialect: Dialect) -> Vec<Token> {
    let mut lexer = Lexer {
        sql,
        position: 0,
        dialect,
    };
    let mut tokens = Vec::new();
    while lexer.position < sql.len() {
        let start = lexer.position;
        let kind = lexer.next_kind();
        tokens.push(Token {
            kind,
            range: start..lexer.position,
        });
    }
    tokens
}

/// The statement's first word, lowercased, skipping comments: usually its verb (`select`,
/// `update`, `create`, …).
pub fn first_word(sql: &str, dialect: Dialect) -> Option<String> {
    tokenize(sql, dialect)
        .into_iter()
        .find(|token| !token.kind.is_trivia())
        .filter(|token| token.kind == TokenKind::Word)
        .map(|token| sql[token.range].to_ascii_lowercase())
}

struct Lexer<'a> {
    sql: &'a str,
    position: usize,
    dialect: Dialect,
}

impl Lexer<'_> {
    fn rest(&self) -> &str {
        &self.sql[self.position..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_second(&self) -> Option<char> {
        self.rest().chars().nth(1)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.position += c.len_utf8();
        Some(c)
    }

    fn bump_while(&mut self, predicate: impl Fn(char) -> bool) {
        while self.peek().is_some_and(&predicate) {
            self.bump();
        }
    }

    /// Consumes one token, which always advances by at least one character.
    fn next_kind(&mut self) -> TokenKind {
        let Some(first) = self.peek() else {
            return TokenKind::Whitespace;
        };
        let second = self.peek_second();
        match (first, second) {
            (c, _) if c.is_whitespace() => {
                self.bump_while(char::is_whitespace);
                TokenKind::Whitespace
            }
            ('-', Some('-')) => {
                self.bump_while(|c| c != '\n');
                TokenKind::LineComment
            }
            ('/', Some('*')) => self.block_comment(),
            ('\'', _) => self.quoted('\'', TokenKind::String),
            ('e' | 'E', Some('\'')) if self.dialect == Dialect::Postgres => {
                self.bump();
                self.escape_string()
            }
            ('"', _) => self.quoted('"', TokenKind::QuotedIdentifier),
            ('`', _) if self.dialect == Dialect::Sqlite => {
                self.quoted('`', TokenKind::QuotedIdentifier)
            }
            ('[', _) if self.dialect == Dialect::Sqlite => {
                self.bump();
                self.bump_while(|c| c != ']');
                self.bump();
                TokenKind::QuotedIdentifier
            }
            ('$', _) if self.dialect == Dialect::Postgres => self.dollar(),
            (c, _) if c.is_ascii_digit() => self.number(),
            ('.', Some(c)) if c.is_ascii_digit() => self.number(),
            (c, _) if is_word_start(c) => {
                let dialect = self.dialect;
                self.bump();
                self.bump_while(|c| is_word_part(c, dialect));
                TokenKind::Word
            }
            (';', _) => {
                self.bump();
                TokenKind::Semicolon
            }
            _ => {
                self.bump();
                TokenKind::Punctuation
            }
        }
    }

    /// `/* … */`. PostgreSQL nests block comments; SQLite ends at the first `*/`.
    fn block_comment(&mut self) -> TokenKind {
        self.position += 2;
        let mut depth = 1;
        while depth > 0 {
            if self.rest().starts_with("*/") {
                self.position += 2;
                depth -= 1;
            } else if self.dialect == Dialect::Postgres && self.rest().starts_with("/*") {
                self.position += 2;
                depth += 1;
            } else if self.bump().is_none() {
                break;
            }
        }
        TokenKind::BlockComment
    }

    /// Text between `quote`s, where a doubled quote stands for itself (`'it''s'`).
    fn quoted(&mut self, quote: char, kind: TokenKind) -> TokenKind {
        self.bump();
        while let Some(c) = self.bump() {
            if c == quote {
                if self.peek() == Some(quote) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        kind
    }

    /// PostgreSQL `E'…'`, where a backslash escapes the next character, quotes included.
    fn escape_string(&mut self) -> TokenKind {
        self.bump();
        while let Some(c) = self.bump() {
            match c {
                '\\' => {
                    self.bump();
                }
                '\'' if self.peek() == Some('\'') => {
                    self.bump();
                }
                '\'' => break,
                _ => {}
            }
        }
        TokenKind::String
    }

    /// `$tag$ … $tag$` (dollar quoting) or `$1` (a parameter marker).
    fn dollar(&mut self) -> TokenKind {
        let tag_length = self.rest()[1..]
            .find(|c: char| !is_word_part(c, Dialect::Sqlite))
            .unwrap_or(self.rest().len() - 1);
        let tag = &self.rest()[..tag_length + 1];
        let opens_quote = self.rest()[tag_length + 1..].starts_with('$')
            && !tag[1..].starts_with(|c: char| c.is_ascii_digit());
        if !opens_quote {
            self.bump();
            return TokenKind::Punctuation;
        }
        let delimiter = format!("{tag}$");
        self.position += delimiter.len();
        match self.rest().find(&delimiter) {
            Some(end) => self.position += end + delimiter.len(),
            None => self.position = self.sql.len(),
        }
        TokenKind::String
    }

    fn number(&mut self) -> TokenKind {
        self.bump();
        self.bump_while(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
        TokenKind::Number
    }
}

fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_word_part(c: char, dialect: Dialect) -> bool {
    c.is_alphanumeric() || c == '_' || (c == '$' && dialect == Dialect::Postgres)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(sql: &str, dialect: Dialect) -> Vec<(TokenKind, &str)> {
        tokenize(sql, dialect)
            .into_iter()
            .filter(|token| token.kind != TokenKind::Whitespace)
            .map(|token| (token.kind, &sql[token.range]))
            .collect()
    }

    #[test]
    fn covers_the_input_without_gaps() {
        let sql = "SELECT 'a;b', \"x\"\"y\" -- c\n/* d */ FROM t WHERE é = $1; $$ body; $$";
        let tokens = tokenize(sql, Dialect::Postgres);

        let mut expected_start = 0;
        for token in &tokens {
            assert_eq!(token.range.start, expected_start);
            expected_start = token.range.end;
        }
        assert_eq!(expected_start, sql.len());
    }

    /// Input, dialect, and the expected non-whitespace tokens.
    type Case = (&'static str, Dialect, Vec<(TokenKind, &'static str)>);

    #[test]
    fn recognises_dialect_specific_quoting() {
        use TokenKind::*;
        let cases: [Case; 6] = [
            (
                "$fn$ a; $x$ b $fn$ ;",
                Dialect::Postgres,
                vec![(String, "$fn$ a; $x$ b $fn$"), (Semicolon, ";")],
            ),
            (
                "$1 + $2",
                Dialect::Postgres,
                vec![
                    (Punctuation, "$"),
                    (Number, "1"),
                    (Punctuation, "+"),
                    (Punctuation, "$"),
                    (Number, "2"),
                ],
            ),
            (
                r"E'it\'s' x",
                Dialect::Postgres,
                vec![(String, r"E'it\'s'"), (Word, "x")],
            ),
            (
                "/* a /* b */ c */ d",
                Dialect::Postgres,
                vec![(BlockComment, "/* a /* b */ c */"), (Word, "d")],
            ),
            (
                "[my col] `x` ;",
                Dialect::Sqlite,
                vec![
                    (QuotedIdentifier, "[my col]"),
                    (QuotedIdentifier, "`x`"),
                    (Semicolon, ";"),
                ],
            ),
            (
                "'unterminated; string",
                Dialect::Sqlite,
                vec![(String, "'unterminated; string")],
            ),
        ];

        for (sql, dialect, expected) in cases {
            assert_eq!(kinds(sql, dialect), expected, "{sql}");
        }
    }
}
