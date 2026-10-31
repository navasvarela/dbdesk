//! What a command may change, and whether it may touch everything, from the tree the parser
//! reads (`docs/nosql.md`, N2).

use dbrover_sql::StatementKind;

use super::methods::{self, Receiver};
use super::syntax::{Call, Command, Value};

/// Keys that run JavaScript on the server, wherever they appear: what the script does is
/// beyond the check, so only a login allowed to administer may send them.
const SCRIPT_KEYS: &[&str] = &["$where", "$function", "$accumulator"];

/// Pipeline stages that write their results to a collection.
const WRITE_STAGES: &[&str] = &["$out", "$merge"];

/// What `command` may change.
pub(crate) fn kind(command: &Command) -> StatementKind {
    match command {
        Command::Use(_) | Command::Show(_) => StatementKind::Read,
        Command::Call(call) => call_kind(call),
    }
}

fn call_kind(call: &Call) -> StatementKind {
    let arguments = &call.method.arguments;
    let keys: Vec<&str> = arguments.iter().flat_map(Value::keys).collect();
    if keys.iter().any(|key| SCRIPT_KEYS.contains(key)) {
        return StatementKind::Admin;
    }
    let receiver = if call.collection.is_some() {
        Receiver::Collection
    } else {
        Receiver::Database
    };
    let Some(spec) = methods::method(receiver, &call.method.name) else {
        return StatementKind::Admin;
    };
    match spec.name {
        "aggregate" => pipeline_kind(arguments.first()),
        "runCommand" | "adminCommand" => arguments
            .first()
            .map_or(StatementKind::Admin, run_command_kind),
        _ => spec.kind,
    }
}

/// A pipeline reads, unless one of its stages writes to a collection.
fn pipeline_kind(pipeline: Option<&Value>) -> StatementKind {
    let writes = pipeline
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_object)
        .flatten()
        .any(|(stage, _)| WRITE_STAGES.contains(&stage.as_str()));
    if writes {
        StatementKind::Write
    } else {
        StatementKind::Read
    }
}

/// A command document is classified by its first key, the command's name, as the server
/// reads it; an `aggregate` by its pipeline as well.
fn run_command_kind(command: &Value) -> StatementKind {
    let Some(fields) = command.as_object() else {
        return StatementKind::Admin;
    };
    let Some((name, _)) = fields.first() else {
        return StatementKind::Admin;
    };
    match methods::command_kind(name) {
        StatementKind::Read if name.eq_ignore_ascii_case("aggregate") => {
            let pipeline = fields
                .iter()
                .find(|(key, _)| key == "pipeline")
                .map(|(_, pipeline)| pipeline);
            pipeline_kind(pipeline)
        }
        kind => kind,
    }
}

/// Why `command` may touch everything, for the warning before it runs.
pub(crate) fn hazard(command: &Command) -> Option<&'static str> {
    let Command::Call(call) = command else {
        return None;
    };
    let filter = call.method.arguments.first();
    let matches_everything = filter.is_none_or(Value::is_empty_object);
    match (call.collection.is_some(), call.method.name.as_str()) {
        (true, "deleteMany") if matches_everything => {
            Some("deleteMany with an empty filter deletes every document in the collection.")
        }
        (true, "remove") if matches_everything => {
            Some("remove with an empty filter deletes every document in the collection.")
        }
        (true, "updateMany") if matches_everything => {
            Some("updateMany with an empty filter changes every document in the collection.")
        }
        (true, "drop") => Some("drop() deletes the collection, with its documents and indexes."),
        (false, "dropDatabase") => {
            Some("dropDatabase() deletes the database, with every collection in it.")
        }
        _ => None,
    }
}
