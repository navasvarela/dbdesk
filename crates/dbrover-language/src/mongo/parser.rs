//! Reads one command of the shell's syntax into a [`Command`], or says what in it DbRover
//! doesn't run, and where.
//!
//! The grammar, informally:
//!
//! ```text
//! command   = "use" name | "show" ("dbs" | "databases" | "collections" | "tables") | call
//! call      = "db" target? "." name arguments ("." name arguments)*
//! target    = ("." name)+                          a collection, dots allowed in its name
//!           | "." "getCollection" "(" string ")"
//!           | "." "getSiblingDB" "(" string ")" target?
//! arguments = "(" (value ("," value)* ","?)? ")"
//! value     = object | array | string | number | regex | true | false | null | undefined
//!           | "new"? constructor arguments | "MinKey" | "MaxKey"
//! ```

use std::fmt;
use std::ops::Range;

use dbrover_sql::{Token, TokenKind};

use super::lexer::tokenize;
use super::methods::{self, Receiver};
use super::syntax::{Call, Command, Constructor, MethodCall, Show, Value};

/// Why a command can't be read, with where in it the trouble is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    /// The bytes of the command the message is about.
    pub range: Range<usize>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

/// JavaScript's words for what DbRover doesn't run: variables, functions, loops and the like.
const SCRIPT_WORDS: &[&str] = &[
    "async", "await", "break", "class", "const", "do", "for", "function", "if", "let", "return",
    "switch", "throw", "try", "var", "while",
];

/// What the error for a script says it can't do, the same wherever it is found.
const ONE_COMMAND: &str =
    "DbRover runs one shell command at a time, without variables, functions or loops";

/// Reads `command`, one command as [`super::split`] gives them.
///
/// # Errors
///
/// [`ParseError`] for text that isn't a command DbRover runs, naming what isn't supported.
pub fn parse(command: &str) -> Result<Command, ParseError> {
    let mut parser = Parser::new(command);
    let parsed = parser.command()?;
    match parser.peek() {
        None => Ok(parsed),
        Some(token) => Err(parser.error(
            token.range.clone(),
            format!(
                "unexpected \u{201c}{}\u{201d} after the command",
                parser.text(token)
            ),
        )),
    }
}

/// Reads `text` as one value of the shell's syntax, whole: what is typed over a document's
/// field.
///
/// # Errors
///
/// [`ParseError`] for text that isn't one value.
pub(crate) fn parse_value(text: &str) -> Result<Value, ParseError> {
    let mut parser = Parser::new(text);
    let value = parser.value()?;
    match parser.peek() {
        None => Ok(value),
        Some(token) => Err(parser.error(
            token.range.clone(),
            format!(
                "unexpected \u{201c}{}\u{201d} after the value",
                parser.text(token)
            ),
        )),
    }
}

