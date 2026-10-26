//! Fields the helpers' forms share.

use dbdesk_core::{CatalogPath, FolderKind, NodeKind};
use egui::{Ui, WidgetType};

use crate::catalog_tree::CatalogTree;
use crate::widgets;

/// The columns of a key or index, in the order they were ticked, which is its order.
pub(super) fn column_picker(
    ui: &mut Ui,
    chosen: &mut Vec<String>,
    columns: &[String],
    label: &str,
) {
    let text = if chosen.is_empty() {
        "Choose…".to_owned()
    } else {
        chosen.join(", ")
    };
    let menu = ui.menu_button(text, |ui| {
        if columns.is_empty() {
            ui.weak("No columns yet");
        }
        for column in columns {
            let mut ticked = chosen.contains(column);
            if ui.checkbox(&mut ticked, column).changed() {
                if ticked {
                    chosen.push(column.clone());
                } else {
                    chosen.retain(|other| other != column);
                }
            }
        }
    });
    widgets::name(menu.response, WidgetType::Button, label, true);
}

pub(super) fn columns_folder(table: &CatalogPath) -> CatalogPath {
    table.child(
        NodeKind::Folder(FolderKind::Columns),
        FolderKind::Columns.label(),
    )
}

/// A table's column names, once its columns folder is loaded.
pub(super) fn loaded_columns(catalog: &CatalogTree, table: &CatalogPath) -> Option<Vec<String>> {
    let columns = catalog.loaded_children(&columns_folder(table))?;
    Some(
        columns
            .iter()
            .filter(|column| column.kind() == Some(NodeKind::Column))
            .map(|column| column.label.clone())
            .collect(),
    )
}
