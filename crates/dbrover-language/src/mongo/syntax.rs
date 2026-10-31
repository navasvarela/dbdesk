//! A command of the shell's syntax as the parser reads it: what the driver runs, the
//! classification checks and the warnings look at.

use std::ops::Range;

/// One command.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `use <database>`: the database later commands run in.
    Use(String),
    Show(Show),
    /// `db.…`, a method of a collection or of the database, with the cursor methods chained to
    /// it.
    Call(Call),
}

/// What `show` lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    /// `show dbs`, or `show databases`.
    Databases,
    /// `show collections`, or `show tables`.
    Collections,
}

/// A method called through `db`.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// The database `db.getSiblingDB("…")` names; `None` for the console's current one.
    pub database: Option<String>,
    /// The collection the method is called on, `orders` in `db.orders.find()`; `None` for a
    /// method of the database itself, as `db.runCommand(…)`.
    pub collection: Option<String>,
    pub method: MethodCall,
    /// Cursor methods chained after it, in order: `.sort(…)`, `.limit(…)`, `.explain()`.
    pub chain: Vec<MethodCall>,
}

impl Call {
    /// The method chained after the first one named `name`, if any.
    pub fn chained(&self, name: &str) -> Option<&MethodCall> {
        self.chain.iter().find(|method| method.name == name)
    }
}

/// A method's name and arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct MethodCall {
    pub name: String,
    pub arguments: Vec<Value>,
    /// Where the method's name is in the command, for messages about it.
    pub range: Range<usize>,
}

/// A value written in the shell's relaxed JSON.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Undefined,
    Bool(bool),
    /// A number as written, with its sign: `1`, `-2.5`, `1e3`, `0x1f`, `Infinity`, `NaN`. The
    /// shell stores an integer that fits 32 bits as one and any other number as a double.
    Number(String),
    String(String),
    Regex {
        pattern: String,
        flags: String,
    },
    Array(Vec<Value>),
    /// An object's fields in the order written, as BSON keeps them.
    Object(Vec<(String, Value)>),
    /// A BSON type's constructor, `ObjectId("…")` or `new Date()`, with arguments whose count
    /// and kinds the parser has checked.
    Construct {
        constructor: Constructor,
        arguments: Vec<Value>,
    },
}

impl Value {
    /// The fields of an object, and `None` for any other value.
    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Self::Object(fields) => Some(fields),
            _ => None,
        }
    }

    /// The items of an array, and `None` for any other value.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// Whether this is an object with no fields, `{}`: a filter that matches everything.
    pub fn is_empty_object(&self) -> bool {
        self.as_object().is_some_and(<[_]>::is_empty)
    }

    /// Every object key in this value and the values inside it, outermost first.
    pub fn keys(&self) -> Vec<&str> {
        let mut keys = Vec::new();
        self.collect_keys(&mut keys);
        keys
    }

    fn collect_keys<'a>(&'a self, keys: &mut Vec<&'a str>) {
        match self {
            Self::Object(fields) => {
                for (key, value) in fields {
                    keys.push(key);
                    value.collect_keys(keys);
                }
            }
            Self::Array(items) => {
                for item in items {
                    item.collect_keys(keys);
                }
            }
            Self::Construct { arguments, .. } => {
                for argument in arguments {
                    argument.collect_keys(keys);
                }
            }
            _ => {}
        }
    }
}

/// The BSON types the shell writes with a constructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constructor {
    /// `ObjectId()` for a new one, or `ObjectId("…")` with 24 hexadecimal digits.
    ObjectId,
    /// `ISODate(…)`, `new Date(…)` and `Date(…)`: now, a date's text, or milliseconds since
    /// 1970.
    Date,
    /// A 64-bit integer, from a number or its text.
    NumberLong,
    /// A 32-bit integer, from a number or its text.
    NumberInt,
    /// A 128-bit decimal, from its text or a number.
    NumberDecimal,
    /// A UUID, a new one or from its text, stored as binary of subtype 4.
    Uuid,
    /// `BinData(subtype, "base64")`.
    BinData,
    /// `Timestamp()`, `Timestamp(seconds, increment)` or `Timestamp({t: …, i: …})`.
    Timestamp,
    MinKey,
    MaxKey,
}

impl Constructor {
    /// The constructor a name in the shell calls, the names other shells and drivers use
    /// included.
    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "ObjectId" => Self::ObjectId,
            "ISODate" | "Date" => Self::Date,
            "NumberLong" | "Long" => Self::NumberLong,
            "NumberInt" | "Int32" => Self::NumberInt,
            "NumberDecimal" | "Decimal128" => Self::NumberDecimal,
            "UUID" => Self::Uuid,
            "BinData" => Self::BinData,
            "Timestamp" => Self::Timestamp,
            "MinKey" => Self::MinKey,
            "MaxKey" => Self::MaxKey,
            _ => return None,
        })
    }

    /// The name the shell shows the type with.
    pub fn name(self) -> &'static str {
        match self {
            Self::ObjectId => "ObjectId",
            Self::Date => "ISODate",
            Self::NumberLong => "NumberLong",
            Self::NumberInt => "NumberInt",
            Self::NumberDecimal => "NumberDecimal",
            Self::Uuid => "UUID",
            Self::BinData => "BinData",
            Self::Timestamp => "Timestamp",
            Self::MinKey => "MinKey",
            Self::MaxKey => "MaxKey",
        }
    }

    /// Every constructor's names, for highlighting and completion.
    pub(crate) const NAMES: [&'static str; 14] = [
        "BinData",
        "Date",
        "Decimal128",
        "ISODate",
        "Int32",
        "Long",
        "MaxKey",
        "MinKey",
        "NumberDecimal",
        "NumberInt",
        "NumberLong",
        "ObjectId",
        "Timestamp",
        "UUID",
    ];
}