struct Parser<'a> {
    source: &'a str,
    /// The command's tokens, comments and whitespace left out.
    tokens: Vec<Token>,
    next: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        let tokens = tokenize(source)
            .into_iter()
            .filter(|token| !token.kind.is_trivia())
            .collect();
        Self {
            source,
            tokens,
            next: 0,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.next)
    }

    fn peek_text(&self) -> Option<&'a str> {
        self.tokens
            .get(self.next)
            .map(|token| &self.source[token.range.clone()])
    }

    fn peek_second_text(&self) -> Option<&'a str> {
        self.tokens
            .get(self.next + 1)
            .map(|token| &self.source[token.range.clone()])
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.next).cloned();
        self.next += usize::from(token.is_some());
        token
    }

    fn text(&self, token: &Token) -> &'a str {
        &self.source[token.range.clone()]
    }

    fn at_punctuation(&self, punctuation: &str) -> bool {
        self.peek()
            .is_some_and(|token| token.kind == TokenKind::Punctuation)
            && self.peek_text() == Some(punctuation)
    }

    fn error(&self, range: Range<usize>, message: impl Into<String>) -> ParseError {
        ParseError {
            message: message.into(),
            range,
        }
    }

    /// Where the text ends, for what is missing at its end.
    fn end(&self) -> Range<usize> {
        self.source.len()..self.source.len()
    }

    /// The next token, which must be `punctuation`; `what` names it in the error.
    fn expect(&mut self, punctuation: &str, what: &str) -> Result<Token, ParseError> {
        if self.at_punctuation(punctuation) {
            return self.bump().ok_or_else(|| self.error(self.end(), what));
        }
        Err(match self.peek() {
            Some(token) => self.error(
                token.range.clone(),
                format!(
                    "expected {what}, found \u{201c}{}\u{201d}",
                    self.text(token)
                ),
            ),
            None => self.error(self.end(), format!("expected {what}")),
        })
    }

    /// The next token, which must be a name.
    fn name(&mut self, what: &str) -> Result<(String, Range<usize>), ParseError> {
        match self.peek() {
            Some(token) if token.kind == TokenKind::Word => {
                let token = token.clone();
                self.next += 1;
                Ok((self.text(&token).to_owned(), token.range))
            }
            Some(token) => Err(self.error(
                token.range.clone(),
                format!(
                    "expected {what}, found \u{201c}{}\u{201d}",
                    self.text(token)
                ),
            )),
            None => Err(self.error(self.end(), format!("expected {what}"))),
        }
    }

    fn command(&mut self) -> Result<Command, ParseError> {
        let Some(first) = self.peek().cloned() else {
            return Err(self.error(self.end(), "there is no command"));
        };
        let word = self.text(&first);
        match (first.kind, word) {
            (TokenKind::Word, "use") => self.use_database(&first),
            (TokenKind::Word, "show") => self.show(&first),
            (TokenKind::Word, "db") => self.call().map(Command::Call),
            (TokenKind::Word, word) if SCRIPT_WORDS.contains(&word) => Err(self.error(
                first.range,
                format!("\u{201c}{word}\u{201d} isn't supported: {ONE_COMMAND}"),
            )),
            _ => Err(self.error(
                first.range,
                format!(
                    "\u{201c}{word}\u{201d} isn't supported: a command starts with db., use or show"
                ),
            )),
        }
    }

    /// `use <database>`. A database's name may hold characters a name token doesn't, such as
    /// `-`, so it is the rest of the command as written, unquoted if quoted.
    fn use_database(&mut self, keyword: &Token) -> Result<Command, ParseError> {
        self.next = self.tokens.len();
        let rest = self.source[keyword.range.end..].trim();
        let name = unquote(rest).unwrap_or(rest);
        if name.is_empty() {
            return Err(self.error(keyword.range.clone(), "use needs a database's name"));
        }
        if name.contains(char::is_whitespace) {
            let start =
                keyword.range.end + self.source[keyword.range.end..].find(rest).unwrap_or(0);
            return Err(self.error(
                start..start + rest.len(),
                "a database's name can't hold spaces",
            ));
        }
        Ok(Command::Use(name.to_owned()))
    }

    fn show(&mut self, keyword: &Token) -> Result<Command, ParseError> {
        self.bump();
        let Some(what) = self.bump() else {
            return Err(self.error(
                keyword.range.clone(),
                "show needs what to list: dbs or collections",
            ));
        };
        match self.text(&what) {
            "dbs" | "databases" => Ok(Command::Show(Show::Databases)),
            "collections" | "tables" => Ok(Command::Show(Show::Collections)),
            other => Err(self.error(
                what.range,
                format!("show {other} isn't supported: DbRover runs show dbs and show collections"),
            )),
        }
    }

    /// `db.…`: the target, its method and the cursor methods chained after it.
    fn call(&mut self) -> Result<Call, ParseError> {
        self.bump();
        let mut database = None;
        let mut collection: Vec<String> = Vec::new();
        let (method, range) = loop {
            self.expect(".", "\u{201c}.\u{201d} and a collection or method")?;
            let (name, range) = self.name("a collection or method")?;
            if !self.at_punctuation("(") {
                collection.push(name);
                continue;
            }
            match name.as_str() {
                "getCollection" if collection.is_empty() => {
                    collection.push(self.single_string(&range, "getCollection")?);
                }
                "getSiblingDB" | "getSisterDB" if collection.is_empty() && database.is_none() => {
                    database = Some(self.single_string(&range, &name)?);
                }
                _ => break (name, range),
            }
        };
        let receiver = if collection.is_empty() {
            Receiver::Database
        } else {
            Receiver::Collection
        };
        let spec = methods::method(receiver, &method)
            .ok_or_else(|| self.unknown_method(receiver, &method, range.clone()))?;
        let arguments = self.arguments()?;
        check_count(&arguments, spec.min_arguments, spec.max_arguments)
            .map_err(|message| self.error(range.clone(), format!("{method}() {message}")))?;
        let mut chain = Vec::new();
        while self.at_punctuation(".") {
            self.bump();
            let (name, range) = self.name("a cursor method")?;
            if !spec.cursor.contains(&name.as_str()) {
                return Err(self.error(
                    range,
                    format!("{name}() can't follow {method}() in DbRover"),
                ));
            }
            if chain
                .iter()
                .any(|chained: &MethodCall| chained.name == "explain")
            {
                return Err(self.error(range, "explain() must come last"));
            }
            let arguments = self.arguments()?;
            if let Some(cursor) = methods::cursor_method_named(&name) {
                check_count(&arguments, cursor.min_arguments, cursor.max_arguments)
                    .map_err(|message| self.error(range.clone(), format!("{name}() {message}")))?;
            }
            chain.push(MethodCall {
                name,
                arguments,
                range,
            });
        }
        Ok(Call {
            database,
            collection: (!collection.is_empty()).then(|| collection.join(".")),
            method: MethodCall {
                name: method,
                arguments,
                range,
            },
            chain,
        })
    }

    fn unknown_method(&self, receiver: Receiver, method: &str, range: Range<usize>) -> ParseError {
        let message = match receiver {
            Receiver::Collection => format!(
                "{method}() isn't a collection method DbRover runs; db.runCommand({{…}}) sends any command"
            ),
            Receiver::Database => format!(
                "{method}() isn't a database method DbRover runs; db.runCommand({{…}}) sends any command"
            ),
        };
        self.error(range, message)
    }

    /// The one string argument of `getCollection("…")` or `getSiblingDB("…")`.
    fn single_string(&mut self, range: &Range<usize>, method: &str) -> Result<String, ParseError> {
        match self.arguments()?.as_slice() {
            [Value::String(name)] if !name.is_empty() => Ok(name.clone()),
            _ => Err(self.error(
                range.clone(),
                format!("{method}() takes a name, as a string"),
            )),
        }
    }

    /// `(` values `)`, a trailing comma allowed.
    fn arguments(&mut self) -> Result<Vec<Value>, ParseError> {
        self.expect("(", "\u{201c}(\u{201d}")?;
        self.list(")", "\u{201c})\u{201d}")
    }

    /// Values separated by commas up to `close`, a trailing comma allowed.
    fn list(&mut self, close: &str, what: &str) -> Result<Vec<Value>, ParseError> {
        let mut values = Vec::new();
        loop {
            if self.at_punctuation(close) {
                self.bump();
                return Ok(values);
            }
            values.push(self.value()?);
            if !self.at_punctuation(close) {
                self.expect(",", &format!("\u{201c},\u{201d} or {what}"))?;
            }
        }
    }

    fn value(&mut self) -> Result<Value, ParseError> {
        let Some(token) = self.peek().cloned() else {
            return Err(self.error(self.end(), "expected a value"));
        };
        let text = self.text(&token);
        match token.kind {
            TokenKind::String => {
                self.bump();
                literal(text, token.range)
            }
            TokenKind::Number => {
                self.bump();
                number(text, token.range.clone())
            }
            TokenKind::Word => self.word_value(&token, text),
            TokenKind::Punctuation => match text {
                "{" => {
                    self.bump();
                    self.object()
                }
                "[" => {
                    self.bump();
                    self.list("]", "\u{201c}]\u{201d}").map(Value::Array)
                }
                "-" | "+" => self.signed_number(&token, text),
                "(" => Err(self.error(
                    token.range,
                    format!("functions aren't supported: {ONE_COMMAND}"),
                )),
                _ => Err(self.error(
                    token.range,
                    format!("expected a value, found \u{201c}{text}\u{201d}"),
                )),
            },
            _ => Err(self.error(
                token.range,
                format!("expected a value, found \u{201c}{text}\u{201d}"),
            )),
        }
    }

    fn signed_number(&mut self, sign: &Token, text: &str) -> Result<Value, ParseError> {
        self.bump();
        let Some(token) = self.bump() else {
            return Err(self.error(sign.range.clone(), "expected a number after the sign"));
        };
        let written = self.text(&token);
        let range = sign.range.start..token.range.end;
        let value = match token.kind {
            TokenKind::Number => number(written, range)?,
            TokenKind::Word if written == "Infinity" => Value::Number(written.to_owned()),
            _ => return Err(self.error(range, "expected a number after the sign")),
        };
        Ok(match (text, value) {
            ("-", Value::Number(number)) => Value::Number(format!("-{number}")),
            (_, value) => value,
        })
    }

    fn word_value(&mut self, token: &Token, word: &str) -> Result<Value, ParseError> {
        self.bump();
        let simple = match word {
            "true" => Some(Value::Bool(true)),
            "false" => Some(Value::Bool(false)),
            "null" => Some(Value::Null),
            "undefined" => Some(Value::Undefined),
            "Infinity" | "NaN" => Some(Value::Number(word.to_owned())),
            _ => None,
        };
        if let Some(value) = simple {
            return Ok(value);
        }
        if word == "new" {
            let (name, range) = self.name("a type after new")?;
            return match Constructor::named(&name) {
                Some(constructor) => self.construct(constructor, range),
                None => Err(self.error(range, format!("new {name}() isn't a type DbRover knows"))),
            };
        }
        if let Some(constructor) = Constructor::named(word) {
            let bare_key = matches!(constructor, Constructor::MinKey | Constructor::MaxKey)
                && !self.at_punctuation("(");
            if bare_key {
                return Ok(Value::Construct {
                    constructor,
                    arguments: Vec::new(),
                });
            }
            return self.construct(constructor, token.range.clone());
        }
        if word == "function" || (self.at_punctuation("=") && self.peek_second_text() == Some(">"))
        {
            return Err(self.error(
                token.range.clone(),
                format!("functions aren't supported: {ONE_COMMAND}"),
            ));
        }
        if self.at_punctuation("(") {
            return Err(self.error(
                token.range.clone(),
                format!("{word}() isn't a function DbRover knows: {ONE_COMMAND}"),
            ));
        }
        Err(self.error(
            token.range.clone(),
            format!("\u{201c}{word}\u{201d} isn't a value: {ONE_COMMAND}"),
        ))
    }

    /// A constructor's arguments, checked for their count and kinds.
    fn construct(
        &mut self,
        constructor: Constructor,
        range: Range<usize>,
    ) -> Result<Value, ParseError> {
        let arguments = self.arguments()?;
        check_constructor(constructor, &arguments)
            .map_err(|message| self.error(range, format!("{}{message}", constructor.name())))?;
        Ok(Value::Construct {
            constructor,
            arguments,
        })
    }

    /// An object's fields up to its `}`, after the `{`.
    fn object(&mut self) -> Result<Value, ParseError> {
        let mut fields = Vec::new();
        loop {
            if self.at_punctuation("}") {
                self.bump();
                return Ok(Value::Object(fields));
            }
            let Some(token) = self.bump() else {
                return Err(self.error(self.end(), "expected \u{201c}}\u{201d}"));
            };
            let text = self.text(&token);
            let key = match token.kind {
                TokenKind::Word | TokenKind::Number => text.to_owned(),
                TokenKind::String if !text.starts_with('/') => {
                    match literal(text, token.range.clone())? {
                        Value::String(key) => key,
                        _ => return Err(self.error(token.range, "expected a field's name")),
                    }
                }
                _ => {
                    return Err(self.error(
                        token.range,
                        format!("expected a field's name, found \u{201c}{text}\u{201d}"),
                    ));
                }
            };
            if !self.at_punctuation(":") {
                let found = self.peek().map(|next| next.range.clone());
                return Err(self.error(
                    found.unwrap_or(token.range),
                    format!("expected \u{201c}:\u{201d} after {key}"),
                ));
            }
            self.bump();
            fields.push((key, self.value()?));
            if !self.at_punctuation("}") {
                self.expect(",", "\u{201c},\u{201d} or \u{201c}}\u{201d}")?;
            }
        }
    }
}

