//! What a command of the shell sends to the server: one server command per shell method, the
//! way the shell itself sends them, and how its reply reaches the grid.
//!
//! Every method becomes a command document run through `runCommand` (or as a cursor, for
//! `find` and `aggregate`), rather than through the driver's typed helpers, so one path carries
//! the run's `comment` that ⌘. looks for, and `explain` wraps the same document the method
//! would have sent.

use bson::{Bson, Document, doc, oid::ObjectId};
use dbrover_language::mongo::{Call, MethodCall, Show, Value};

use crate::convert::{ValueError, document_argument, to_bson};

/// The server commands a shell command sends, and how their reply is shown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Plan {
    /// The database the commands run in.
    pub(crate) database: String,
    pub(crate) step: Step,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Step {
    /// A command whose reply is a cursor, read a page at a time.
    Cursor { command: Document, rows: CursorRows },
    /// A command whose whole reply is shown through `answer`.
    Command { command: Document, answer: Answer },
    /// Writes run one after another, as `bulkWrite` sends them, until one fails when ordered.
    Bulk {
        writes: Vec<(Document, Answer)>,
        ordered: bool,
    },
}

/// How a cursor's documents reach the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorRows {
    /// As documents.
    Documents,
    /// Each document's `name`, in one column: collections by name.
    Names,
}

/// How a command's reply is shown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Answer {
    /// The reply itself, as one document.
    Reply,
    /// The reply's `n`, in a column named `count`.
    Count,
    /// The `n` of `countDocuments`' pipeline, in a column named `count`; none means 0.
    CountDocuments,
    /// The reply's `values`, one a row, in a column named after the field.
    Distinct(String),
    /// What the shell shows after inserting documents with these `_id`s.
    Inserted {
        ids: Vec<Bson>,
        many: bool,
    },
    Updated,
    Deleted,
    /// `findAndModify`'s `value`: the document before or after, or nothing.
    Modified,
    /// The names of the indexes made, in a column named `index`.
    Indexes(Vec<String>),
    /// The databases `listDatabases` names, with their size.
    Databases,
    /// One value of the reply, in a column of its own.
    Field {
        field: &'static str,
        column: &'static str,
    },
}

/// Why a command can't be sent, though it parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PlanError {
    #[error(transparent)]
    Value(#[from] ValueError),
    #[error("{0}")]
    Method(String),
}

/// What a command of the shell sends: `call` against `current`, the console's database.
pub(crate) fn call(call: &Call, current: &str) -> Result<Plan, PlanError> {
    let database = call.database.clone().unwrap_or_else(|| current.to_owned());
    let method = &call.method;
    let step = match &call.collection {
        Some(collection) if method.name == "renameCollection" => {
            rename(method, &database, collection)?
        }
        Some(collection) => collection_step(collection, method, &call.chain)?,
        None => database_step(method, &call.chain)?,
    };
    // These run against `admin` whichever database the console is in.
    let in_admin = matches!(
        &method.name[..],
        "adminCommand" | "killOp" | "currentOp" | "renameCollection"
    );
    Ok(Plan {
        database: if in_admin {
            "admin".to_owned()
        } else {
            database
        },
        step,
    })
}

/// What `show dbs` and `show collections` send.
pub(crate) fn show(show: Show, current: &str) -> Plan {
    match show {
        Show::Databases => Plan {
            database: "admin".to_owned(),
            step: Step::Command {
                command: doc! { "listDatabases": 1, "nameOnly": false },
                answer: Answer::Databases,
            },
        },
        Show::Collections => Plan {
            database: current.to_owned(),
            step: Step::Cursor {
                command: doc! { "listCollections": 1, "nameOnly": true, "authorizedCollections": true },
                rows: CursorRows::Names,
            },
        },
    }
}

/// The argument at `index`, if given.
fn argument(method: &MethodCall, index: usize) -> Option<&Value> {
    method.arguments.get(index)
}

