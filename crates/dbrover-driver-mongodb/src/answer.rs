//! A command's reply as the grid shows it, the way the shell prints the method's result: a
//! count as a number, a write as what it changed, anything else as the reply itself.

use bson::{Bson, Document, doc};

use crate::convert::cell;
use crate::cursor::Table;
use crate::plan::Answer;

/// Why a reply that the server accepted still reports a failure: a write it refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub(crate) struct WriteFailed(pub(crate) String);

/// `reply` shown through `answer`.
///
/// # Errors
///
/// [`WriteFailed`] when the reply holds `writeErrors` or a `writeConcernError`: the command ran,
/// but some of its writes, or the wait for them to be acknowledged, failed.
pub(crate) fn table(answer: &Answer, reply: Document) -> Result<Table, WriteFailed> {
    write_failure(&reply)?;
    Ok(match answer {
        Answer::Reply => Table::documents(vec![reply]),
        Answer::Count => Table::rows(
            &["count"],
            vec![vec![reply.get("n").cloned().and_then(cell)]],
        ),
        Answer::CountDocuments => {
            let count = first_batch(&reply)
                .first()
                .and_then(|document| document.get("n").cloned())
                .unwrap_or(Bson::Int32(0));
            Table::rows(&["count"], vec![vec![cell(count)]])
        }
        Answer::Distinct(field) => {
            let values = match reply.get("values") {
                Some(Bson::Array(values)) => values.clone(),
                _ => Vec::new(),
            };
            Table::rows(
                &[field.as_str()],
                values.into_iter().map(|value| vec![cell(value)]).collect(),
            )
        }
        Answer::Inserted { ids, many } => {
            let inserted = count(&reply, "n");
            let document = if *many {
                let ids: Document = ids
                    .iter()
                    .enumerate()
                    .map(|(index, id)| (index.to_string(), id.clone()))
                    .collect();
                doc! { "acknowledged": true, "insertedIds": ids }
            } else {
                let id = ids.first().cloned().unwrap_or(Bson::Null);
                doc! { "acknowledged": true, "insertedId": id }
            };
            Table::documents(vec![document]).changing(inserted)
        }
        Answer::Updated => {
            let upserted = match reply.get("upserted") {
                Some(Bson::Array(upserted)) => upserted.clone(),
                _ => Vec::new(),
            };
            let upserted_count = upserted.len() as u64;
            let modified = count(&reply, "nModified");
            let matched = count(&reply, "n").saturating_sub(upserted_count);
            let mut document = doc! {
                "acknowledged": true,
                "matchedCount": to_bson(matched),
                "modifiedCount": to_bson(modified),
                "upsertedCount": to_bson(upserted_count),
            };
            if let Some(Bson::Document(first)) = upserted.first()
                && let Some(id) = first.get("_id")
            {
                document.insert("upsertedId", id.clone());
            }
            Table::documents(vec![document]).changing(modified + upserted_count)
        }
        Answer::Deleted => {
            let deleted = count(&reply, "n");
            Table::documents(vec![
                doc! { "acknowledged": true, "deletedCount": to_bson(deleted) },
            ])
            .changing(deleted)
        }
        Answer::Modified => {
            let changed = match reply.get("lastErrorObject") {
                Some(Bson::Document(last)) => count(last, "n"),
                _ => 0,
            };
            let documents = match reply.get("value") {
                Some(Bson::Document(value)) => vec![value.clone()],
                _ => Vec::new(),
            };
            Table::documents(documents).changing(changed)
        }
        Answer::Indexes(names) => Table::rows(
            &["index"],
            names.iter().map(|name| vec![Some(name.clone())]).collect(),
        ),
        Answer::Databases => {
            let databases = match reply.get("databases") {
                Some(Bson::Array(databases)) => databases.clone(),
                _ => Vec::new(),
            };
            let rows = databases
                .into_iter()
                .filter_map(|database| match database {
                    Bson::Document(mut database) => Some(vec![
                        database.remove("name").and_then(cell),
                        database.remove("sizeOnDisk").and_then(cell),
                    ]),
                    _ => None,
                })
                .collect();
            Table::rows(&["name", "size on disk"], rows)
        }
        Answer::Field { field, column } => Table::rows(
            &[column],
            vec![vec![reply.get(*field).cloned().and_then(cell)]],
        ),
    })
}

/// The documents of a reply's first batch, for a cursor command read whole.
fn first_batch(reply: &Document) -> Vec<Document> {
    let Ok(cursor) = reply.get_document("cursor") else {
        return Vec::new();
    };
    match cursor.get("firstBatch") {
        Some(Bson::Array(batch)) => batch
            .iter()
            .filter_map(|document| document.as_document().cloned())
            .collect(),
        _ => Vec::new(),
    }
}

/// A count in a reply, whichever integer type the server sent it as.
fn count(reply: &Document, field: &str) -> u64 {
    match reply.get(field) {
        Some(Bson::Int32(count)) => u64::try_from(*count).unwrap_or_default(),
        Some(Bson::Int64(count)) => u64::try_from(*count).unwrap_or_default(),
        _ => 0,
    }
}