/// Checks a method's argument count, saying what is wrong in words that follow its name.
fn check_count(arguments: &[Value], min: usize, max: usize) -> Result<(), String> {
    let count = arguments.len();
    if (min..=max).contains(&count) {
        return Ok(());
    }
    let expected = match (min, max) {
        (0, 0) => "no arguments".to_owned(),
        (min, max) if min == max => format!("{min} argument{}", plural(min)),
        (0, max) => format!("at most {max} argument{}", plural(max)),
        (min, max) => format!("{min} to {max} arguments"),
    };
    Err(format!("takes {expected}, not {count}"))
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Checks a constructor's arguments, saying what is wrong in words that follow its name.
fn check_constructor(constructor: Constructor, arguments: &[Value]) -> Result<(), &'static str> {
    use Value::{Number, Object, String};
    let fine = match (constructor, arguments) {
        (Constructor::ObjectId, []) => true,
        (Constructor::ObjectId, [String(hex)]) => {
            hex.len() == 24 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        }
        (Constructor::Date, [] | [String(_)] | [Number(_)]) => true,
        (Constructor::NumberLong | Constructor::NumberInt, [String(text) | Number(text)]) => {
            let digits = text.strip_prefix('-').unwrap_or(text);
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        }
        (Constructor::NumberDecimal, [String(_) | Number(_)]) => true,
        (Constructor::Uuid, []) => true,
        (Constructor::Uuid, [String(text)]) => {
            let hex: std::string::String = text.chars().filter(|c| *c != '-').collect();
            hex.len() == 32 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        }
        (Constructor::BinData, [Number(_), String(_)]) => true,
        (Constructor::Timestamp, [] | [Number(_), Number(_)]) => true,
        (Constructor::Timestamp, [Object(fields)]) => {
            fields.len() == 2
                && fields.iter().all(|(key, value)| {
                    matches!(key.as_str(), "t" | "i") && matches!(value, Number(_))
                })
        }
        (Constructor::MinKey | Constructor::MaxKey, []) => true,
        _ => false,
    };
    if fine {
        return Ok(());
    }
    Err(match constructor {
        Constructor::ObjectId => "() takes nothing, or 24 hexadecimal digits as a string",
        Constructor::Date => "() takes nothing, a date as a string, or milliseconds since 1970",
        Constructor::NumberLong | Constructor::NumberInt => "() takes an integer",
        Constructor::NumberDecimal => "() takes a number, as a string to keep every digit",
        Constructor::Uuid => "() takes nothing, or 32 hexadecimal digits as a string",
        Constructor::BinData => "() takes a subtype and base64 text",
        Constructor::Timestamp => "() takes nothing, seconds and an increment, or {t, i}",
        Constructor::MinKey | Constructor::MaxKey => "() takes nothing",
    })
}