/// The argument at `index` as a document, or an empty one when it isn't given.
fn document_or_empty(method: &MethodCall, index: usize, what: &str) -> Result<Document, PlanError> {
    match argument(method, index) {
        Some(value) => Ok(document_argument(value, what)?),
        None => Ok(Document::new()),
    }
}

fn bson_argument(method: &MethodCall, index: usize) -> Result<Option<Bson>, PlanError> {
    argument(method, index)
        .map(to_bson)
        .transpose()
        .map_err(PlanError::from)
}

/// Copies every field of `options` into `command`, as the shell does for options whose names
/// are the command's own (`collation`, `hint`, `maxTimeMS`, `let`, `writeConcern`, …).
fn merge(command: &mut Document, options: Document) {
    for (key, value) in options {
        command.insert(key, value);
    }
}

/// Takes `key` out of `options`.
fn take(options: &mut Document, key: &str) -> Option<Bson> {
    options.remove(key)
}

fn collection_step(
    collection: &str,
    method: &MethodCall,
    chain: &[MethodCall],
) -> Result<Step, PlanError> {
    let step = match &method.name[..] {
        "find" => find(collection, method, chain, false)?,
        "findOne" => find(collection, method, chain, true)?,
        "aggregate" => aggregate(Bson::String(collection.to_owned()), method, chain)?,
        "countDocuments" => count_documents(collection, method)?,
        "estimatedDocumentCount" => {
            let mut command = doc! { "count": collection };
            merge(&mut command, document_or_empty(method, 0, "the options")?);
            Step::Command {
                command,
                answer: Answer::Count,
            }
        }
        "count" => {
            let mut command =
                doc! { "count": collection, "query": document_or_empty(method, 0, "the filter")? };
            merge(&mut command, document_or_empty(method, 1, "the options")?);
            Step::Command {
                command,
                answer: Answer::Count,
            }
        }
        "distinct" => distinct(collection, method)?,
        "getIndexes" => Step::Cursor {
            command: doc! { "listIndexes": collection },
            rows: CursorRows::Documents,
        },
        "stats" => {
            let mut command = doc! { "collStats": collection };
            merge(&mut command, document_or_empty(method, 0, "the options")?);
            reply(command)
        }
        "insertOne" => insert(collection, method, false)?,
        "insertMany" => insert(collection, method, true)?,
        "insert" => {
            let many = matches!(argument(method, 0), Some(Value::Array(_)));
            insert(collection, method, many)?
        }
        "updateOne" | "replaceOne" => update(collection, method, Some(false))?,
        "updateMany" => update(collection, method, Some(true))?,
        "update" => update(collection, method, None)?,
        "deleteOne" => delete(collection, method, Some(1))?,
        "deleteMany" => delete(collection, method, Some(0))?,
        "remove" => delete(collection, method, None)?,
        "findOneAndUpdate" | "findOneAndReplace" => find_and_modify(collection, method, false)?,
        "findOneAndDelete" => find_and_modify(collection, method, true)?,
        "bulkWrite" => bulk_write(collection, method)?,
        "createIndex" => create_indexes(collection, method, false)?,
        "createIndexes" => create_indexes(collection, method, true)?,
        "dropIndex" => {
            let index = bson_argument(method, 0)?.unwrap_or(Bson::Null);
            reply(doc! { "dropIndexes": collection, "index": index })
        }
        "dropIndexes" => {
            let index = bson_argument(method, 0)?.unwrap_or_else(|| Bson::String("*".to_owned()));
            reply(doc! { "dropIndexes": collection, "index": index })
        }
        "drop" => reply(doc! { "drop": collection }),
        other => return Err(unsupported(other)),
    };
    if !matches!(&method.name[..], "find" | "aggregate") && !chain.is_empty() {
        return Err(PlanError::Method(format!(
            "{}() takes no cursor methods",
            method.name
        )));
    }
    Ok(step)
}