/// A count as the shell shows it, an `int` when it fits.
fn to_bson(count: u64) -> Bson {
    match i32::try_from(count) {
        Ok(count) => Bson::Int32(count),
        Err(_) => Bson::Int64(i64::try_from(count).unwrap_or(i64::MAX)),
    }
}

fn write_failure(reply: &Document) -> Result<(), WriteFailed> {
    if let Some(Bson::Array(errors)) = reply.get("writeErrors")
        && let Some(Bson::Document(first)) = errors.first()
    {
        let message = first.get_str("errmsg").unwrap_or("a write failed");
        let more = match errors.len() {
            1 => String::new(),
            count => format!(" ({} more writes failed)", count - 1),
        };
        return Err(WriteFailed(format!("{message}{more}")));
    }
    if let Ok(error) = reply.get_document("writeConcernError") {
        let message = error
            .get_str("errmsg")
            .unwrap_or("the write concern failed");
        return Err(WriteFailed(message.to_owned()));
    }
    Ok(())
}

/// Several writes' answers as one, as the shell shows `bulkWrite`'s result: what each kind of
/// write changed, summed.
#[derive(Debug, Default)]
pub(crate) struct BulkResult {
    inserted: u64,
    matched: u64,
    modified: u64,
    deleted: u64,
    upserted: u64,
    inserted_ids: Document,
}

impl BulkResult {
    /// Adds the reply of the write at `index`, sent to be shown through `answer`.
    pub(crate) fn add(&mut self, index: usize, answer: &Answer, reply: &Document) {
        match answer {
            Answer::Inserted { ids, .. } => {
                self.inserted += count(reply, "n");
                if let Some(id) = ids.first() {
                    self.inserted_ids.insert(index.to_string(), id.clone());
                }
            }
            Answer::Updated => {
                let upserted = match reply.get("upserted") {
                    Some(Bson::Array(upserted)) => upserted.len() as u64,
                    _ => 0,
                };
                self.upserted += upserted;
                self.matched += count(reply, "n").saturating_sub(upserted);
                self.modified += count(reply, "nModified");
            }
            Answer::Deleted => self.deleted += count(reply, "n"),
            _ => {}
        }
    }

    pub(crate) fn table(self) -> Table {
        let changed = self.inserted + self.modified + self.deleted + self.upserted;
        let document = doc! {
            "acknowledged": true,
            "insertedCount": to_bson(self.inserted),
            "insertedIds": self.inserted_ids,
            "matchedCount": to_bson(self.matched),
            "modifiedCount": to_bson(self.modified),
            "deletedCount": to_bson(self.deleted),
            "upsertedCount": to_bson(self.upserted),
        };
        Table::documents(vec![document]).changing(changed)
    }
}

/// Checks a write's reply in a bulk write, as [`table`] does.
///
/// # Errors
///
/// [`WriteFailed`], as [`table`].
pub(crate) fn check_write(reply: &Document) -> Result<(), WriteFailed> {
    write_failure(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(table: Table) -> Vec<Vec<Option<String>>> {
        table.rows.into_iter().collect()
    }

    #[test]
    fn a_write_shows_what_it_changed() {
        let inserted = table(
            &Answer::Inserted {
                ids: vec![Bson::Int32(1), Bson::Int32(2)],
                many: true,
            },
            doc! { "n": 2, "ok": 1.0 },
        )
        .unwrap();
        assert_eq!(inserted.changed, Some(2));
        assert_eq!(
            rows(inserted),
            [[Some(
                r#"{"acknowledged":true,"insertedIds":{"0":{"$numberInt":"1"},"1":{"$numberInt":"2"}}}"#
                    .to_owned()
            )]]
        );
        let updated = table(
            &Answer::Updated,
            doc! { "n": 3, "nModified": 2, "upserted": [{ "index": 0, "_id": 9 }], "ok": 1.0 },
        )
        .unwrap();
        assert_eq!(updated.changed, Some(3));
    }

    #[test]
    fn a_refused_write_is_an_error() {
        let reply = doc! {
            "n": 0,
            "writeErrors": [{ "index": 0, "code": 11000, "errmsg": "E11000 duplicate key" }],
            "ok": 1.0,
        };
        assert_eq!(
            table(&Answer::Deleted, reply).err(),
            Some(WriteFailed("E11000 duplicate key".to_owned()))
        );
    }

    #[test]
    fn counts_and_distinct_values_are_columns() {
        assert_eq!(
            rows(table(&Answer::Count, doc! { "n": 42, "ok": 1.0 }).unwrap()),
            [[Some("42".to_owned())]]
        );
        let empty = doc! { "cursor": { "firstBatch": [], "id": 0_i64 }, "ok": 1.0 };
        assert_eq!(
            rows(table(&Answer::CountDocuments, empty).unwrap()),
            [[Some("0".to_owned())]]
        );
        let distinct = table(
            &Answer::Distinct("status".to_owned()),
            doc! { "values": ["open", "closed", Bson::Null], "ok": 1.0 },
        )
        .unwrap();
        assert_eq!(distinct.columns[0].name, "status");
        assert_eq!(
            rows(distinct),
            [
                vec![Some("open".to_owned())],
                vec![Some("closed".to_owned())],
                vec![None]
            ]
        );
    }
}
