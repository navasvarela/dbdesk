//! What the result grid shows of a statement's loaded rows: a client-side sort and filter,
//! and the selected cells. Rows are never re-fetched for either; both work on what has been
//! loaded so far, and the grid says so.

use std::cmp::Ordering;
use std::ops::RangeInclusive;

use dbdesk_core::{Cell, Row};

/// The text copied or exported for SQL `NULL`.
const NULL_TEXT: &str = "NULL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    pub column: usize,
    pub descending: bool,
}

/// A cell by its position on screen: the row's place in the sorted, filtered order, and the
/// column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPos {
    pub row: usize,
    pub column: usize,
}

/// A rectangle of cells, from where the selection started to where it ends now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Selection {
    anchor: CellPos,
    focus: CellPos,
}

impl Selection {
    fn rows(&self) -> RangeInclusive<usize> {
        self.anchor.row.min(self.focus.row)..=self.anchor.row.max(self.focus.row)
    }

    fn columns(&self) -> RangeInclusive<usize> {
        self.anchor.column.min(self.focus.column)..=self.anchor.column.max(self.focus.column)
    }
}

#[derive(Debug, Default)]
pub struct GridView {
    sort: Option<SortKey>,
    filter: String,
    /// Indices into the loaded rows, in display order.
    order: Vec<usize>,
    /// What `order` was computed from: the number of rows, the sort and the filter. Rows
    /// are only ever appended, so their count tells whether they changed.
    computed_for: Option<(usize, Option<SortKey>, String)>,
    selection: Option<Selection>,
    /// A drag selection is in progress.
    dragging: bool,
}

impl GridView {
    pub fn sort(&self) -> Option<SortKey> {
        self.sort
    }

    pub fn filter_mut(&mut self) -> &mut String {
        &mut self.filter
    }

    /// Whether the rows are shown in a different order or number than loaded.
    pub fn is_rearranged(&self) -> bool {
        self.sort.is_some() || !self.filter.trim().is_empty()
    }

    /// Cycles the column through ascending, descending and unsorted.
    pub fn toggle_sort(&mut self, column: usize) {
        self.sort = match self.sort {
            Some(SortKey {
                column: sorted,
                descending: false,
            }) if sorted == column => Some(SortKey {
                column,
                descending: true,
            }),
            Some(SortKey {
                column: sorted,
                descending: true,
            }) if sorted == column => None,
            _ => Some(SortKey {
                column,
                descending: false,
            }),
        };
    }

    /// Recomputes the display order if the rows, sort or filter changed.
    pub fn refresh(&mut self, rows: &[Row]) {
        let key = (rows.len(), self.sort, self.filter.clone());
        if self.computed_for.as_ref() == Some(&key) {
            return;
        }
        let rearranged = self
            .computed_for
            .as_ref()
            .is_some_and(|(_, sort, filter)| *sort != self.sort || *filter != self.filter);
        if rearranged {
            // Positions on screen now hold other rows.
            self.selection = None;
        }
        self.order = display_order(rows, self.sort, &self.filter);
        self.computed_for = Some(key);
    }

    /// Indices into the loaded rows, in display order, as of the last [`GridView::refresh`].
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    pub fn is_selected(&self, cell: CellPos) -> bool {
        self.selection.is_some_and(|selection| {
            selection.rows().contains(&cell.row) && selection.columns().contains(&cell.column)
        })
    }

    /// Starts a new selection at `cell`, as a click or the start of a drag does.
    pub fn select(&mut self, cell: CellPos) {
        self.selection = Some(Selection {
            anchor: cell,
            focus: cell,
        });
    }

    /// Stretches the selection to `cell`, as shift-click and dragging do.
    pub fn extend_selection(&mut self, cell: CellPos) {
        match &mut self.selection {
            Some(selection) => selection.focus = cell,
            None => self.select(cell),
        }
    }