fn database_step(method: &MethodCall, chain: &[MethodCall]) -> Result<Step, PlanError> {
    Ok(match &method.name[..] {
        "runCommand" | "adminCommand" => reply(document_or_empty(method, 0, "the command")?),
        "aggregate" => aggregate(Bson::Int32(1), method, chain)?,
        "getCollectionNames" => Step::Cursor {
            command: doc! {
                "listCollections": 1,
                "filter": document_or_empty(method, 0, "the filter")?,
                "nameOnly": true,
                "authorizedCollections": true,
            },
            rows: CursorRows::Names,
        },
        "getCollectionInfos" => {
            let mut command = doc! { "listCollections": 1, "filter": document_or_empty(method, 0, "the filter")? };
            merge(&mut command, document_or_empty(method, 1, "the options")?);
            Step::Cursor {
                command,
                rows: CursorRows::Documents,
            }
        }
        "getName" => Step::Command {
            command: doc! { "dbStats": 1, "scale": 1 },
            answer: Answer::Field {
                field: "db",
                column: "name",
            },
        },
        "stats" => {
            let mut command = doc! { "dbStats": 1 };
            merge(&mut command, document_or_empty(method, 0, "the options")?);
            reply(command)
        }
        "version" => Step::Command {
            command: doc! { "buildInfo": 1 },
            answer: Answer::Field {
                field: "version",
                column: "version",
            },
        },
        "createCollection" => {
            let name = string_argument(method, 0, "the collection's name")?;
            let mut command = doc! { "create": name };
            merge(&mut command, document_or_empty(method, 1, "the options")?);
            reply(command)
        }
        "createView" => {
            let name = string_argument(method, 0, "the view's name")?;
            let source = string_argument(method, 1, "the source collection's name")?;
            let pipeline = bson_argument(method, 2)?.unwrap_or_else(|| Bson::Array(Vec::new()));
            let mut command = doc! { "create": name, "viewOn": source, "pipeline": pipeline };
            merge(&mut command, document_or_empty(method, 3, "the options")?);
            reply(command)
        }
        "dropDatabase" => reply(doc! { "dropDatabase": 1 }),
        "currentOp" => {
            let filter = document_or_empty(method, 0, "the filter")?;
            Step::Cursor {
                command: doc! {
                    "aggregate": 1,
                    "pipeline": [ { "$currentOp": {} }, { "$match": filter } ],
                    "cursor": {},
                },
                rows: CursorRows::Documents,
            }
        }
        "killOp" => {
            let op = bson_argument(method, 0)?.unwrap_or(Bson::Null);
            reply(doc! { "killOp": 1, "op": op })
        }
        other => return Err(unsupported(other)),
    })
}

fn unsupported(method: &str) -> PlanError {
    PlanError::Method(format!("{method}() isn't a method DbRover runs"))
}

fn reply(command: Document) -> Step {
    Step::Command {
        command,
        answer: Answer::Reply,
    }
}

fn string_argument(method: &MethodCall, index: usize, what: &str) -> Result<String, PlanError> {
    match argument(method, index) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(PlanError::Method(format!(
            "{}() takes {what} as a string",
            method.name
        ))),
    }
}

/// `find` and `findOne`, with their cursor methods folded into the command.
fn find(
    collection: &str,
    method: &MethodCall,
    chain: &[MethodCall],
    one: bool,
) -> Result<Step, PlanError> {
    let mut command =
        doc! { "find": collection, "filter": document_or_empty(method, 0, "the filter")? };
    if let Some(projection) = argument(method, 1) {
        command.insert(
            "projection",
            document_argument(projection, "the projection")?,
        );
    }
    merge(&mut command, document_or_empty(method, 2, "the options")?);
    let mut explain = None;
    for chained in chain {
        let value = bson_argument(chained, 0)?;
        match (&chained.name[..], value) {
            ("explain", verbosity) => explain = Some(verbosity),
            ("pretty" | "toArray", _) => {}
            ("allowDiskUse", None) => {
                command.insert("allowDiskUse", true);
            }
            (name, Some(value)) => {
                command.insert(name, value);
            }
            (name, None) => {
                return Err(PlanError::Method(format!("{name}() needs an argument")));
            }
        }
    }
    if one {
        command.insert("limit", 1);
        command.insert("singleBatch", true);
    }
    Ok(cursor_or_explain(command, explain))
}

