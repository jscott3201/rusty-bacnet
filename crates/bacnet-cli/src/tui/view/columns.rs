//! Content-sized table columns (#1892): each column is as wide as its
//! header or its widest cell, so a wide terminal leaves the spare room at
//! the right instead of pushing the columns apart.

use ratatui::layout::Constraint;

use super::text_width;

/// The space between two columns.
const GAP: usize = 1;

/// One column: what it shows, its header and the width it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Column<K> {
    /// What the column shows.
    pub(super) kind: K,
    /// The header text.
    pub(super) header: String,
    /// Columns it takes: the header's width or the widest cell's.
    pub(super) width: usize,
}

impl<K: Copy + PartialEq> Column<K> {
    /// A column as wide as its header.
    pub(super) fn new(kind: K, header: impl Into<String>) -> Self {
        let header = header.into();
        let width = text_width(&header);
        Self {
            kind,
            header,
            width,
        }
    }

    /// Widen the column to take a cell `width` columns wide.
    pub(super) fn fit(&mut self, width: usize) {
        self.width = self.width.max(width);
    }
}

/// Columns `columns` take, with the gaps between them.
pub(super) fn total<K>(columns: &[Column<K>]) -> usize {
    let cells: usize = columns.iter().map(|column| column.width).sum();
    cells + GAP * columns.len().saturating_sub(1)
}

/// Narrow the `kind` column, no further than `min`, until the columns fit
/// in `room`.
pub(super) fn shrink<K: Copy + PartialEq>(
    columns: &mut [Column<K>],
    kind: K,
    room: usize,
    min: usize,
) {
    let over = total(columns).saturating_sub(room);
    if let Some(column) = columns.iter_mut().find(|column| column.kind == kind) {
        column.width = column.width.saturating_sub(over).max(min.min(column.width));
    }
}

/// Put `column` before index `at` when the whole of it fits in `room`.
/// Returns whether it did.
pub(super) fn insert_if_room<K>(
    columns: &mut Vec<Column<K>>,
    at: usize,
    column: Column<K>,
    room: usize,
) -> bool {
    if total(columns) + GAP + column.width > room {
        return false;
    }
    columns.insert(at.min(columns.len()), column);
    true
}

/// Put `column` before index `at`, narrowed to the room `columns` leave in
/// `room` but no narrower than `min` (or its own width, when that is less).
/// Left out when even that doesn't fit. Returns whether it went in.
pub(super) fn insert_narrowed<K>(
    columns: &mut Vec<Column<K>>,
    at: usize,
    mut column: Column<K>,
    room: usize,
    min: usize,
) -> bool {
    let left = room.saturating_sub(total(columns) + GAP);
    if left < min.min(column.width) {
        return false;
    }
    column.width = column.width.min(left);
    columns.insert(at.min(columns.len()), column);
    true
}

/// The table's width constraints.
pub(super) fn constraints<K>(columns: &[Column<K>]) -> Vec<Constraint> {
    columns
        .iter()
        .map(|column| Constraint::Length(column.width as u16))
        .collect()
}

/// The headers, for the header row.
pub(super) fn headers<K>(columns: &[Column<K>]) -> Vec<String> {
    columns.iter().map(|column| column.header.clone()).collect()
}

/// The table's column spacing.
pub(super) const SPACING: u16 = GAP as u16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_take_their_widest_cell_and_shrink_one_column_to_fit() {
        let mut columns = vec![Column::new(1, "Id"), Column::new(2, "Vendor")];
        columns[0].fit(9);
        columns[1].fit(30);
        assert_eq!(total(&columns), 9 + 1 + 30);
        shrink(&mut columns, 2, 20, 6);
        assert_eq!(columns[1].width, 10);
        shrink(&mut columns, 2, 5, 6);
        assert_eq!(columns[1].width, 6, "never below the minimum");
        assert!(!insert_if_room(&mut columns, 1, Column::new(3, "Via"), 18));
        assert!(insert_if_room(&mut columns, 1, Column::new(3, "Via"), 20));
        assert_eq!(columns[1].kind, 3);
    }

    #[test]
    fn a_narrowed_column_takes_the_room_left_down_to_its_minimum() {
        let base = || vec![Column::new(1, "Id")];
        let name = |width| {
            let mut column = Column::new(2, "Name");
            column.fit(width);
            column
        };
        // Room for all of it, then for part of it, then not enough.
        let mut columns = base();
        assert!(insert_narrowed(&mut columns, 1, name(20), 40, 8));
        assert_eq!(columns[1].width, 20);
        let mut columns = base();
        assert!(insert_narrowed(&mut columns, 1, name(20), 13, 8));
        assert_eq!(columns[1].width, 10);
        let mut columns = base();
        assert!(!insert_narrowed(&mut columns, 1, name(20), 10, 8));
        // A column narrower than the minimum goes in whole when it fits.
        let mut columns = base();
        assert!(insert_narrowed(&mut columns, 1, name(5), 8, 8));
        assert_eq!((columns.len(), columns[1].width), (2, 5));
    }
}