    pub fn start_drag(&mut self, cell: CellPos) {
        self.select(cell);
        self.dragging = true;
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn stop_drag(&mut self) {
        self.dragging = false;
    }

    /// The selected cells as tab-separated lines, rows in display order. `None` without a
    /// selection.
    pub fn selection_as_tsv(&self, rows: &[Row]) -> Option<String> {
        let selection = self.selection?;
        let lines: Vec<String> = selection
            .rows()
            .filter_map(|position| self.order.get(position).and_then(|index| rows.get(*index)))
            .map(|row| {
                selection
                    .columns()
                    .filter_map(|column| row.get(column))
                    .map(tsv_field)
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect();
        Some(lines.join("\n"))
    }

    /// The rows as shown, in display order, for exporting.
    pub fn shown_rows(&self, rows: &[Row]) -> Vec<Row> {
        display_order(rows, self.sort, &self.filter)
            .into_iter()
            .filter_map(|index| rows.get(index).cloned())
            .collect()
    }

    /// The single selected cell, for the cell viewer.
    pub fn selected_cell(&self) -> Option<CellPos> {
        self.selection
            .filter(|selection| selection.anchor == selection.focus)
            .map(|selection| selection.anchor)
    }
}

/// The loaded rows' indices that pass the filter, in sorted order.
fn display_order(rows: &[Row], sort: Option<SortKey>, filter: &str) -> Vec<usize> {
    let needle = filter.trim().to_lowercase();
    let mut order: Vec<usize> = (0..rows.len())
        .filter(|&index| needle.is_empty() || row_matches(&rows[index], &needle))
        .collect();
    if let Some(SortKey { column, descending }) = sort {
        // Stable, so equal values keep the order the database returned them in.
        order.sort_by(|&a, &b| {
            let ordering = compare_cells(rows[a].get(column), rows[b].get(column));
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }
    order
}

fn row_matches(row: &Row, needle: &str) -> bool {
    row.iter()
        .flatten()
        .any(|value| value.to_lowercase().contains(needle))
}

/// `NULL` first, then numbers by value when both cells look numeric, then text.
///
/// Cells are text (see ADR-0004), so "numeric" is a guess from the text, the same one the grid
/// uses to right-align: `9` sorts before `10` in a numeric column, and a column mixing numbers
/// and words sorts the numbers by value among themselves.
fn compare_cells(a: Option<&Cell>, b: Option<&Cell>) -> Ordering {
    let a = a.and_then(Option::as_deref);
    let b = b.and_then(Option::as_deref);
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a), Some(b)) => match (a.parse::<f64>(), b.parse::<f64>()) {
            (Ok(x), Ok(y)) => x.total_cmp(&y),
            _ => a.cmp(b),
        },
    }
}

/// A cell for TSV: values with tabs, line breaks or quotes are quoted the way spreadsheets
/// read them back, with quotes doubled.
fn tsv_field(cell: &Cell) -> String {
    let Some(value) = cell else {
        return NULL_TEXT.to_owned();
    };
    if value.contains(['\t', '\n', '\r', '"']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(values: &[&[Option<&str>]]) -> Vec<Row> {
        values
            .iter()
            .map(|row| row.iter().map(|cell| cell.map(str::to_owned)).collect())
            .collect()
    }

    fn cell(row: usize, column: usize) -> CellPos {
        CellPos { row, column }
    }

    #[test]
    fn sorts_numbers_by_value_with_null_first_and_cycles_back_to_unsorted() {
        let rows = rows(&[
            &[Some("10")],
            &[None],
            &[Some("9")],
            &[Some("b")],
            &[Some("a")],
        ]);
        let mut grid = GridView::default();

        let order = |grid: &mut GridView| {
            grid.refresh(&rows);
            grid.order().to_vec()
        };
        grid.toggle_sort(0);
        assert_eq!(order(&mut grid), [1, 2, 0, 4, 3]);
        grid.toggle_sort(0);
        assert_eq!(order(&mut grid), [3, 4, 0, 2, 1]);
        grid.toggle_sort(0);
        assert_eq!(order(&mut grid), [0, 1, 2, 3, 4]);
        assert!(!grid.is_rearranged());
    }

    #[test]
    fn filters_on_any_cell_ignoring_case_and_follows_new_rows() {
        let mut loaded = rows(&[&[Some("Alice"), Some("x")], &[Some("bob"), None]]);
        let mut grid = GridView::default();
        *grid.filter_mut() = " ALI ".to_owned();
        grid.refresh(&loaded);
        assert_eq!(grid.order(), [0]);
        assert!(grid.is_rearranged());

        loaded.push(vec![Some("x".to_owned()), Some("malicious".to_owned())]);
        grid.refresh(&loaded);

        assert_eq!(grid.order(), [0, 2]);
        assert_eq!(
            grid.shown_rows(&loaded),
            [loaded[0].clone(), loaded[2].clone()]
        );
    }

    #[test]
    fn copies_a_selected_rectangle_in_display_order_as_tsv() {
        let rows = rows(&[
            &[Some("1"), Some("one"), Some("x")],
            &[Some("2"), None, Some("tab\there")],
            &[Some("3"), Some("say \"hi\""), Some("z")],
        ]);
        let mut grid = GridView::default();
        grid.toggle_sort(0);
        grid.toggle_sort(0);
        grid.refresh(&rows);
        assert_eq!(grid.selection_as_tsv(&rows), None);

        grid.select(cell(0, 1));
        grid.extend_selection(cell(1, 2));

        assert_eq!(
            grid.selection_as_tsv(&rows).unwrap(),
            "\"say \"\"hi\"\"\"\tz\nNULL\t\"tab\there\""
        );
        assert!(grid.is_selected(cell(1, 1)));
        assert!(!grid.is_selected(cell(2, 1)));
        assert_eq!(grid.selected_cell(), None);
    }

    #[test]
    fn changing_the_order_clears_the_selection() {
        let rows = rows(&[&[Some("1")], &[Some("2")]]);
        let mut grid = GridView::default();
        grid.refresh(&rows);
        grid.select(cell(0, 0));
        assert_eq!(grid.selected_cell(), Some(cell(0, 0)));

        *grid.filter_mut() = "2".to_owned();
        grid.refresh(&rows);

        assert_eq!(grid.selected_cell(), None);
    }
}