fn aggregate(target: Bson, method: &MethodCall, chain: &[MethodCall]) -> Result<Step, PlanError> {
    let pipeline = match argument(method, 0) {
        Some(value @ Value::Array(_)) => to_bson(value)?,
        _ => {
            return Err(PlanError::Method(
                "aggregate() takes its pipeline as an array, [{…}, …]".to_owned(),
            ));
        }
    };
    let mut options = document_or_empty(method, 1, "the options")?;
    let mut cursor = Document::new();
    if let Some(size) = take(&mut options, "batchSize") {
        cursor.insert("batchSize", size);
    }
    let mut explain = take(&mut options, "explain").map(Some);
    let mut command = doc! { "aggregate": target, "pipeline": pipeline, "cursor": cursor };
    merge(&mut command, options);
    for chained in chain {
        match (&chained.name[..], bson_argument(chained, 0)?) {
            ("explain", verbosity) => explain = Some(verbosity),
            ("pretty" | "toArray" | "batchSize", _) => {}
            (name, Some(value)) => {
                command.insert(name, value);
            }
            (name, None) => {
                return Err(PlanError::Method(format!("{name}() needs an argument")));
            }
        }
    }
    Ok(cursor_or_explain(command, explain))
}

/// The cursor for `command`, or, with `.explain(verbosity)` chained, its plan: `explain` runs
/// the same command with `queryPlanner` verbosity unless told another (`executionStats`,
/// `allPlansExecution`, or `true` for all of them).
fn cursor_or_explain(command: Document, explain: Option<Option<Bson>>) -> Step {
    match explain {
        None => Step::Cursor {
            command,
            rows: CursorRows::Documents,
        },
        Some(verbosity) => reply(explain_command(command, verbosity)),
    }
}

/// The command of a read the server can plan without running: a `find`, an `aggregate`, a
/// `count` or a `distinct`. `None` for anything else, writes included.
pub(crate) fn plannable_read(step: &Step) -> Option<&Document> {
    let command = match step {
        Step::Cursor { command, .. } | Step::Command { command, .. } => command,
        Step::Bulk { .. } => return None,
    };
    let name = command.keys().next()?;
    matches!(name.as_str(), "find" | "aggregate" | "count" | "distinct").then_some(command)
}

/// `{explain: command, verbosity}`, a verbosity of `true` or `false` read as the shell reads it.
pub(crate) fn explain_command(command: Document, verbosity: Option<Bson>) -> Document {
    let verbosity = match verbosity {
        Some(Bson::String(verbosity)) => verbosity,
        Some(Bson::Boolean(true)) => "allPlansExecution".to_owned(),
        _ => "queryPlanner".to_owned(),
    };
    doc! { "explain": command, "verbosity": verbosity }
}

/// `countDocuments`, which the shell runs as a pipeline, so it counts exactly what matches.
fn count_documents(collection: &str, method: &MethodCall) -> Result<Step, PlanError> {
    let filter = document_or_empty(method, 0, "the filter")?;
    let mut options = document_or_empty(method, 1, "the options")?;
    let mut pipeline = vec![Bson::Document(doc! { "$match": filter })];
    if let Some(skip) = take(&mut options, "skip") {
        pipeline.push(Bson::Document(doc! { "$skip": skip }));
    }
    if let Some(limit) = take(&mut options, "limit") {
        pipeline.push(Bson::Document(doc! { "$limit": limit }));
    }
    pipeline.push(Bson::Document(
        doc! { "$group": { "_id": 1, "n": { "$sum": 1 } } },
    ));
    let mut command = doc! { "aggregate": collection, "pipeline": pipeline, "cursor": {} };
    merge(&mut command, options);
    Ok(Step::Command {
        command,
        answer: Answer::CountDocuments,
    })
}

fn distinct(collection: &str, method: &MethodCall) -> Result<Step, PlanError> {
    let field = string_argument(method, 0, "the field's name")?;
    let mut command = doc! {
        "distinct": collection,
        "key": &field,
        "query": document_or_empty(method, 1, "the filter")?,
    };
    merge(&mut command, document_or_empty(method, 2, "the options")?);
    Ok(Step::Command {
        command,
        answer: Answer::Distinct(field),
    })
}

