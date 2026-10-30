//! The explorer's top level: the databases that hold keys. Their keys are in `keys`.

use dbrover_core::{CatalogNode, CatalogPath, NodeKind};

/// The databases `INFO keyspace` lists, with their key counts, as `(number, keys)` in the
/// order listed: lines such as `db0:keys=12,expires=0,avg_ttl=0`. An empty database isn't
/// listed.
pub(crate) fn keyspace(info: &str) -> Vec<(i64, u64)> {
    info.lines()
        .filter_map(|line| {
            let (database, fields) = line.trim().split_once(':')?;
            let number = database.strip_prefix("db")?.parse().ok()?;
            let keys = fields
                .split(',')
                .find_map(|field| field.strip_prefix("keys="))?
                .parse()
                .ok()?;
            Some((number, keys))
        })
        .collect()
}

/// The databases to show: those that hold keys, and the data source's own even when it is
/// empty, so there is always one to open; in number order.
pub(crate) fn databases(info: &str, own: i64) -> Vec<CatalogNode> {
    let mut databases = keyspace(info);
    if !databases.iter().any(|(number, _)| *number == own) {
        databases.push((own, 0));
    }
    databases.sort_unstable();
    databases
        .into_iter()
        .map(|(number, keys)| CatalogNode {
            path: CatalogPath::root().child(NodeKind::Database, number.to_string()),
            label: format!("db{number}"),
            detail: Some(match keys {
                1 => "1 key".to_owned(),
                keys => format!("{keys} keys"),
            }),
            has_children: true,
            has_definition: false,
            system: false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "# Keyspace\r\ndb0:keys=12,expires=1,avg_ttl=0,subexpiry=0\r\n\
                        db3:keys=1,expires=0,avg_ttl=0\r\n";

    #[test]
    fn reads_the_key_counts_of_info_keyspace() {
        assert_eq!(keyspace(INFO), [(0, 12), (3, 1)]);
        assert_eq!(keyspace("# Keyspace\r\n"), []);
    }

    #[test]
    fn lists_the_databases_with_keys_and_the_data_sources_own() {
        let labels = |own| -> Vec<(String, Option<String>)> {
            databases(INFO, own)
                .into_iter()
                .map(|node| (node.label, node.detail))
                .collect()
        };
        assert_eq!(
            labels(2),
            [
                ("db0".to_owned(), Some("12 keys".to_owned())),
                ("db2".to_owned(), Some("0 keys".to_owned())),
                ("db3".to_owned(), Some("1 key".to_owned())),
            ]
        );
        assert_eq!(labels(0).len(), 2);
        assert_eq!(
            databases(INFO, 0)[0].path,
            CatalogPath::root().child(NodeKind::Database, "0")
        );
    }
}