/// A number token's value, checked: hexadecimal, or decimal as JavaScript reads it.
fn number(text: &str, range: Range<usize>) -> Result<Value, ParseError> {
    let valid = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).is_ok(),
        None => text.parse::<f64>().is_ok(),
    };
    if valid {
        Ok(Value::Number(text.to_owned()))
    } else {
        Err(ParseError {
            message: format!("\u{201c}{text}\u{201d} isn't a number"),
            range,
        })
    }
}

/// A string or regular expression token's value.
fn literal(text: &str, range: Range<usize>) -> Result<Value, ParseError> {
    let unclosed = |what: &str| ParseError {
        message: format!("{what} isn't closed"),
        range: range.clone(),
    };
    if let Some(body) = text.strip_prefix('/') {
        let close = body
            .rfind('/')
            .ok_or_else(|| unclosed("a regular expression"))?;
        return Ok(Value::Regex {
            pattern: body[..close].to_owned(),
            flags: body[close + 1..].to_owned(),
        });
    }
    if text.starts_with('`') {
        if text.contains("${") {
            return Err(ParseError {
                message: format!("template strings with ${{…}} aren't supported: {ONE_COMMAND}"),
                range,
            });
        }
        let body = text
            .strip_prefix('`')
            .and_then(|rest| rest.strip_suffix('`'))
            .filter(|_| text.len() > 1)
            .ok_or_else(|| unclosed("a string"))?;
        return Ok(Value::String(unescape(body)));
    }
    unquote(text)
        .map(|body| Value::String(unescape(body)))
        .ok_or_else(|| unclosed("a string"))
}

