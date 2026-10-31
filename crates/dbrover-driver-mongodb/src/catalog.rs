//! The explorer's tree of a MongoDB server: its databases, the collections, views and
//! time-series collections in each, and a collection's indexes, with each collection's
//! estimated document count and the commands that make it again.

use std::collections::HashMap;

use bson::{Bson, Document, doc};
use dbrover_core::{
    CatalogNode, CatalogPath, ChildrenPage, ChildrenQuery, DriverError, NodeKind, PageToken,
};
use dbrover_language::mongo::quote;
use mongodb::{Client, Database};
use tokio::task::JoinSet;

use crate::errors;
use crate::shell;

/// Databases the server keeps for itself, which the explorer hides unless asked.
const SYSTEM_DATABASES: [&str; 3] = ["admin", "config", "local"];

/// The index every collection has on `_id`, which the collection makes itself.
const ID_INDEX: &str = "_id_";

/// What the explorer lists at a path.
pub(crate) enum Level<'a> {
    Databases,
    /// The collections of a database.
    Collections {
        database: &'a str,
    },
    /// The indexes of a collection.
    Indexes {
        database: &'a str,
        collection: &'a str,
    },
}

impl<'a> Level<'a> {
    pub(crate) fn of(path: &'a CatalogPath) -> Option<Self> {
        match path.segments() {
            [] => Some(Self::Databases),
            [database] if database.kind == NodeKind::Database => Some(Self::Collections {
                database: &database.name,
            }),
            [database, collection]
                if database.kind == NodeKind::Database
                    && collection.kind == NodeKind::Collection =>
            {
                Some(Self::Indexes {
                    database: &database.name,
                    collection: &collection.name,
                })
            }
            _ => None,
        }
    }
}

/// The databases the user may list, by name.
pub(crate) async fn databases(client: &Client) -> Result<Vec<CatalogNode>, DriverError> {
    let reply = client
        .database("admin")
        .run_command(doc! { "listDatabases": 1, "nameOnly": true, "authorizedDatabases": true })
        .await
        .map_err(errors::catalog)?;
    let mut names: Vec<String> = match reply.get("databases") {
        Some(Bson::Array(databases)) => databases
            .iter()
            .filter_map(|database| database.as_document()?.get_str("name").ok())
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    };
    names.sort();
    Ok(names
        .into_iter()
        .map(|name| CatalogNode {
            path: CatalogPath::root().child(NodeKind::Database, &name),
            system: SYSTEM_DATABASES.contains(&name.as_str()),
            label: name,
            detail: None,
            has_children: true,
            has_definition: false,
        })
        .collect())
}

/// A page of a database's collections, by name: their names are listed whole (cheap even for
/// thousands), sorted, and the page's own collections then read with their options and
/// counted. The filter is a case-insensitive `listCollections` filter on the name.
pub(crate) async fn collections(
    database: &Database,
    path: &CatalogPath,
    query: &ChildrenQuery,
) -> Result<ChildrenPage, DriverError> {
    let mut filter = Document::new();
    if let Some(text) = query.filter.as_deref().filter(|text| !text.is_empty()) {
        filter.insert(
            "name",
            doc! { "$regex": regex_escape(text), "$options": "i" },
        );
    }
    let listed = list(
        database,
        doc! { "listCollections": 1, "filter": filter, "nameOnly": true, "authorizedCollections": true },
    )
    .await?;
    let mut names: Vec<String> = listed
        .iter()
        .filter_map(|collection| collection.get_str("name").ok())
        .map(str::to_owned)
        .collect();
    names.sort();
    let after = query.after.as_ref().map(PageToken::as_str);
    names.retain(|name| after.is_none_or(|after| name.as_str() > after));
    let limit = query.limit.unwrap_or(usize::MAX);
    let next = (names.len() > limit).then(|| {
        names.truncate(limit);
        names.last().map(|last| PageToken::new(last.clone()))
    });
    let infos = infos(database, &names).await?;
    let counts = estimated_counts(database, &infos).await;
    let nodes = names
        .into_iter()
        .map(|name| {
            let kind = infos
                .get(&name)
                .and_then(|info| info.get_str("type").ok())
                .unwrap_or("collection");
            collection_node(path, name.clone(), kind, counts.get(&name).copied())
        })
        .collect();
    Ok(ChildrenPage {
        nodes,
        next: next.flatten(),
    })
}

fn collection_node(
    parent: &CatalogPath,
    name: String,
    kind: &str,
    count: Option<u64>,
) -> CatalogNode {
    let count = count.map(|count| format!("~{} documents", thousands(count)));
    let detail = match (kind, count) {
        ("view", _) => Some("view".to_owned()),
        ("timeseries", _) => Some("time series".to_owned()),
        (_, count) => count,
    };
    CatalogNode {
        path: parent.child(NodeKind::Collection, &name),
        system: name.starts_with("system."),
        has_children: kind != "view",
        has_definition: true,
        label: name,
        detail,
    }
}

