//! Which schemas a statement names, so a project can limit an assistant to some of them.
//!
//! This reads names, not effects: a function can still reach another schema, and a name in
//! a string (`nextval('other.seq')`) isn't seen. The project's database login is what makes
//! the limit airtight.

use std::collections::{BTreeSet, HashSet};

use sqlparser::ast::{CommentObject, Ident, ObjectName, ObjectType, Query, Statement};

/// One part of a qualified name, as written.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NamePart {
    pub value: String,
    /// Quoted names keep their case; unquoted ones fold to the dialect's.
    pub quoted: bool,
}

impl From<&Ident> for NamePart {
    fn from(ident: &Ident) -> Self {
        Self {
            value: ident.value.clone(),
            quoted: ident.quote_style.is_some(),
        }
    }
}

/// The schemas a statement names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamedSchemas {
    /// Every table, view and function the statement names was found.
    Known {
        /// The schemas names were qualified with (`analytics` in `analytics.events`).
        qualified: BTreeSet<NamePart>,
        /// Whether some name had no schema, so it resolves to the connection's default.
        unqualified: bool,
    },
    /// The statement names objects this analysis doesn't look for, such as a `GRANT`'s
    /// or a procedure's, so which schemas it touches is unknown.
    Unknown,
}

/// Gathers names while [`super::Classifier`] walks the tree.
#[derive(Debug)]
pub(super) struct NameCollector {
    /// Each name as a list of parts, `[schema, table]` or `[table]`.
    names: Vec<Vec<NamePart>>,
    /// Schemas a statement names as the object itself, such as `DROP SCHEMA`.
    schemas: Vec<NamePart>,
    /// `WITH` names, which look like unqualified tables where they are used.
    ctes: HashSet<String>,
    known: bool,
}

impl Default for NameCollector {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            schemas: Vec::new(),
            ctes: HashSet::new(),
            known: true,
        }
    }
}

impl NameCollector {
    /// Statements whose names sqlparser's visitor shows as relations are covered by
    /// [`Self::relation`]; a few others carry their names in fields of their own; any other
    /// statement makes the names unknown, so a limit on schemas refuses it rather than
    /// letting through what it can't check.
    pub(super) fn statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Query(_)
            | Statement::Explain { .. }
            | Statement::Insert(_)
            | Statement::Update(_)
            | Statement::Delete(_)
            | Statement::Merge(_)
            | Statement::Truncate(_)
            | Statement::CreateTable(_)
            | Statement::CreateView(_)
            | Statement::CreateIndex(_)
            | Statement::AlterTable(_) => {}
            Statement::Drop {
                object_type: ObjectType::Schema,
                names,
                ..
            } => {
                for name in names {
                    match name.0.as_slice() {
                        [part] => self.schemas.extend(part.as_ident().map(NamePart::from)),
                        _ => self.known = false,
                    }
                }
            }
            Statement::Drop {
                object_type:
                    ObjectType::Table
                    | ObjectType::View
                    | ObjectType::MaterializedView
                    | ObjectType::Index
                    | ObjectType::Sequence
                    | ObjectType::Type,
                names,
                table,
                ..
            } => {
                for name in names.iter().chain(table) {
                    self.relation(name);
                }
            }
            Statement::Comment {
                object_type: CommentObject::Column,
                object_name,
                ..
            } => {
                // `schema.table.column`: the column isn't a level of its own.
                let parts = &object_name.0[..object_name.0.len().saturating_sub(1)];
                self.relation(&ObjectName(parts.to_vec()));
            }
            Statement::Comment {
                object_type:
                    CommentObject::Table
                    | CommentObject::View
                    | CommentObject::MaterializedView
                    | CommentObject::Index
                    | CommentObject::Sequence
                    | CommentObject::Type,
                object_name,
                ..
            } => self.relation(object_name),
            _ => self.known = false,
        }
    }

    pub(super) fn query(&mut self, query: &Query) {
        let Some(with) = &query.with else {
            return;
        };
        for cte in &with.cte_tables {
            self.ctes.insert(cte.alias.name.value.to_lowercase());
        }
    }

    pub(super) fn relation(&mut self, name: &ObjectName) {
        let parts: Option<Vec<NamePart>> = name
            .0
            .iter()
            .map(|part| part.as_ident().map(NamePart::from))
            .collect();
        match parts {
            Some(parts) if !parts.is_empty() => self.names.push(parts),
            // A name computed by a function (Snowflake's `IDENTIFIER(…)`) can't be read.
            _ => self.known = false,
        }
    }

    /// A function counts only when qualified: an unqualified one is usually built in, and
    /// resolves through the search path like any other function.
    pub(super) fn function(&mut self, name: &ObjectName) {
        if name.0.len() > 1 {
            self.relation(name);
        }
    }

    /// A `WITH` name used as a table is left out wherever it appears in the statement,
    /// rather than only within its scope: a real table spelt like one of the statement's
    /// `WITH` names, unqualified, outside that `WITH`, would be missed. Writing it
    /// qualified, or with another name, is all it takes not to be.
    pub(super) fn finish(self) -> NamedSchemas {
        if !self.known {
            return NamedSchemas::Unknown;
        }
        let mut qualified: BTreeSet<NamePart> = self.schemas.into_iter().collect();
        let mut unqualified = false;
        for parts in self.names {
            match parts.as_slice() {
                [only] if self.ctes.contains(&only.value.to_lowercase()) => {}
                [_] => unqualified = true,
                [.., schema, _] => {
                    qualified.insert(schema.clone());
                }
                [] => {}
            }
        }
        NamedSchemas::Known {
            qualified,
            unqualified,
        }
    }
}
