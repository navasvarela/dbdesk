//! The methods DbRover runs, of a collection and of the database, with what each may change:
//! the table the parser checks a command against and classification reads.

use dbrover_sql::StatementKind;
use dbrover_sql::StatementKind::{Admin, Read, Schema, Write};

/// What a method is called on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Receiver {
    Collection,
    Database,
}

/// A method of the shell DbRover runs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MethodSpec {
    pub(crate) name: &'static str,
    pub(crate) receiver: Receiver,
    /// What it may change, before its arguments are looked at: an `aggregate` with `$out`
    /// writes, and `runCommand` depends on the command it sends.
    pub(crate) kind: StatementKind,
    pub(crate) min_arguments: usize,
    pub(crate) max_arguments: usize,
    /// Its arguments, as completion shows them.
    pub(crate) syntax: &'static str,
    /// The cursor methods that may be chained after it; empty for a method that returns no
    /// cursor.
    pub(crate) cursor: &'static [&'static str],
}

/// What may follow `find(…)`, as the shell's cursor has them.
const FIND_CURSOR: &[&str] = &[
    "sort",
    "limit",
    "skip",
    "projection",
    "hint",
    "collation",
    "batchSize",
    "maxTimeMS",
    "allowDiskUse",
    "explain",
    "pretty",
    "toArray",
];

/// What may follow `aggregate(…)`.
const AGGREGATE_CURSOR: &[&str] = &["batchSize", "maxTimeMS", "explain", "pretty", "toArray"];

/// A cursor method: how many arguments it takes, and what completion shows beside it.
pub(crate) struct CursorMethod {
    pub(crate) name: &'static str,
    pub(crate) min_arguments: usize,
    pub(crate) max_arguments: usize,
    pub(crate) syntax: &'static str,
}

pub(crate) const CURSOR_METHODS: &[CursorMethod] = &[
    cursor_method("sort", 1, 1, "sort"),
    cursor_method("limit", 1, 1, "count"),
    cursor_method("skip", 1, 1, "count"),
    cursor_method("projection", 1, 1, "projection"),
    cursor_method("hint", 1, 1, "index"),
    cursor_method("collation", 1, 1, "collation"),
    cursor_method("batchSize", 1, 1, "size"),
    cursor_method("maxTimeMS", 1, 1, "milliseconds"),
    cursor_method("allowDiskUse", 0, 1, "allow"),
    cursor_method("explain", 0, 1, "verbosity"),
    cursor_method("pretty", 0, 0, ""),
    cursor_method("toArray", 0, 0, ""),
];

const fn cursor_method(
    name: &'static str,
    min_arguments: usize,
    max_arguments: usize,
    syntax: &'static str,
) -> CursorMethod {
    CursorMethod {
        name,
        min_arguments,
        max_arguments,
        syntax,
    }
}

const fn collection(
    name: &'static str,
    kind: StatementKind,
    arguments: (usize, usize),
    syntax: &'static str,
) -> MethodSpec {
    MethodSpec {
        name,
        receiver: Receiver::Collection,
        kind,
        min_arguments: arguments.0,
        max_arguments: arguments.1,
        syntax,
        cursor: &[],
    }
}

const fn database(
    name: &'static str,
    kind: StatementKind,
    arguments: (usize, usize),
    syntax: &'static str,
) -> MethodSpec {
    MethodSpec {
        name,
        receiver: Receiver::Database,
        kind,
        min_arguments: arguments.0,
        max_arguments: arguments.1,
        syntax,
        cursor: &[],
    }
}

const fn with_cursor(spec: MethodSpec, cursor: &'static [&'static str]) -> MethodSpec {
    MethodSpec { cursor, ..spec }
}

