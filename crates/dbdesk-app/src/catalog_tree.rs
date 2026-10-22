//! What the explorer knows about one data source's catalog: which nodes are expanded and
//! which children have been loaded. Pure state; the explorer draws it and the app turns its
//! "needs loading" answers into engine commands.

use std::collections::{HashMap, HashSet};

use dbdesk_core::{CatalogNode, CatalogPath};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CatalogTree {
    expanded: HashSet<CatalogPath>,
    children: HashMap<CatalogPath, Children>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Children {
    Loading,
    Loaded(Vec<CatalogNode>),
    Failed(String),
}

impl CatalogTree {
    pub fn is_expanded(&self, path: &CatalogPath) -> bool {
        self.expanded.contains(path)
    }

    pub fn children(&self, path: &CatalogPath) -> Option<&Children> {
        self.children.get(path)
    }

    /// Expands `path`. Returns `true` if its children must be requested: they were never
    /// loaded, or the last attempt failed.
    #[must_use]
    pub fn expand(&mut self, path: &CatalogPath) -> bool {
        self.expanded.insert(path.clone());
        match self.children.get(path) {
            Some(Children::Loading | Children::Loaded(_)) => false,
            Some(Children::Failed(_)) | None => {
                self.children.insert(path.clone(), Children::Loading);
                true
            }
        }
    }

    pub fn collapse(&mut self, path: &CatalogPath) {
        self.expanded.remove(path);
    }

    /// Forgets everything loaded at and below `path` and collapses its descendants. Returns
    /// `true` if `path` itself is expanded, so its children must be requested again.
    #[must_use]
    pub fn refresh(&mut self, path: &CatalogPath) -> bool {
        self.children.retain(|loaded, _| !loaded.starts_with(path));
        self.expanded
            .retain(|expanded| expanded == path || !expanded.starts_with(path));
        if self.is_expanded(path) {
            self.children.insert(path.clone(), Children::Loading);
            true
        } else {
            false
        }
    }

    /// Records children that arrived from the engine. Ignored unless they were being waited
    /// for, so a late answer can't resurrect a subtree that was reset in the meantime.
    pub fn loaded(&mut self, path: &CatalogPath, result: &Result<Vec<CatalogNode>, String>) {
        if let Some(children @ Children::Loading) = self.children.get_mut(path) {
            *children = match result {
                Ok(nodes) => Children::Loaded(nodes.clone()),
                Err(error) => Children::Failed(error.clone()),
            };
        }
    }

    /// Forgets everything, e.g. after the connection closed.
    pub fn reset(&mut self) {
        self.expanded.clear();
        self.children.clear();
    }
}

#[cfg(test)]
mod tests {
    use dbdesk_core::NodeKind;

    use super::*;

    fn schema(name: &str) -> CatalogPath {
        CatalogPath::root().child(NodeKind::Schema, name)
    }

    fn schema_node(name: &str) -> CatalogNode {
        CatalogNode {
            path: schema(name),
            label: name.to_owned(),
            detail: None,
            has_children: true,
            has_definition: false,
            system: false,
        }
    }

    #[test]
    fn requests_children_once_and_again_after_a_failure() {
        let mut tree = CatalogTree::default();
        let root = CatalogPath::root();

        assert!(tree.expand(&root));
        assert!(!tree.expand(&root));
        tree.loaded(&root, &Err("timed out".to_owned()));
        tree.collapse(&root);

        assert!(tree.expand(&root));
    }

    #[test]
    fn refresh_forgets_the_subtree_and_reloads_only_if_expanded() {
        let mut tree = CatalogTree::default();
        let root = CatalogPath::root();
        assert!(tree.expand(&root));
        tree.loaded(&root, &Ok(vec![schema_node("main")]));
        assert!(tree.expand(&schema("main")));
        tree.loaded(&schema("main"), &Ok(vec![]));

        assert!(tree.refresh(&root));

        assert_eq!(tree.children(&root), Some(&Children::Loading));
        assert_eq!(tree.children(&schema("main")), None);
        assert!(!tree.is_expanded(&schema("main")));
        assert!(!tree.refresh(&schema("other")));
    }

    #[test]
    fn ignores_answers_nobody_is_waiting_for() {
        let mut tree = CatalogTree::default();
        let root = CatalogPath::root();
        assert!(tree.expand(&root));
        tree.reset();

        tree.loaded(&root, &Ok(vec![schema_node("main")]));

        assert_eq!(tree.children(&root), None);
    }
}
