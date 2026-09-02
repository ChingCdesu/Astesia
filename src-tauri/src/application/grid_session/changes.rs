use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{
    GridCell, GridCellEdit, GridChangeSummary, GridDelete, GridDraftRow, GridInsert, GridPage,
    GridSavePlan, GridSessionError, GridUpdate,
};
use crate::{application::QueryTarget, db::TableRef};

#[derive(Clone, Debug)]
enum GridHistoryEntry {
    Cell {
        cell: GridCell,
        previous: Option<GridCellEdit>,
    },
    DraftAdded {
        draft_id: u64,
    },
    DraftCell {
        draft_id: u64,
        column: usize,
        previous: Option<Value>,
    },
    DraftRemoved {
        index: usize,
        draft: GridDraftRow,
    },
    RowsDeleted {
        rows: Vec<usize>,
        removed_edits: Vec<(GridCell, GridCellEdit)>,
    },
    Batch(Vec<GridHistoryEntry>),
}

#[derive(Debug, Default)]
pub(super) struct GridChangeSet {
    next_draft_id: u64,
    edits: BTreeMap<GridCell, GridCellEdit>,
    drafts: Vec<GridDraftRow>,
    deleted_rows: BTreeSet<usize>,
    history: Vec<GridHistoryEntry>,
}

