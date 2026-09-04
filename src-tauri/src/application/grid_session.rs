use std::{
    collections::{BTreeMap, HashSet},
    error::Error,
    fmt,
};

use serde_json::Value;

use crate::db::{ColumnInfo, DbType, RowMutationMode, TableRef};

use super::{GridColumn, QueryTarget};

mod changes;
mod formatting;
mod lifecycle;
mod selection;

use changes::GridChangeSet;
use formatting::{format_grid_tsv, format_grid_value};
pub(crate) use lifecycle::GridSessionStatus;
use lifecycle::GridState;
use selection::GridSelection;

pub(crate) const DEFAULT_GRID_PAGE_SIZE: u32 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GridSortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GridSort {
    pub(crate) column: String,
    pub(crate) direction: GridSortDirection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GridQuery {
    pub(crate) page: u32,
    pub(crate) page_size: u32,
    pub(crate) filter: Option<String>,
    pub(crate) sort: Vec<GridSort>,
}

#[derive(Clone, Debug)]
pub(crate) struct GridPage {
    pub(crate) columns: Vec<GridColumn>,
    pub(crate) rows: Vec<Vec<Value>>,
    pub(crate) total_rows: Option<u64>,
}

impl GridPage {
    pub(crate) fn new(
        columns: Vec<ColumnInfo>,
        rows: Vec<Vec<Value>>,
        total_rows: Option<u64>,
    ) -> Result<Self, GridSessionError> {
        let expected = columns.len();
        if let Some((row, actual)) = rows
            .iter()
            .enumerate()
            .find_map(|(row, values)| (values.len() != expected).then_some((row, values.len())))
        {
            return Err(GridSessionError::InvalidPageShape {
                row,
                expected,
                actual,
            });
        }
        Ok(Self {
            columns: columns
                .into_iter()
                .map(|column| GridColumn::new(column, Vec::new()))
                .collect(),
            rows,
            total_rows,
        })
    }

    pub(crate) fn with_enum_values(mut self, values: BTreeMap<usize, Vec<String>>) -> Self {
        for (index, values) in values {
            if let Some(column) = self.columns.get_mut(index) {
                column.set_enum_values(values);
            }
        }
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GridLoadRequest {
    generation: u64,
    target: QueryTarget,
    table: TableRef,
    query: GridQuery,
}

impl GridLoadRequest {
    pub(crate) fn for_chart_page(
        target: QueryTarget,
        table: TableRef,
        query: GridQuery,
        page: u32,
        page_size: u32,
    ) -> Self {
        let mut query = query;
        query.page = page;
        query.page_size = page_size;
        Self {
            generation: u64::from(page),
            target,
            table,
            query,
        }
    }

    pub(crate) fn target(&self) -> &QueryTarget {
        &self.target
    }

    pub(crate) fn table(&self) -> &TableRef {
        &self.table
    }

    pub(crate) fn query(&self) -> &GridQuery {
        &self.query
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridSaveRequest {
    generation: u64,
    plan: GridSavePlan,
}

impl GridSaveRequest {
    pub(crate) fn plan(&self) -> &GridSavePlan {
        &self.plan
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GridSaveFailure {
    pub(crate) completed_statements: usize,
    pub(crate) total_statements: usize,
    pub(crate) message: String,
}

impl GridSaveFailure {
    pub(crate) fn before_execution(total_statements: usize, message: impl Into<String>) -> Self {
        Self {
            completed_statements: 0,
            total_statements,
            message: message.into(),
        }
    }

    pub(super) fn during_execution(
        completed_statements: usize,
        total_statements: usize,
        message: impl Into<String>,
    ) -> Self {
        Self {
            completed_statements,
            total_statements,
            message: message.into(),
        }
    }
}

impl fmt::Display for GridSaveFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.completed_statements == 0 {
            formatter.write_str(&self.message)
        } else {
            write!(
                formatter,
                "{} (statement {} of {} failed; transaction rolled back)",
                self.message,
                self.completed_statements + 1,
                self.total_statements
            )
        }
    }
}

impl Error for GridSaveFailure {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GridEditability<'a> {
    AwaitingData,
    Editable {
        primary_key_index: usize,
        primary_key_column: &'a str,
    },
    ReadOnlyEngine(DbType),
    MissingPrimaryKey,
    CompositePrimaryKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GridRowSelectionMode {
    Replace,
    Toggle,
    Extend,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct GridCell {
    pub(crate) row: usize,
    pub(crate) column: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GridCellSelection {
    pub(crate) anchor: GridCell,
    pub(crate) focus: GridCell,
}

impl GridCellSelection {
    pub(crate) fn contains(self, cell: GridCell) -> bool {
        let min_row = self.anchor.row.min(self.focus.row);
        let max_row = self.anchor.row.max(self.focus.row);
        let min_column = self.anchor.column.min(self.focus.column);
        let max_column = self.anchor.column.max(self.focus.column);
        (min_row..=max_row).contains(&cell.row) && (min_column..=max_column).contains(&cell.column)
    }

    pub(crate) fn top_left(self) -> GridCell {
        GridCell {
            row: self.anchor.row.min(self.focus.row),
            column: self.anchor.column.min(self.focus.column),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridCellEdit {
    pub(crate) row: usize,
    pub(crate) column: usize,
    pub(crate) old_value: Value,
    pub(crate) new_value: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridDraftRow {
    pub(crate) id: u64,
    pub(crate) values: Vec<Option<Value>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct GridChangeSummary {
    pub(crate) updated_cells: usize,
    pub(crate) inserted_rows: usize,
    pub(crate) deleted_rows: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridUpdate {
    pub(crate) row: usize,
    pub(crate) primary_key_value: Value,
    pub(crate) column: String,
    pub(crate) new_value: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridInsert {
    pub(crate) draft_id: u64,
    pub(crate) columns: Vec<String>,
    pub(crate) values: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridDelete {
    pub(crate) primary_key_values: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridSavePlan {
    pub(crate) target: QueryTarget,
    pub(crate) table: TableRef,
    pub(crate) primary_key_column: String,
    pub(crate) updates: Vec<GridUpdate>,
    pub(crate) inserts: Vec<GridInsert>,
    pub(crate) delete: Option<GridDelete>,
    pub(crate) operation_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GridSessionError {
    InvalidPage,
    InvalidPageSize,
    InvalidPageShape {
        row: usize,
        expected: usize,
        actual: usize,
    },
    Loading,
    Saving,
    Unavailable,
    PendingChanges,
    NoChanges,
    AwaitingData,
    ReadOnlyEngine(DbType),
    MissingPrimaryKey,
    CompositePrimaryKey,
    RowOutOfBounds(usize),
    ColumnOutOfBounds(usize),
    DeletedRow(usize),
    UnknownSortColumn(String),
    DuplicateSortColumn(String),
    DraftNotFound(u64),
    MissingRowIdentity(usize),
}

#[derive(Debug)]
pub(crate) struct GridSession {
    target: QueryTarget,
    table: TableRef,
    query: GridQuery,
    next_load_generation: u64,
    next_save_generation: u64,
    state: GridState,
    selection: GridSelection,
    changes: GridChangeSet,
}

struct GridProjection {
    columns: Vec<usize>,
    rows: Vec<usize>,
}

impl GridSession {
    pub(crate) fn new(
        target: QueryTarget,
        table: TableRef,
        page_size: u32,
    ) -> Result<Self, GridSessionError> {
        if page_size == 0 {
            return Err(GridSessionError::InvalidPageSize);
        }
        Ok(Self {
            target,
            table,
            query: GridQuery {
                page: 1,
                page_size,
                filter: None,
                sort: Vec::new(),
            },
            next_load_generation: 0,
            next_save_generation: 0,
            state: GridState::Idle,
            selection: GridSelection::default(),
            changes: GridChangeSet::default(),
        })
    }

    pub(crate) fn target(&self) -> &QueryTarget {
        &self.target
    }

    pub(crate) fn table(&self) -> &TableRef {
        &self.table
    }

    pub(crate) fn query(&self) -> &GridQuery {
        &self.query
    }

    pub(crate) fn page(&self) -> Option<&GridPage> {
        match &self.state {
            GridState::Idle => None,
            GridState::Loading { page, .. }
            | GridState::Failed { page, .. }
            | GridState::Unavailable { page, .. } => page.as_ref(),
            GridState::Saving { page, .. }
            | GridState::Ready(page)
            | GridState::SaveFailed { page, .. } => Some(page),
        }
    }

    pub(crate) fn status(&self) -> GridSessionStatus<'_> {
        match &self.state {
            GridState::Idle => GridSessionStatus::Idle,
            GridState::Loading { .. } => GridSessionStatus::Loading,
            GridState::Saving { .. } => GridSessionStatus::Saving,
            GridState::Ready(_) => GridSessionStatus::Ready,
            GridState::Failed { error, .. } => GridSessionStatus::Failed { error },
            GridState::SaveFailed { error, .. } => GridSessionStatus::SaveFailed { error },
            GridState::Unavailable { reason, .. } => GridSessionStatus::Unavailable { reason },
        }
    }

    pub(crate) fn begin_load(&mut self) -> Result<GridLoadRequest, GridSessionError> {
        self.require_idle_clean_session()?;
        self.next_load_generation = self
            .next_load_generation
            .checked_add(1)
            .expect("grid load generation exhausted");
        let request = GridLoadRequest {
            generation: self.next_load_generation,
            target: self.target.clone(),
            table: self.table.clone(),
            query: self.query.clone(),
        };
        let page = self.take_page();
        self.state = GridState::Loading {
            generation: request.generation,
            page,
        };
        Ok(request)
    }

    pub(crate) fn finish_load(
        &mut self,
        request: &GridLoadRequest,
        result: Result<GridPage, String>,
    ) -> bool {
        let state = std::mem::replace(&mut self.state, GridState::Idle);
        let GridState::Loading { generation, page } = state else {
            self.state = state;
            return false;
        };
        if generation != request.generation || request.query != self.query {
            self.state = GridState::Loading { generation, page };
            return false;
        }
        self.state = match result {
            Ok(page) => {
                self.clear_selection();
                GridState::Ready(page)
            }
            Err(error) => GridState::Failed { error, page },
        };
        true
    }

    pub(crate) fn begin_save(&mut self) -> Result<GridSaveRequest, GridSessionError> {
        if !self.has_changes() {
            return Err(GridSessionError::NoChanges);
        }
        let plan = self.save_plan()?;
        self.next_save_generation = self
            .next_save_generation
            .checked_add(1)
            .expect("grid save generation exhausted");
        let request = GridSaveRequest {
            generation: self.next_save_generation,
            plan,
        };
        let page = self
            .take_page()
            .expect("an editable save plan requires a loaded page");
        self.state = GridState::Saving {
            generation: request.generation,
            page,
        };
        Ok(request)
    }

    pub(crate) fn finish_save(
        &mut self,
        request: &GridSaveRequest,
        result: Result<(), GridSaveFailure>,
    ) -> bool {
        let state = std::mem::replace(&mut self.state, GridState::Idle);
        let GridState::Saving { generation, page } = state else {
            self.state = state;
            return false;
        };
        if generation != request.generation {
            self.state = GridState::Saving { generation, page };
            return false;
        }
        match result {
            Ok(()) => {
                self.clear_changes();
                self.clear_selection();
            }
            Err(error) => {
                self.state = GridState::SaveFailed {
                    error: error.to_string(),
                    page,
                };
            }
        }
        true
    }

    pub(crate) fn invalidate_session(
        &mut self,
        connection_id: &str,
        session_generation: u64,
        reason: impl Into<String>,
    ) -> bool {
        if self.target.connection_id != connection_id
            || self.target.session_generation != session_generation
            || matches!(self.state, GridState::Unavailable { .. })
        {
            return false;
        }
        let page = self.take_page();
        self.state = GridState::Unavailable {
            reason: reason.into(),
            page,
        };
        true
    }

    pub(crate) fn set_page(&mut self, page: u32) -> Result<bool, GridSessionError> {
        if page == 0 {
            return Err(GridSessionError::InvalidPage);
        }
        self.require_idle_clean_session()?;
        if self.query.page == page {
            return Ok(false);
        }
        self.query.page = page;
        self.invalidate_page();
        Ok(true)
    }

    pub(crate) fn set_query_options(
        &mut self,
        filter: Option<String>,
        sort: Vec<GridSort>,
    ) -> Result<bool, GridSessionError> {
        self.require_idle_clean_session()?;
        let filter = filter
            .map(|filter| filter.trim().to_string())
            .filter(|filter| !filter.is_empty());
        let page = self.page().ok_or(GridSessionError::AwaitingData)?;
        let columns = page
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        for item in &sort {
            if !columns.contains(item.column.as_str()) {
                return Err(GridSessionError::UnknownSortColumn(item.column.clone()));
            }
            if !seen.insert(item.column.as_str()) {
                return Err(GridSessionError::DuplicateSortColumn(item.column.clone()));
            }
        }
        if self.query.filter == filter && self.query.sort == sort {
            return Ok(false);
        }
        self.query.filter = filter;
        self.query.sort = sort;
        self.query.page = 1;
        self.invalidate_page();
        Ok(true)
    }

    pub(crate) fn set_filter(&mut self, filter: Option<String>) -> Result<bool, GridSessionError> {
        self.require_idle_clean_session()?;
        let filter = filter
            .map(|filter| filter.trim().to_string())
            .filter(|filter| !filter.is_empty());
        if self.query.filter == filter {
            return Ok(false);
        }
        self.query.filter = filter;
        self.query.page = 1;
        self.invalidate_page();
        Ok(true)
    }

    pub(crate) fn editability(&self) -> GridEditability<'_> {
        let capabilities = self.target.db_type.capabilities();
        if capabilities.data_browser_read_only
            || capabilities.row_mutation != RowMutationMode::StructuredSql
        {
            return GridEditability::ReadOnlyEngine(self.target.db_type);
        }
        let Some(page) = self.page() else {
            return GridEditability::AwaitingData;
        };
        let keys = page
            .columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.is_primary_key)
            .collect::<Vec<_>>();
        match keys.as_slice() {
            [] => GridEditability::MissingPrimaryKey,
            [(index, column)] => GridEditability::Editable {
                primary_key_index: *index,
                primary_key_column: &column.name,
            },
            _ => GridEditability::CompositePrimaryKey,
        }
    }

    pub(crate) fn select_row(
        &mut self,
        row: usize,
        mode: GridRowSelectionMode,
    ) -> Result<bool, GridSessionError> {
        self.require_row(row)?;
        if self.changes.is_row_deleted(row) {
            return Err(GridSessionError::DeletedRow(row));
        }
        Ok(self
            .selection
            .select_row(row, mode, self.changes.deleted_rows()))
    }

    pub(crate) fn row_selected(&self, row: usize) -> bool {
        self.selection.row_selected(row)
    }

    pub(crate) fn selected_row_count(&self) -> usize {
        self.selection.row_count()
    }

    pub(crate) fn has_selection(&self) -> bool {
        self.selection.has_selection()
    }

    pub(crate) fn selection_tsv(&self, include_headers: bool) -> Option<String> {
        let page = self.page()?;
        let projection = self.selected_projection(page)?;
        let (columns, values) = self.project(page, projection, format_grid_value)?;
        let mut rows = Vec::with_capacity(values.len() + usize::from(include_headers));
        if include_headers {
            rows.push(columns);
        }
        rows.extend(values);
        Some(format_grid_tsv(rows))
    }

    pub(crate) fn export_rows(&self) -> Option<(Vec<String>, Vec<Vec<Value>>)> {
        let page = self.page()?;
        let projection = self
            .selected_projection(page)
            .unwrap_or_else(|| self.visible_projection(page));
        self.project(page, projection, Value::clone)
    }

    fn selected_projection(&self, page: &GridPage) -> Option<GridProjection> {
        if let Some(selection) = self.selection.cells() {
            let row_start = selection.anchor.row.min(selection.focus.row);
            let row_end = selection.anchor.row.max(selection.focus.row);
            let column_start = selection.anchor.column.min(selection.focus.column);
            let column_end = selection.anchor.column.max(selection.focus.column);
            return Some(GridProjection {
                columns: (column_start..=column_end).collect(),
                rows: (row_start..=row_end).collect(),
            });
        }

        (!self.selection.rows().is_empty()).then(|| GridProjection {
            columns: (0..page.columns.len()).collect(),
            rows: self.selection.rows().iter().copied().collect(),
        })
    }

    fn visible_projection(&self, page: &GridPage) -> GridProjection {
        GridProjection {
            columns: (0..page.columns.len()).collect(),
            rows: (0..page.rows.len())
                .filter(|row| !self.changes.is_row_deleted(*row))
                .collect(),
        }
    }

    fn project<T>(
        &self,
        page: &GridPage,
        projection: GridProjection,
        project_value: impl Fn(&Value) -> T,
    ) -> Option<(Vec<String>, Vec<Vec<T>>)> {
        let columns = projection
            .columns
            .iter()
            .map(|column| page.columns.get(*column).map(|column| column.name.clone()))
            .collect::<Option<Vec<_>>>()?;
        let rows = projection
            .rows
            .into_iter()
            .map(|row| {
                projection
                    .columns
                    .iter()
                    .map(|column| {
                        self.cell_value(GridCell {
                            row,
                            column: *column,
                        })
                        .map(&project_value)
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .collect::<Option<Vec<_>>>()?;
        Some((columns, rows))
    }

    pub(crate) fn select_cell(
        &mut self,
        cell: GridCell,
        extend: bool,
    ) -> Result<bool, GridSessionError> {
        self.require_cell(cell)?;
        Ok(self.selection.select_cell(cell, extend))
    }

    pub(crate) fn cell_selection(&self) -> Option<GridCellSelection> {
        self.selection.cells()
    }

    pub(crate) fn clear_selection(&mut self) -> bool {
        self.selection.clear()
    }

    pub(crate) fn cell_value(&self, cell: GridCell) -> Option<&Value> {
        self.changes.cell_value(self.page()?, cell)
    }

    pub(crate) fn is_cell_dirty(&self, cell: GridCell) -> bool {
        self.changes.is_cell_dirty(cell)
    }

    pub(crate) fn is_row_deleted(&self, row: usize) -> bool {
        self.changes.is_row_deleted(row)
    }

    pub(crate) fn stage_cell_value(
        &mut self,
        cell: GridCell,
        new_value: Value,
    ) -> Result<bool, GridSessionError> {
        self.require_editable()?;
        self.require_cell(cell)?;
        if self.changes.is_row_deleted(cell.row) {
            return Err(GridSessionError::DeletedRow(cell.row));
        }
        let original = self
            .page()
            .and_then(|page| page.rows.get(cell.row))
            .and_then(|row| row.get(cell.column))
            .cloned()
            .ok_or(GridSessionError::RowOutOfBounds(cell.row))?;
        Ok(self.changes.stage_cell_value(cell, original, new_value))
    }

    pub(crate) fn stage_cell_values(
        &mut self,
        values: Vec<(GridCell, Value)>,
    ) -> Result<usize, GridSessionError> {
        self.require_editable()?;
        let mut prepared = Vec::with_capacity(values.len());
        for (cell, value) in values {
            self.require_cell(cell)?;
            if self.changes.is_row_deleted(cell.row) {
                return Err(GridSessionError::DeletedRow(cell.row));
            }
            let original = self
                .page()
                .and_then(|page| page.rows.get(cell.row))
                .and_then(|row| row.get(cell.column))
                .cloned()
                .ok_or(GridSessionError::RowOutOfBounds(cell.row))?;
            prepared.push((cell, original, value));
        }
        Ok(self.changes.stage_cell_values(prepared))
    }

    pub(crate) fn stage_insert(&mut self) -> Result<u64, GridSessionError> {
        self.require_editable()?;
        let column_count = self
            .page()
            .ok_or(GridSessionError::AwaitingData)?
            .columns
            .len();
        Ok(self.changes.insert_draft(column_count))
    }

    pub(crate) fn set_draft_value(
        &mut self,
        draft_id: u64,
        column: usize,
        value: Value,
    ) -> Result<bool, GridSessionError> {
        self.require_editable()?;
        self.changes.set_draft_value(draft_id, column, value)
    }

    pub(crate) fn unset_draft_value(
        &mut self,
        draft_id: u64,
        column: usize,
    ) -> Result<bool, GridSessionError> {
        self.require_editable()?;
        self.changes.unset_draft_value(draft_id, column)
    }

    pub(crate) fn remove_draft(&mut self, draft_id: u64) -> Result<bool, GridSessionError> {
        self.require_editable()?;
        self.changes.remove_draft(draft_id)?;
        Ok(true)
    }

    pub(crate) fn drafts(&self) -> &[GridDraftRow] {
        self.changes.drafts()
    }

    pub(crate) fn stage_delete_selection(&mut self) -> Result<bool, GridSessionError> {
        self.require_editable()?;
        let rows = self
            .selection
            .rows()
            .iter()
            .copied()
            .filter(|row| !self.changes.is_row_deleted(*row))
            .collect::<Vec<_>>();
        if rows.is_empty() {
            return Ok(false);
        }
        self.changes.delete_rows(rows);
        self.selection.clear();
        Ok(true)
    }

    pub(crate) fn undo(&mut self) -> bool {
        !matches!(self.state, GridState::Saving { .. }) && self.changes.undo()
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.changes.can_undo() && !matches!(self.state, GridState::Saving { .. })
    }

    pub(crate) fn discard_changes(&mut self) -> bool {
        if matches!(self.state, GridState::Saving { .. }) {
            return false;
        }
        let changed = self.changes.has_changes() || self.changes.has_history();
        self.changes.clear();
        let state = std::mem::replace(&mut self.state, GridState::Idle);
        self.state = match state {
            GridState::SaveFailed { page, .. } => GridState::Ready(page),
            state => state,
        };
        self.selection.clear();
        changed
    }

    fn clear_changes(&mut self) {
        self.changes.clear();
    }

    pub(crate) fn has_changes(&self) -> bool {
        self.changes.has_changes()
    }

    pub(crate) fn change_summary(&self) -> GridChangeSummary {
        self.changes.summary()
    }

    pub(crate) fn primary_key_edit_count(&self) -> usize {
        let Some(page) = self.page() else {
            return 0;
        };
        let Some(primary_key) = page.columns.iter().position(|column| column.is_primary_key) else {
            return 0;
        };
        self.changes.primary_key_edit_count(primary_key)
    }

    pub(crate) fn save_plan(&self) -> Result<GridSavePlan, GridSessionError> {
        let (primary_key_index, primary_key_column) = self.require_editable()?;
        let page = self.page().ok_or(GridSessionError::AwaitingData)?;
        self.changes.save_plan(
            page,
            &self.target,
            &self.table,
            primary_key_index,
            primary_key_column,
        )
    }

    fn require_idle_clean_session(&self) -> Result<(), GridSessionError> {
        self.require_available()?;
        self.require_not_busy()?;
        if self.has_changes() {
            return Err(GridSessionError::PendingChanges);
        }
        Ok(())
    }

    fn invalidate_page(&mut self) {
        self.state = GridState::Idle;
        self.clear_selection();
    }

    fn take_page(&mut self) -> Option<GridPage> {
        match std::mem::replace(&mut self.state, GridState::Idle) {
            GridState::Idle => None,
            GridState::Loading { page, .. }
            | GridState::Failed { page, .. }
            | GridState::Unavailable { page, .. } => page,
            GridState::Saving { page, .. }
            | GridState::Ready(page)
            | GridState::SaveFailed { page, .. } => Some(page),
        }
    }

    fn require_available(&self) -> Result<(), GridSessionError> {
        if matches!(self.state, GridState::Unavailable { .. }) {
            Err(GridSessionError::Unavailable)
        } else {
            Ok(())
        }
    }

    fn require_editable(&self) -> Result<(usize, &str), GridSessionError> {
        self.require_available()?;
        self.require_not_busy()?;
        match self.editability() {
            GridEditability::AwaitingData => Err(GridSessionError::AwaitingData),
            GridEditability::Editable {
                primary_key_index,
                primary_key_column,
            } => Ok((primary_key_index, primary_key_column)),
            GridEditability::ReadOnlyEngine(db_type) => {
                Err(GridSessionError::ReadOnlyEngine(db_type))
            }
            GridEditability::MissingPrimaryKey => Err(GridSessionError::MissingPrimaryKey),
            GridEditability::CompositePrimaryKey => Err(GridSessionError::CompositePrimaryKey),
        }
    }

    fn require_not_busy(&self) -> Result<(), GridSessionError> {
        match self.state {
            GridState::Loading { .. } => Err(GridSessionError::Loading),
            GridState::Saving { .. } => Err(GridSessionError::Saving),
            _ => Ok(()),
        }
    }

    fn require_row(&self, row: usize) -> Result<(), GridSessionError> {
        let row_count = self
            .page()
            .ok_or(GridSessionError::AwaitingData)?
            .rows
            .len();
        if row < row_count {
            Ok(())
        } else {
            Err(GridSessionError::RowOutOfBounds(row))
        }
    }

    fn require_cell(&self, cell: GridCell) -> Result<(), GridSessionError> {
        self.require_row(cell.row)?;
        let column_count = self
            .page()
            .ok_or(GridSessionError::AwaitingData)?
            .columns
            .len();
        if cell.column < column_count {
            Ok(())
        } else {
            Err(GridSessionError::ColumnOutOfBounds(cell.column))
        }
    }
}

#[cfg(test)]
mod tests;