/// The documents to insert, each given an `_id` first if it has none, as the shell does, so
/// the answer can name them.
fn insert(collection: &str, method: &MethodCall, many: bool) -> Result<Step, PlanError> {
    let documents: Vec<Document> = match (many, argument(method, 0)) {
        (true, Some(Value::Array(items))) => items
            .iter()
            .map(|item| document_argument(item, "each document"))
            .collect::<Result<_, _>>()?,
        (false, Some(value)) => vec![document_argument(value, "the document")?],
        _ => {
            return Err(PlanError::Method(format!(
                "{}() takes the documents as an array, [{{…}}, …]",
                method.name
            )));
        }
    };
    let mut ids = Vec::with_capacity(documents.len());
    let documents: Vec<Bson> = documents
        .into_iter()
        .map(|document| {
            let (id, document) = with_id(document);
            ids.push(id);
            Bson::Document(document)
        })
        .collect();
    let mut command = doc! { "insert": collection, "documents": documents };
    merge(&mut command, document_or_empty(method, 1, "the options")?);
    Ok(Step::Command {
        command,
        answer: Answer::Inserted { ids, many },
    })
}

/// `document` with an `_id`, a new `ObjectId` put first when it had none, and that `_id`.
fn with_id(document: Document) -> (Bson, Document) {
    if let Some(id) = document.get("_id") {
        return (id.clone(), document);
    }
    let id = Bson::ObjectId(ObjectId::new());
    let mut with = doc! { "_id": id.clone() };
    merge(&mut with, document);
    (id, with)
}

/// `updateOne`, `updateMany`, `replaceOne` and the shell's older `update`, whose `multi`
/// option says how many it changes. Options that belong to the statement (`upsert`,
/// `arrayFilters`, `collation`, `hint`) go in it; the rest to the command.
fn update(collection: &str, method: &MethodCall, multi: Option<bool>) -> Result<Step, PlanError> {
    let filter = document_or_empty(method, 0, "the filter")?;
    let change = match argument(method, 1) {
        Some(value @ (Value::Object(_) | Value::Array(_))) => to_bson(value)?,
        _ => {
            return Err(PlanError::Method(format!(
                "{}() takes the change as an object or a pipeline",
                method.name
            )));
        }
    };
    let mut options = document_or_empty(method, 2, "the options")?;
    let multi = match multi {
        Some(multi) => multi,
        None => matches!(take(&mut options, "multi"), Some(Bson::Boolean(true))),
    };
    let mut statement = doc! { "q": filter, "u": change, "multi": multi };
    for key in ["upsert", "arrayFilters", "collation", "hint"] {
        if let Some(value) = take(&mut options, key) {
            statement.insert(key, value);
        }
    }
    let mut command = doc! { "update": collection, "updates": [statement] };
    merge(&mut command, options);
    Ok(Step::Command {
        command,
        answer: Answer::Updated,
    })
}

/// `deleteOne`, `deleteMany`, and the shell's older `remove`, which deletes one when its
/// second argument is `true` or `{justOne: true}`.
fn delete(collection: &str, method: &MethodCall, limit: Option<i32>) -> Result<Step, PlanError> {
    let filter = document_or_empty(method, 0, "the filter")?;
    let (limit, mut options) = match (limit, argument(method, 1)) {
        (Some(limit), _) => (limit, document_or_empty(method, 1, "the options")?),
        (None, Some(Value::Bool(just_one))) => (i32::from(*just_one), Document::new()),
        (None, _) => {
            let mut options = document_or_empty(method, 1, "the options")?;
            let just_one = matches!(take(&mut options, "justOne"), Some(Bson::Boolean(true)));
            (i32::from(just_one), options)
        }
    };
    let mut statement = doc! { "q": filter, "limit": limit };
    for key in ["collation", "hint"] {
        if let Some(value) = take(&mut options, key) {
            statement.insert(key, value);
        }
    }
    let mut command = doc! { "delete": collection, "deletes": [statement] };
    merge(&mut command, options);
    Ok(Step::Command {
        command,
        answer: Answer::Deleted,
    })
}