impl GridChangeSet {
    pub(super) fn cell_value<'a>(
        &'a self,
        page: &'a GridPage,
        cell: GridCell,
    ) -> Option<&'a Value> {
        self.edits
            .get(&cell)
            .map(|edit| &edit.new_value)
            .or_else(|| page.rows.get(cell.row)?.get(cell.column))
    }

    pub(super) fn is_cell_dirty(&self, cell: GridCell) -> bool {
        self.edits.contains_key(&cell)
    }

    pub(super) fn is_row_deleted(&self, row: usize) -> bool {
        self.deleted_rows.contains(&row)
    }

    pub(super) fn deleted_rows(&self) -> &BTreeSet<usize> {
        &self.deleted_rows
    }

    pub(super) fn stage_cell_value(
        &mut self,
        cell: GridCell,
        original: Value,
        new_value: Value,
    ) -> bool {
        let previous = self.edits.get(&cell).cloned();
        let current = previous.as_ref().map_or(&original, |edit| &edit.new_value);
        if *current == new_value {
            return false;
        }
        self.history.push(GridHistoryEntry::Cell {
            cell,
            previous: previous.clone(),
        });
        if original == new_value {
            self.edits.remove(&cell);
        } else {
            self.edits.insert(
                cell,
                GridCellEdit {
                    row: cell.row,
                    column: cell.column,
                    old_value: original,
                    new_value,
                },
            );
        }
        true
    }

    pub(super) fn stage_cell_values(&mut self, values: Vec<(GridCell, Value, Value)>) -> usize {
        let history_start = self.history.len();
        let mut changed = 0;
        for (cell, original, value) in values {
            changed += usize::from(self.stage_cell_value(cell, original, value));
        }
        let entries = self.history.split_off(history_start);
        if !entries.is_empty() {
            self.history.push(GridHistoryEntry::Batch(entries));
        }
        changed
    }

    pub(super) fn insert_draft(&mut self, column_count: usize) -> u64 {
        self.next_draft_id = self
            .next_draft_id
            .checked_add(1)
            .expect("grid draft id exhausted");
        let draft_id = self.next_draft_id;
        self.drafts.push(GridDraftRow {
            id: draft_id,
            values: vec![None; column_count],
        });
        self.history.push(GridHistoryEntry::DraftAdded { draft_id });
        draft_id
    }

    pub(super) fn set_draft_value(
        &mut self,
        draft_id: u64,
        column: usize,
        value: Value,
    ) -> Result<bool, GridSessionError> {
        let draft = self
            .drafts
            .iter_mut()
            .find(|draft| draft.id == draft_id)
            .ok_or(GridSessionError::DraftNotFound(draft_id))?;
        let current = draft
            .values
            .get_mut(column)
            .ok_or(GridSessionError::ColumnOutOfBounds(column))?;
        if current.as_ref() == Some(&value) {
            return Ok(false);
        }
        let previous = current.replace(value);
        self.history.push(GridHistoryEntry::DraftCell {
            draft_id,
            column,
            previous,
        });
        Ok(true)
    }

    pub(super) fn unset_draft_value(
        &mut self,
        draft_id: u64,
        column: usize,
    ) -> Result<bool, GridSessionError> {
        let draft = self
            .drafts
            .iter_mut()
            .find(|draft| draft.id == draft_id)
            .ok_or(GridSessionError::DraftNotFound(draft_id))?;
        let current = draft
            .values
            .get_mut(column)
            .ok_or(GridSessionError::ColumnOutOfBounds(column))?;
        let Some(previous) = current.take() else {
            return Ok(false);
        };
        self.history.push(GridHistoryEntry::DraftCell {
            draft_id,
            column,
            previous: Some(previous),
        });
        Ok(true)
    }

    pub(super) fn remove_draft(&mut self, draft_id: u64) -> Result<(), GridSessionError> {
        let Some(index) = self.drafts.iter().position(|draft| draft.id == draft_id) else {
            return Err(GridSessionError::DraftNotFound(draft_id));
        };
        let draft = self.drafts.remove(index);
        self.history
            .push(GridHistoryEntry::DraftRemoved { index, draft });
        Ok(())
    }

    pub(super) fn drafts(&self) -> &[GridDraftRow] {
        &self.drafts
    }

    pub(super) fn delete_rows(&mut self, rows: Vec<usize>) {
        let row_set = rows.iter().copied().collect::<BTreeSet<_>>();
        let removed_edits = self
            .edits
            .iter()
            .filter(|(cell, _)| row_set.contains(&cell.row))
            .map(|(cell, edit)| (*cell, edit.clone()))
            .collect::<Vec<_>>();
        self.edits.retain(|cell, _| !row_set.contains(&cell.row));
        self.deleted_rows.extend(rows.iter().copied());
        self.history.push(GridHistoryEntry::RowsDeleted {
            rows,
            removed_edits,
        });
    }

    pub(super) fn undo(&mut self) -> bool {
        let Some(entry) = self.history.pop() else {
            return false;
        };
        self.undo_entry(entry);
        true
    }

    fn undo_entry(&mut self, entry: GridHistoryEntry) {
        match entry {
            GridHistoryEntry::Cell { cell, previous } => match previous {
                Some(edit) => {
                    self.edits.insert(cell, edit);
                }
                None => {
                    self.edits.remove(&cell);
                }
            },
            GridHistoryEntry::DraftAdded { draft_id } => {
                self.drafts.retain(|draft| draft.id != draft_id);
            }
            GridHistoryEntry::DraftCell {
                draft_id,
                column,
                previous,
            } => {
                if let Some(value) = self
                    .drafts
                    .iter_mut()
                    .find(|draft| draft.id == draft_id)
                    .and_then(|draft| draft.values.get_mut(column))
                {
                    *value = previous;
                }
            }
            GridHistoryEntry::DraftRemoved { index, draft } => {
                self.drafts.insert(index.min(self.drafts.len()), draft);
            }
            GridHistoryEntry::RowsDeleted {
                rows,
                removed_edits,
            } => {
                for row in rows {
                    self.deleted_rows.remove(&row);
                }
                self.edits.extend(removed_edits);
            }
            GridHistoryEntry::Batch(entries) => {
                for entry in entries.into_iter().rev() {
                    self.undo_entry(entry);
                }
            }
        }
    }

    pub(super) fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.edits.clear();
        self.drafts.clear();
        self.deleted_rows.clear();
        self.history.clear();
    }

    pub(super) fn has_changes(&self) -> bool {
        !self.edits.is_empty() || !self.drafts.is_empty() || !self.deleted_rows.is_empty()
    }

    pub(super) fn has_history(&self) -> bool {
        !self.history.is_empty()
    }

    pub(super) fn summary(&self) -> GridChangeSummary {
        GridChangeSummary {
            updated_cells: self.edits.len(),
            inserted_rows: self.drafts.len(),
            deleted_rows: self.deleted_rows.len(),
        }
    }

    pub(super) fn primary_key_edit_count(&self, primary_key: usize) -> usize {
        self.edits
            .keys()
            .filter(|cell| cell.column == primary_key)
            .count()
    }

    pub(super) fn save_plan(
        &self,
        page: &GridPage,
        target: &QueryTarget,
        table: &TableRef,
        primary_key_index: usize,
        primary_key_column: &str,
    ) -> Result<GridSavePlan, GridSessionError> {
        let mut updates = Vec::with_capacity(self.edits.len());
        let mut ordered_edits = self.edits.values().collect::<Vec<_>>();
        ordered_edits.sort_by_key(|edit| (edit.row, edit.column == primary_key_index, edit.column));
        for edit in ordered_edits {
            let primary_key_value = page.rows[edit.row][primary_key_index].clone();
            if primary_key_value.is_null() {
                return Err(GridSessionError::MissingRowIdentity(edit.row));
            }
            updates.push(GridUpdate {
                row: edit.row,
                primary_key_value,
                column: page.columns[edit.column].name.clone(),
                new_value: edit.new_value.clone(),
            });
        }

        let inserts = self
            .drafts
            .iter()
            .map(|draft| {
                let (columns, values) = page
                    .columns
                    .iter()
                    .zip(&draft.values)
                    .filter_map(|(column, value)| {
                        value
                            .as_ref()
                            .map(|value| (column.name.clone(), value.clone()))
                    })
                    .unzip();
                GridInsert {
                    draft_id: draft.id,
                    columns,
                    values,
                }
            })
            .collect::<Vec<_>>();

        let primary_key_values = self
            .deleted_rows
            .iter()
            .map(|row| {
                let value = page.rows[*row][primary_key_index].clone();
                if value.is_null() {
                    Err(GridSessionError::MissingRowIdentity(*row))
                } else {
                    Ok(value)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let delete = (!primary_key_values.is_empty()).then_some(GridDelete { primary_key_values });
        let operation_count = updates.len() + inserts.len() + self.deleted_rows.len();
        Ok(GridSavePlan {
            target: target.clone(),
            table: table.clone(),
            primary_key_column: primary_key_column.to_string(),
            updates,
            inserts,
            delete,
            operation_count,
        })
    }
}
