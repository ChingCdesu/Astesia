use std::collections::BTreeSet;

use super::{GridCell, GridCellSelection, GridRowSelectionMode};

#[derive(Debug, Default)]
pub(super) struct GridSelection {
    rows: BTreeSet<usize>,
    row_anchor: Option<usize>,
    cells: Option<GridCellSelection>,
}

impl GridSelection {
    pub(super) fn select_row(
        &mut self,
        row: usize,
        mode: GridRowSelectionMode,
        deleted_rows: &BTreeSet<usize>,
    ) -> bool {
        let previous = self.rows.clone();
        match mode {
            GridRowSelectionMode::Replace => {
                self.rows.clear();
                self.rows.insert(row);
            }
            GridRowSelectionMode::Toggle => {
                if !self.rows.remove(&row) {
                    self.rows.insert(row);
                }
            }
            GridRowSelectionMode::Extend => {
                let anchor = self.row_anchor.unwrap_or(row);
                self.rows = (anchor.min(row)..=anchor.max(row))
                    .filter(|row| !deleted_rows.contains(row))
                    .collect();
            }
        }
        self.row_anchor = Some(row);
        self.cells = None;
        previous != self.rows
    }

    pub(super) fn select_cell(&mut self, cell: GridCell, extend: bool) -> bool {
        let selection = if extend {
            GridCellSelection {
                anchor: self.cells.map_or(cell, |selection| selection.anchor),
                focus: cell,
            }
        } else {
            GridCellSelection {
                anchor: cell,
                focus: cell,
            }
        };
        let changed = self.cells != Some(selection) || !self.rows.is_empty();
        self.cells = Some(selection);
        self.rows.clear();
        self.row_anchor = None;
        changed
    }

    pub(super) fn clear(&mut self) -> bool {
        let changed = !self.rows.is_empty() || self.row_anchor.is_some() || self.cells.is_some();
        self.rows.clear();
        self.row_anchor = None;
        self.cells = None;
        changed
    }

    pub(super) fn row_selected(&self, row: usize) -> bool {
        self.rows.contains(&row)
    }

    pub(super) fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub(super) fn has_selection(&self) -> bool {
        self.cells.is_some() || !self.rows.is_empty()
    }

    pub(super) fn cells(&self) -> Option<GridCellSelection> {
        self.cells
    }

    pub(super) fn rows(&self) -> &BTreeSet<usize> {
        &self.rows
    }
}
