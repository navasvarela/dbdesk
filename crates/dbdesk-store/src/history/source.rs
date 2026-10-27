//! Where a statement in the history came from, as typed provenance and as the columns that
//! store it.

use std::path::PathBuf;

/// Who sent a statement, and from where in dbdesk.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HistorySource {
    /// A query console's run.
    Console,
    /// The grid's Submit: one `UPDATE`, `INSERT` or `DELETE` per row.
    GridEdit,
    /// One of the explorer's object helpers: creating, changing, renaming, dropping or copying
    /// a table, view or schema.
    ObjectHelper,
    /// A CSV import into a table, recorded once for the whole file.
    Import { file: PathBuf },
    /// Export › All rows: the statement the rows were read with.
    Export,
    /// An MCP client, through `dbdesk mcp`, with the name it gave when it connected.
    Mcp { client: Option<String> },
    /// dbdesk, before it recorded where a statement came from: rows written before the history
    /// had provenance, or since then by an older dbdesk sharing the file.
    Dbdesk,
}

/// What [`HistorySource`] is, without its details: what the history is filtered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistorySourceKind {
    Console,
    GridEdit,
    ObjectHelper,
    Import,
    Export,
    Mcp,
    Dbdesk,
}

impl HistorySourceKind {
    pub const ALL: [Self; 7] = [
        Self::Console,
        Self::GridEdit,
        Self::ObjectHelper,
        Self::Import,
        Self::Export,
        Self::Mcp,
        Self::Dbdesk,
    ];

    /// The kind as `coalesce(origin, source)` reads in the database. `app` and `mcp` are the
    /// values of `source` from before `origin` existed, so a row an older dbdesk wrote reads as
    /// [`Self::Dbdesk`] or [`Self::Mcp`].
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Console => "console",
            Self::GridEdit => "grid",
            Self::ObjectHelper => "object",
            Self::Import => "import",
            Self::Export => "export",
            Self::Mcp => "mcp",
            Self::Dbdesk => "app",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == text)
    }
}

impl HistorySource {
    pub fn kind(&self) -> HistorySourceKind {
        match self {
            Self::Console => HistorySourceKind::Console,
            Self::GridEdit => HistorySourceKind::GridEdit,
            Self::ObjectHelper => HistorySourceKind::ObjectHelper,
            Self::Import { .. } => HistorySourceKind::Import,
            Self::Export => HistorySourceKind::Export,
            Self::Mcp { .. } => HistorySourceKind::Mcp,
            Self::Dbdesk => HistorySourceKind::Dbdesk,
        }
    }

    /// The `source` column, the only one a dbdesk from before provenance reads: it parses
    /// `app` and `mcp`, and skips a row with anything else.
    pub(super) fn legacy_source(&self) -> &'static str {
        match self {
            Self::Mcp { .. } => HistorySourceKind::Mcp.as_str(),
            _ => HistorySourceKind::Dbdesk.as_str(),
        }
    }

    /// The `origin` column; `NULL` for [`Self::Dbdesk`], as an older dbdesk leaves it.
    pub(super) fn origin(&self) -> Option<&'static str> {
        match self {
            Self::Dbdesk => None,
            other => Some(other.kind().as_str()),
        }
    }

    /// The `detail` column.
    pub(super) fn detail(&self) -> Option<String> {
        match self {
            Self::Import { file } => Some(file.to_string_lossy().into_owned()),
            Self::Mcp { client } => client.clone(),
            _ => None,
        }
    }

    /// The source a row stores, from `coalesce(origin, source)` and `detail`.
    pub(super) fn from_stored(kind: &str, detail: Option<String>) -> Option<Self> {
        Some(match HistorySourceKind::parse(kind)? {
            HistorySourceKind::Console => Self::Console,
            HistorySourceKind::GridEdit => Self::GridEdit,
            HistorySourceKind::ObjectHelper => Self::ObjectHelper,
            HistorySourceKind::Import => Self::Import {
                file: PathBuf::from(detail.unwrap_or_default()),
            },
            HistorySourceKind::Export => Self::Export,
            HistorySourceKind::Mcp => Self::Mcp { client: detail },
            HistorySourceKind::Dbdesk => Self::Dbdesk,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_reads_back_as_stored() {
        let sources = [
            HistorySource::Console,
            HistorySource::GridEdit,
            HistorySource::ObjectHelper,
            HistorySource::Import {
                file: PathBuf::from("/data/orders.csv"),
            },
            HistorySource::Export,
            HistorySource::Mcp {
                client: Some("claude-ai".to_owned()),
            },
            HistorySource::Mcp { client: None },
            HistorySource::Dbdesk,
        ];
        for source in sources {
            let kind = source.origin().unwrap_or(source.legacy_source());
            assert_eq!(
                HistorySource::from_stored(kind, source.detail()),
                Some(source.clone())
            );
        }
    }

    #[test]
    fn older_readers_see_only_app_and_mcp() {
        for kind in HistorySourceKind::ALL {
            let source = HistorySource::from_stored(kind.as_str(), None).unwrap();
            assert!(["app", "mcp"].contains(&source.legacy_source()), "{kind:?}");
        }
    }
}