/// Every method DbRover runs. Others, the users' and roles' helpers among them, are refused by
/// the parser, which suggests `db.runCommand(…)`; `runCommand` and `adminCommand` are
/// classified by the command they send ([`command_kind`]).
pub(crate) const METHODS: &[MethodSpec] = &[
    with_cursor(
        collection("find", Read, (0, 3), "filter, projection, options"),
        FIND_CURSOR,
    ),
    collection("findOne", Read, (0, 3), "filter, projection, options"),
    with_cursor(
        collection("aggregate", Read, (1, 2), "pipeline, options"),
        AGGREGATE_CURSOR,
    ),
    collection("countDocuments", Read, (0, 2), "filter, options"),
    collection("estimatedDocumentCount", Read, (0, 1), "options"),
    collection("count", Read, (0, 2), "filter, options"),
    collection("distinct", Read, (1, 3), "field, filter, options"),
    collection("getIndexes", Read, (0, 0), ""),
    collection("stats", Read, (0, 1), "options"),
    collection("insertOne", Write, (1, 2), "document, options"),
    collection("insertMany", Write, (1, 2), "documents, options"),
    collection("insert", Write, (1, 2), "document or documents, options"),
    collection("updateOne", Write, (2, 3), "filter, update, options"),
    collection("updateMany", Write, (2, 3), "filter, update, options"),
    collection("replaceOne", Write, (2, 3), "filter, replacement, options"),
    collection("deleteOne", Write, (1, 2), "filter, options"),
    collection("deleteMany", Write, (1, 2), "filter, options"),
    collection("remove", Write, (1, 2), "filter, options"),
    collection("findOneAndUpdate", Write, (2, 3), "filter, update, options"),
    collection(
        "findOneAndReplace",
        Write,
        (2, 3),
        "filter, replacement, options",
    ),
    collection("findOneAndDelete", Write, (1, 2), "filter, options"),
    collection("bulkWrite", Write, (1, 2), "operations, options"),
    collection("createIndex", Schema, (1, 2), "keys, options"),
    collection("createIndexes", Schema, (1, 2), "indexes, options"),
    collection("dropIndex", Schema, (1, 1), "index"),
    collection("dropIndexes", Schema, (0, 1), "indexes"),
    collection("drop", Schema, (0, 1), "options"),
    collection("renameCollection", Schema, (1, 2), "new name, drop target"),
    database("runCommand", Admin, (1, 1), "command"),
    database("adminCommand", Admin, (1, 1), "command"),
    with_cursor(
        database("aggregate", Read, (1, 2), "pipeline, options"),
        AGGREGATE_CURSOR,
    ),
    database("getCollectionNames", Read, (0, 1), "filter"),
    database("getCollectionInfos", Read, (0, 2), "filter, options"),
    database("getName", Read, (0, 0), ""),
    database("stats", Read, (0, 1), "options"),
    database("version", Read, (0, 0), ""),
    database("createCollection", Schema, (1, 2), "name, options"),
    database(
        "createView",
        Schema,
        (3, 4),
        "name, source, pipeline, options",
    ),
    database("dropDatabase", Admin, (0, 1), "options"),
    database("currentOp", Admin, (0, 1), "filter"),
    database("killOp", Admin, (1, 1), "operation id"),
];

/// The method `name` of `receiver`, as the table has it.
pub(crate) fn method(receiver: Receiver, name: &str) -> Option<&'static MethodSpec> {
    METHODS
        .iter()
        .find(|spec| spec.receiver == receiver && spec.name == name)
}

/// The cursor method `name`.
pub(crate) fn cursor_method_named(name: &str) -> Option<&'static CursorMethod> {
    CURSOR_METHODS.iter().find(|spec| spec.name == name)
}

/// Commands `runCommand` may send that only read, compared ignoring case as the server
/// compares the common ones (`buildinfo`, `ismaster`). Everything else it sends is
/// administration, unless [`WRITE_COMMANDS`] or [`SCHEMA_COMMANDS`] name it.
const READ_COMMANDS: &[&str] = &[
    "aggregate",
    "buildInfo",
    "collStats",
    "connectionStatus",
    "count",
    "dataSize",
    "dbStats",
    "distinct",
    "explain",
    "find",
    "hello",
    "hostInfo",
    "isMaster",
    "listCollections",
    "listCommands",
    "listDatabases",
    "listIndexes",
    "listSearchIndexes",
    "ping",
    "serverStatus",
];

const WRITE_COMMANDS: &[&str] = &["insert", "update", "delete", "findAndModify", "bulkWrite"];

const SCHEMA_COMMANDS: &[&str] = &[
    "create",
    "createIndexes",
    "drop",
    "dropIndexes",
    "collMod",
    "renameCollection",
];

/// What the command named `name`, a `runCommand` document's first key, may change.
pub(crate) fn command_kind(name: &str) -> StatementKind {
    let listed = |names: &[&str]| names.iter().any(|known| known.eq_ignore_ascii_case(name));
    if listed(READ_COMMANDS) {
        Read
    } else if listed(WRITE_COMMANDS) {
        Write
    } else if listed(SCHEMA_COMMANDS) {
        Schema
    } else {
        Admin
    }
}
