//! What the result grid shows of a statement's loaded rows: a client-side sort and filter,
//! and the selected cells. Rows are never re-fetched for either; both work on what has been
//! loaded so far, and the grid says so. Sorted and filtered in the database instead, the rows
//! arrive in order, and the grid shows them as they are.

use std::cmp::Ordering;
use std::ops::RangeInclusive;

use dbrover_core::{Cell, Row};

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
    /// are appended as pages load, and only removed by deleting them from the grid, so their
    /// count tells whether they changed.
    computed_for: Option<(usize, Option<SortKey>, String)>,
    selection: Option<Selection>,
    /// A drag selection is in progress.
    dragging: bool,
    /// The sort and filter are the database's to apply, by running the statement again.
    in_database: bool,
}

impl GridView {
    /// A grid whose rows the database sorts and filters as `sort` and `filter` say.
    pub fn in_database(sort: Option<SortKey>, filter: String) -> Self {
        Self {
            sort,
            filter,
            in_database: true,
            ..Self::default()
        }
    }

    pub fn is_in_database(&self) -> bool {
        self.in_database
    }

    /// Switches between sorting and filtering here and in the database.
    pub fn set_in_database(&mut self, in_database: bool) {
        self.in_database = in_database;
        self.computed_for = None;
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn sort(&self) -> Option<SortKey> {
        self.sort
    }

    pub fn filter_mut(&mut self) -> &mut String {
        &mut self.filter
    }

    /// Whether the rows are shown in a different order or number than loaded.
    pub fn is_rearranged(&self) -> bool {
        !self.in_database && self.has_sort_or_filter()
    }

    pub fn has_sort_or_filter(&self) -> bool {
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

    /// Forgets the sort and the selection after the columns moved, as expanding a column of
    /// documents moves those after it: their indices would now point at other columns. The
    /// filter, which matches any column, stays.
    pub fn columns_moved(&mut self) {
        self.sort = None;
        self.selection = None;
        self.computed_for = None;
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
        let fewer = self
            .computed_for
            .as_ref()
            .is_some_and(|(count, _, _)| *count > rows.len());
        if rearranged || fewer {
            // Positions on screen now hold other rows: rows were rearranged, or deleted.
            self.selection = None;
        }
        self.order = self.arranged(rows);
        self.computed_for = Some(key);
    }

    /// The loaded rows' indices in display order: as loaded when the database sorted and
    /// filtered them.
    fn arranged(&self, rows: &[Row]) -> Vec<usize> {
        if self.in_database {
            (0..rows.len()).collect()
        } else {
            display_order(rows, self.sort, &self.filter)
        }
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
        self.shown_indices(rows)
            .into_iter()
            .filter_map(|index| rows.get(index).cloned())
            .collect()
    }

    /// The indices of the rows [`Self::shown_rows`] gives, in the same order.
    pub fn shown_indices(&self, rows: &[Row]) -> Vec<usize> {
        self.arranged(rows)
    }

    /// The selected columns, left to right.
    pub fn selected_columns(&self) -> Option<RangeInclusive<usize>> {
        self.selection.map(|selection| selection.columns())
    }

    /// The display positions of the selected rows, top to bottom.
    pub fn selected_rows(&self) -> Option<RangeInclusive<usize>> {
        self.selection.map(|selection| selection.rows())
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

/// `NULL` first, then cells that look numeric by value, then the rest as text.
///
/// Cells are text (see ADR-0004), so "numeric" is a guess from the text, the same one the grid
/// uses to right-align: `9` sorts before `10` in a numeric column. In a column mixing numbers
/// and words, the numbers come first.
///
/// Each cell's place is decided by the cell alone, never by what it is compared with, which
/// keeps this a total order. `sort_by` panics on a comparison that isn't one (since Rust 1.81).
/// The first version compared two numbers by value but a number and a word as text, which
/// goes round in circles on a mixed column (`9 < 10`, `"10" < "5x"`, `"5x" < "9"`). Sorting
/// such a column crashed the app.
fn compare_cells(a: Option<&Cell>, b: Option<&Cell>) -> Ordering {
    match (SortValue::of(a), SortValue::of(b)) {
        (SortValue::Number(x), SortValue::Number(y)) => x.total_cmp(&y),
        (SortValue::Text(x), SortValue::Text(y)) => x.cmp(y),
        (a, b) => a.group().cmp(&b.group()),
    }
}

/// How a cell sorts: its group, and its value within the group.
enum SortValue<'a> {
    Null,
    Number(f64),
    Text(&'a str),
}

impl<'a> SortValue<'a> {
    fn of(cell: Option<&'a Cell>) -> Self {
        match cell.and_then(Option::as_deref) {
            None => Self::Null,
            Some(text) => text.parse().map_or(Self::Text(text), Self::Number),
        }
    }

    /// Groups in ascending order.
    fn group(&self) -> u8 {
        match self {
            Self::Null => 0,
            Self::Number(_) => 1,
            Self::Text(_) => 2,
        }
    }
}

/// A cell for TSV: values with tabs, line breaks or quotes are quoted the way spreadsheets
/// read them back, with quotes doubled.
pub(super) fn tsv_field(cell: &Cell) -> String {
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
    fn shows_rows_as_loaded_when_the_database_sorts_and_filters_them() {
        let rows = vec![vec![Some("2".to_owned())], vec![Some("1".to_owned())]];
        let mut view = GridView::in_database(
            Some(SortKey {
                column: 0,
                descending: false,
            }),
            "2".to_owned(),
        );
        view.refresh(&rows);
        assert_eq!(view.order(), [0, 1]);
        assert!(!view.is_rearranged());

        view.set_in_database(false);
        view.refresh(&rows);
        assert_eq!(
            view.order(),
            [0],
            "here, the filter and sort apply to the loaded rows"
        );
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

    /// Cells chosen to break a comparison that isn't a total order: numbers whose order by
    /// value and as text disagree, words between them as text, and text `f64` parses.
    const AWKWARD_CELLS: [Option<&str>; 14] = [
        None,
        Some(""),
        Some("10"),
        Some("9"),
        Some("5x"),
        Some("-1"),
        Some("1e3"),
        Some("NaN"),
        Some("inf"),
        Some("abc"),
        Some("ABC"),
        Some("10a"),
        Some("0.5"),
        Some("N/A"),
    ];

    #[test]
    fn the_cell_order_is_a_total_order() {
        let cells: Vec<Cell> = AWKWARD_CELLS.iter().map(|c| c.map(str::to_owned)).collect();
        for a in &cells {
            assert_eq!(compare_cells(Some(a), Some(a)), Ordering::Equal, "{a:?}");
            for b in &cells {
                let ab = compare_cells(Some(a), Some(b));
                assert_eq!(ab, compare_cells(Some(b), Some(a)).reverse(), "{a:?} {b:?}");
                for c in &cells {
                    let bc = compare_cells(Some(b), Some(c));
                    if ab != Ordering::Greater && bc != Ordering::Greater {
                        assert_ne!(
                            compare_cells(Some(a), Some(c)),
                            Ordering::Greater,
                            "{a:?} <= {b:?} <= {c:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn sorts_a_mixed_column_numbers_first_without_panicking() {
        // A pseudo-random mix of numbers and codes, 51 rows like the query that crashed. The
        // old comparison panicked on the first shuffle of a column like this.
        let mut seed = 42_u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            seed >> 33
        };
        for _ in 0..200 {
            let rows: Vec<Row> = (0..51)
                .map(|index| {
                    let value = next() % 1000;
                    let text = if index % 3 == 0 {
                        format!("{value}x")
                    } else {
                        value.to_string()
                    };
                    vec![Some(text)]
                })
                .collect();
            let ascending = shown_cells(&rows, false);
            let numbers: Vec<f64> = ascending
                .iter()
                .map_while(|text| text.parse().ok())
                .collect();
            assert!(numbers.is_sorted(), "{ascending:?}");
            assert!(
                ascending[numbers.len()..]
                    .iter()
                    .all(|text| text.ends_with('x')),
                "numbers first: {ascending:?}"
            );

            let mut descending = shown_cells(&rows, true);
            descending.reverse();
            assert_eq!(descending, ascending);
        }
    }

    fn shown_cells(rows: &[Row], descending: bool) -> Vec<String> {
        let sort = SortKey {
            column: 0,
            descending,
        };
        display_order(rows, Some(sort), "")
            .into_iter()
            .filter_map(|index| rows[index][0].clone())
            .collect()
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