/// What `listCollections` says of each of `names`: its type and options.
async fn infos(
    database: &Database,
    names: &[String],
) -> Result<HashMap<String, Document>, DriverError> {
    if names.is_empty() {
        return Ok(HashMap::new());
    }
    let listed = list(
        database,
        doc! { "listCollections": 1, "filter": { "name": { "$in": names } } },
    )
    .await?;
    Ok(listed
        .into_iter()
        .filter_map(|info| Some((info.get_str("name").ok()?.to_owned(), info)))
        .collect())
}

/// Each plain collection's estimated document count, from its metadata (`count` without a
/// filter), never by reading its documents. Views and time-series collections are left out:
/// counting either reads every document, since each is a pipeline over another collection.
/// Asked of every collection at once, over the client's pool; one whose count fails is left
/// without a count.
async fn estimated_counts(
    database: &Database,
    infos: &HashMap<String, Document>,
) -> HashMap<String, u64> {
    let mut counts = JoinSet::new();
    for (name, info) in infos {
        if info.get_str("type").ok() != Some("collection") {
            continue;
        }
        let database = database.clone();
        let name = name.clone();
        counts.spawn(async move {
            let reply = database.run_command(doc! { "count": &name }).await.ok()?;
            let count = match reply.get("n")? {
                Bson::Int32(count) => u64::try_from(*count).ok()?,
                Bson::Int64(count) => u64::try_from(*count).ok()?,
                _ => return None,
            };
            Some((name, count))
        });
    }
    let mut found = HashMap::new();
    while let Some(counted) = counts.join_next().await {
        if let Ok(Some((name, count))) = counted {
            found.insert(name, count);
        }
    }
    found
}

/// A collection's indexes, in the order the server lists them, each with its keys.
pub(crate) async fn indexes(
    database: &Database,
    collection: &str,
    path: &CatalogPath,
) -> Result<Vec<CatalogNode>, DriverError> {
    let listed = list(database, doc! { "listIndexes": collection, "cursor": {} }).await?;
    Ok(listed
        .into_iter()
        .filter_map(|index| {
            let name = index.get_str("name").ok()?.to_owned();
            let mut detail = index
                .get_document("key")
                .map(shell::document)
                .unwrap_or_default();
            if index.get_bool("unique").unwrap_or(false) {
                detail.push_str(" · unique");
            }
            Some(CatalogNode {
                path: path.child(NodeKind::Index, &name),
                label: name,
                detail: Some(detail),
                has_children: false,
                has_definition: false,
                system: false,
            })
        })
        .collect())
}

/// The commands that make the collection or view again: `db.createCollection(…)` with its
/// options and validator, or `db.createView(…)`, then a `createIndex(…)` for each index but
/// `_id`'s.
pub(crate) async fn definition(
    database: &Database,
    collection: &str,
) -> Result<Option<String>, DriverError> {
    let listed = list(
        database,
        doc! { "listCollections": 1, "filter": { "name": collection } },
    )
    .await?;
    let Some(info) = listed.into_iter().next() else {
        return Ok(None);
    };
    let mut options = info.get_document("options").cloned().unwrap_or_default();
    let name = quote(collection);
    if info.get_str("type").ok() == Some("view") {
        let source = options.remove("viewOn").unwrap_or(Bson::Null);
        let pipeline = options
            .remove("pipeline")
            .unwrap_or_else(|| Bson::Array(Vec::new()));
        let rest = if options.is_empty() {
            String::new()
        } else {
            format!(", {}", shell::document(&options))
        };
        return Ok(Some(format!(
            "db.createView({name}, {}, {}{rest});\n",
            shell::value(&source),
            shell::value(&pipeline)
        )));
    }
    let mut text = if options.is_empty() {
        format!("db.createCollection({name});\n")
    } else {
        format!(
            "db.createCollection({name}, {});\n",
            shell::document(&options)
        )
    };
    for mut index in list(database, doc! { "listIndexes": collection, "cursor": {} }).await? {
        if index.get_str("name").ok() == Some(ID_INDEX) {
            continue;
        }
        let key = index.remove("key").unwrap_or(Bson::Null);
        // The index's version is the server's to choose.
        index.remove("v");
        text.push_str(&format!(
            "db.getCollection({name}).createIndex({}, {});\n",
            shell::value(&key),
            shell::document(&index)
        ));
    }
    Ok(Some(text))
}

/// Every document a cursor command answers, read to its end.
async fn list(database: &Database, command: Document) -> Result<Vec<Document>, DriverError> {
    let mut cursor = database
        .run_cursor_command(command)
        .await
        .map_err(errors::catalog)?;
    let mut documents = Vec::new();
    while cursor.advance().await.map_err(errors::catalog)? {
        documents.push(cursor.deserialize_current().map_err(errors::catalog)?);
    }
    Ok(documents)
}

/// `text` as a regular expression matching it literally.
fn regex_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// `1234567` as `1,234,567`.
fn thousands(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_in_thousands() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_200), "1,200");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn a_filter_matches_its_text_literally() {
        assert_eq!(regex_escape("a.b*"), "a\\.b\\*");
    }
}