/// `findOneAndUpdate`, `findOneAndReplace` and `findOneAndDelete`, as `findAndModify`.
fn find_and_modify(collection: &str, method: &MethodCall, remove: bool) -> Result<Step, PlanError> {
    let filter = document_or_empty(method, 0, "the filter")?;
    let mut command = doc! { "findAndModify": collection, "query": filter };
    let options_at = if remove {
        command.insert("remove", true);
        1
    } else {
        let change = bson_argument(method, 1)?.unwrap_or(Bson::Null);
        command.insert("update", change);
        2
    };
    let mut options = document_or_empty(method, options_at, "the options")?;
    let after = matches!(take(&mut options, "returnDocument"), Some(Bson::String(when)) if when == "after")
        || matches!(
            take(&mut options, "returnNewDocument"),
            Some(Bson::Boolean(true))
        );
    if !remove {
        command.insert("new", after);
    }
    if let Some(projection) = take(&mut options, "projection") {
        command.insert("fields", projection);
    }
    merge(&mut command, options);
    Ok(Step::Command {
        command,
        answer: Answer::Modified,
    })
}

/// `bulkWrite`'s operations, each the command its own method would send.
fn bulk_write(collection: &str, method: &MethodCall) -> Result<Step, PlanError> {
    let Some(Value::Array(operations)) = argument(method, 0) else {
        return Err(PlanError::Method(
            "bulkWrite() takes its operations as an array, [{insertOne: …}, …]".to_owned(),
        ));
    };
    let mut options = document_or_empty(method, 1, "the options")?;
    let ordered = !matches!(take(&mut options, "ordered"), Some(Bson::Boolean(false)));
    let writes = operations
        .iter()
        .map(|operation| bulk_operation(collection, operation))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Step::Bulk { writes, ordered })
}

fn bulk_operation(collection: &str, operation: &Value) -> Result<(Document, Answer), PlanError> {
    let invalid = || {
        PlanError::Method(
            "each bulkWrite() operation is {insertOne: …}, {updateOne: …}, {updateMany: …}, \
             {replaceOne: …}, {deleteOne: …} or {deleteMany: …}"
                .to_owned(),
        )
    };
    let [(name, Value::Object(fields))] = operation.as_object().ok_or_else(invalid)? else {
        return Err(invalid());
    };
    let field = |key: &str| {
        fields
            .iter()
            .find(|(field, _)| field == key)
            .map(|(_, value)| value.clone())
    };
    let options: Vec<(String, Value)> = fields
        .iter()
        .filter(|(key, _)| !matches!(&key[..], "document" | "filter" | "update" | "replacement"))
        .cloned()
        .collect();
    let synthetic = |method: &str, arguments: Vec<Value>| MethodCall {
        name: method.to_owned(),
        arguments,
        range: 0..0,
    };
    let step = match &name[..] {
        "insertOne" => insert(
            collection,
            &synthetic("insertOne", vec![field("document").ok_or_else(invalid)?]),
            false,
        )?,
        "updateOne" | "updateMany" | "replaceOne" => {
            let change = field("update")
                .or_else(|| field("replacement"))
                .ok_or_else(invalid)?;
            let filter = field("filter").ok_or_else(invalid)?;
            update(
                collection,
                &synthetic(name, vec![filter, change, Value::Object(options)]),
                Some(name == "updateMany"),
            )?
        }
        "deleteOne" | "deleteMany" => {
            let filter = field("filter").ok_or_else(invalid)?;
            delete(
                collection,
                &synthetic(name, vec![filter, Value::Object(options)]),
                Some(i32::from(name == "deleteOne")),
            )?
        }
        _ => return Err(invalid()),
    };
    match step {
        Step::Command { command, answer } => Ok((command, answer)),
        _ => Err(invalid()),
    }
}