/// `text` without its surrounding `"` or `'`, if it has both.
fn unquote(text: &str) -> Option<&str> {
    let quote = text.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    text.strip_prefix(quote)?
        .strip_suffix(quote)
        .filter(|_| text.len() > 1)
}

/// A string's body with JavaScript's escapes read: `\n`, `\t`, `\xHH`, `\uHHHH`, `\u{…}`, a
/// line continuation, and any other character escaped as itself.
fn unescape(body: &str) -> String {
    let mut text = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        let Some(escaped) = chars.next() else {
            break;
        };
        match escaped {
            'n' => text.push('\n'),
            'r' => text.push('\r'),
            't' => text.push('\t'),
            'b' => text.push('\u{8}'),
            'f' => text.push('\u{c}'),
            'v' => text.push('\u{b}'),
            '0' => text.push('\0'),
            '\n' => {}
            'x' => push_code(&mut text, &mut chars, 2),
            'u' if chars.peek() == Some(&'{') => {
                chars.next();
                let digits: String = chars.by_ref().take_while(|c| *c != '}').collect();
                match u32::from_str_radix(&digits, 16)
                    .ok()
                    .and_then(char::from_u32)
                {
                    Some(c) => text.push(c),
                    None => text.push('\u{fffd}'),
                }
            }
            'u' => push_code(&mut text, &mut chars, 4),
            other => text.push(other),
        }
    }
    text
}

/// Pushes the character the next `count` hexadecimal digits spell, a UTF-16 surrogate pair
/// (`\ud83d\ude00`) joined into one.
fn push_code(
    text: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    count: usize,
) {
    let hex: String = chars.by_ref().take(count).collect();
    let Ok(code) = u32::from_str_radix(&hex, 16) else {
        text.push('\u{fffd}');
        return;
    };
    if (0xd800..0xdc00).contains(&code) {
        let mut lookahead = chars.clone();
        if lookahead.next() == Some('\\') && lookahead.next() == Some('u') {
            let low: String = lookahead.by_ref().take(4).collect();
            if let Ok(low) = u32::from_str_radix(&low, 16)
                && (0xdc00..0xe000).contains(&low)
            {
                *chars = lookahead;
                let joined = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                text.push(char::from_u32(joined).unwrap_or('\u{fffd}'));
                return;
            }
        }
    }
    text.push(char::from_u32(code).unwrap_or('\u{fffd}'));
}