/// `createIndex(keys, options)` and `createIndexes([keys, …], options)`, each index named as
/// the server would name it (`status_1_created_-1`) unless the options name it.
fn create_indexes(collection: &str, method: &MethodCall, many: bool) -> Result<Step, PlanError> {
    let keys: Vec<Document> = match (many, argument(method, 0)) {
        (true, Some(Value::Array(items))) => items
            .iter()
            .map(|item| document_argument(item, "each index's keys"))
            .collect::<Result<_, _>>()?,
        (false, Some(value)) => vec![document_argument(value, "the index's keys")?],
        _ => {
            return Err(PlanError::Method(
                "createIndexes() takes the indexes' keys as an array, [{…}, …]".to_owned(),
            ));
        }
    };
    let options = document_or_empty(method, 1, "the options")?;
    let mut names = Vec::with_capacity(keys.len());
    let indexes: Vec<Bson> = keys
        .into_iter()
        .map(|key| {
            let mut index = doc! { "key": key.clone() };
            merge(&mut index, options.clone());
            if !index.contains_key("name") {
                index.insert("name", index_name(&key));
            }
            names.push(index.get_str("name").unwrap_or_default().to_owned());
            Bson::Document(index)
        })
        .collect();
    Ok(Step::Command {
        command: doc! { "createIndexes": collection, "indexes": indexes },
        answer: Answer::Indexes(names),
    })
}

/// The name the server gives an index on `key`: each field and its direction, joined by `_`.
fn index_name(key: &Document) -> String {
    key.iter()
        .map(|(field, direction)| {
            let direction = match direction {
                Bson::String(kind) => kind.clone(),
                Bson::Int32(number) => number.to_string(),
                Bson::Int64(number) => number.to_string(),
                Bson::Double(number) => number.to_string(),
                other => other.to_string(),
            };
            format!("{field}_{direction}")
        })
        .collect::<Vec<_>>()
        .join("_")
}

/// `renameCollection(to, dropTarget)`, an administration command naming both collections in
/// full.
fn rename(method: &MethodCall, database: &str, collection: &str) -> Result<Step, PlanError> {
    let to = string_argument(method, 0, "the new name")?;
    let drop_target = matches!(argument(method, 1), Some(Value::Bool(true)));
    Ok(reply(doc! {
        "renameCollection": format!("{database}.{collection}"),
        "to": format!("{database}.{to}"),
        "dropTarget": drop_target,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbrover_language::mongo::{Command, parse};

    fn planned(text: &str) -> Plan {
        let Ok(Command::Call(parsed)) = parse(text) else {
            panic!("{text} doesn't parse");
        };
        call(&parsed, "shop").unwrap_or_else(|error| panic!("{text}: {error}"))
    }

    fn command(text: &str) -> Document {
        match planned(text).step {
            Step::Cursor { command, .. } | Step::Command { command, .. } => command,
            Step::Bulk { .. } => panic!("{text} is a bulk write"),
        }
    }

    #[test]
    fn a_find_folds_its_cursor_methods_into_the_command() {
        assert_eq!(
            command("db.orders.find({status: 'open'}, {_id: 0}).sort({_id: -1}).skip(5).limit(50)"),
            doc! {
                "find": "orders",
                "filter": { "status": "open" },
                "projection": { "_id": 0 },
                "sort": { "_id": -1 },
                "skip": 5,
                "limit": 50,
            }
        );
        assert_eq!(
            command("db.orders.findOne({_id: 1})"),
            doc! { "find": "orders", "filter": { "_id": 1 }, "limit": 1, "singleBatch": true }
        );
    }

    #[test]
    fn explain_wraps_the_command_the_method_would_send() {
        assert_eq!(
            command("db.orders.find({a: 1}).limit(2).explain('executionStats')"),
            doc! {
                "explain": { "find": "orders", "filter": { "a": 1 }, "limit": 2 },
                "verbosity": "executionStats",
            }
        );
        assert_eq!(
            command("db.orders.aggregate([{$match: {}}]).explain()"),
            doc! {
                "explain": { "aggregate": "orders", "pipeline": [{ "$match": {} }], "cursor": {} },
                "verbosity": "queryPlanner",
            }
        );
    }

    #[test]
    fn writes_send_the_commands_the_shell_sends() {
        assert_eq!(
            command(
                "db.orders.updateMany({}, {$set: {a: 1}}, {upsert: true, writeConcern: {w: 1}})"
            ),
            doc! {
                "update": "orders",
                "updates": [{ "q": {}, "u": { "$set": { "a": 1 } }, "multi": true, "upsert": true }],
                "writeConcern": { "w": 1 },
            }
        );
        assert_eq!(
            command("db.orders.deleteOne({a: 1})"),
            doc! { "delete": "orders", "deletes": [{ "q": { "a": 1 }, "limit": 1 }] }
        );
        assert_eq!(
            command("db.orders.remove({a: 1}, true)"),
            doc! { "delete": "orders", "deletes": [{ "q": { "a": 1 }, "limit": 1 }] }
        );
        assert_eq!(
            command(
                "db.orders.findOneAndUpdate({a: 1}, {$inc: {n: 1}}, {returnDocument: 'after', projection: {n: 1}})"
            ),
            doc! {
                "findAndModify": "orders",
                "query": { "a": 1 },
                "update": { "$inc": { "n": 1 } },
                "new": true,
                "fields": { "n": 1 },
            }
        );
    }

    #[test]
    fn an_insert_gives_each_document_an_id_first() {
        let Step::Command {
            command,
            answer: Answer::Inserted { ids, many: true },
        } = planned("db.orders.insertMany([{_id: 7, a: 1}, {a: 2}])").step
        else {
            panic!("expected an insert");
        };
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], Bson::Int32(7));
        let documents = command.get_array("documents").cloned().unwrap_or_default();
        let second = documents[1].as_document().cloned().unwrap_or_default();
        assert_eq!(second.keys().collect::<Vec<_>>(), ["_id", "a"]);
        assert_eq!(second.get("_id"), Some(&ids[1]));
    }

    #[test]
    fn indexes_are_named_as_the_server_names_them() {
        let Step::Command {
            command,
            answer: Answer::Indexes(names),
        } = planned("db.orders.createIndex({status: 1, created: -1}, {unique: true})").step
        else {
            panic!("expected createIndexes");
        };
        assert_eq!(names, ["status_1_created_-1"]);
        assert_eq!(
            command,
            doc! {
                "createIndexes": "orders",
                "indexes": [{ "key": { "status": 1, "created": -1 }, "unique": true, "name": "status_1_created_-1" }],
            }
        );
    }

    #[test]
    fn commands_go_to_the_database_they_name() {
        assert_eq!(planned("db.orders.find()").database, "shop");
        assert_eq!(
            planned("db.getSiblingDB('logs').events.find()").database,
            "logs"
        );
        assert_eq!(
            planned("db.adminCommand({listDatabases: 1})").database,
            "admin"
        );
        assert_eq!(
            planned("db.orders.renameCollection('old')"),
            Plan {
                database: "admin".to_owned(),
                step: Step::Command {
                    command: doc! { "renameCollection": "shop.orders", "to": "shop.old", "dropTarget": false },
                    answer: Answer::Reply,
                },
            }
        );
    }

    #[test]
    fn a_bulk_write_sends_each_operation_as_its_method_would() {
        let Step::Bulk { writes, ordered } = planned(
            "db.orders.bulkWrite([{insertOne: {document: {_id: 1}}}, {deleteMany: {filter: {a: 1}}}], {ordered: false})",
        )
        .step
        else {
            panic!("expected a bulk write");
        };
        assert!(!ordered);
        let commands: Vec<&Document> = writes.iter().map(|(command, _)| command).collect();
        assert_eq!(
            commands,
            [
                &doc! { "insert": "orders", "documents": [{ "_id": 1 }] },
                &doc! { "delete": "orders", "deletes": [{ "q": { "a": 1 }, "limit": 0 }] },
            ]
        );
    }
}
