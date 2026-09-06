use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::ui::components::{prelude::*, Indicator, Tooltip};
use crate::ui::text_editor::{Editor, EditorEvent};
use gpui_kit::component::table::{TableEvent, TableState};
use gpui_kit::{
    actions, point, App, ClickEvent, ClipboardItem, Entity, FocusHandle, FontWeight, PromptButton,
    PromptLevel, ScrollHandle, ScrollStrategy, Subscription,
};
use serde_json::Value;
#[cfg(test)]
use std::cell::RefCell;

use crate::application::{
    Application, ChartModel, GridCell, GridCellInputError, GridCellSelection, GridColumn,
    GridColumnKind, GridEditability, GridPage, GridRowSelectionMode, GridSaveFailure,
    GridSaveOutcome, GridSession, GridSessionError, GridSessionStatus, GridSort, GridSortDirection,
    QueryTarget, DEFAULT_GRID_PAGE_SIZE,
};
#[cfg(test)]
use crate::db::ColumnInfo;
use crate::db::TableRef;
use crate::platform::UiLanguage;

use super::chart_view::ChartView;
use super::localization::text;
use super::query_item::display_value;
use super::shell::ShellSettings;

const ROW_NUMBER_WIDTH: f32 = 48.0;
const COLUMN_WIDTH: f32 = 180.0;
const MIN_COLUMN_WIDTH: f32 = 96.0;
const MAX_COLUMN_WIDTH: f32 = 560.0;

actions!(
    astesia_data_grid,
    [
        MoveGridUp,
        MoveGridDown,
        MoveGridLeft,
        MoveGridRight,
        ExtendGridUp,
        ExtendGridDown,
        ExtendGridLeft,
        ExtendGridRight,
        BeginActiveGridCellEdit,
        CommitGridCellEdit,
        CancelGridCellEdit,
        SelectActiveGridCell,
        SelectActiveGridRow,
        ClearGridSelection,
        SaveGridChanges,
        UndoGridChanges,
        DiscardGridChanges,
        ApplyGridFilter,
        CopyGridSelection,
        PasteGridSelection
    ]
);

pub(super) fn bind_data_grid_item_keys(cx: &mut App) {
    cx.bind_keys([
        gpui_kit::KeyBinding::new("enter", menu::Confirm, Some("DataGridColumnHeader")),
        gpui_kit::KeyBinding::new("space", menu::Confirm, Some("DataGridColumnHeader")),
        gpui_kit::KeyBinding::new("up", MoveGridUp, Some("DataGrid")),
        gpui_kit::KeyBinding::new("down", MoveGridDown, Some("DataGrid")),
        gpui_kit::KeyBinding::new("left", MoveGridLeft, Some("DataGrid")),
        gpui_kit::KeyBinding::new("right", MoveGridRight, Some("DataGrid")),
        gpui_kit::KeyBinding::new("shift-up", ExtendGridUp, Some("DataGrid")),
        gpui_kit::KeyBinding::new("shift-down", ExtendGridDown, Some("DataGrid")),
        gpui_kit::KeyBinding::new("shift-left", ExtendGridLeft, Some("DataGrid")),
        gpui_kit::KeyBinding::new("shift-right", ExtendGridRight, Some("DataGrid")),
        gpui_kit::KeyBinding::new("enter", BeginActiveGridCellEdit, Some("DataGrid")),
        gpui_kit::KeyBinding::new("space", SelectActiveGridCell, Some("DataGrid")),
        gpui_kit::KeyBinding::new("shift-space", SelectActiveGridRow, Some("DataGrid")),
        gpui_kit::KeyBinding::new("escape", ClearGridSelection, Some("DataGrid")),
        gpui_kit::KeyBinding::new(
            "enter",
            CommitGridCellEdit,
            Some("DataGridCellEditor > Input"),
        ),
        gpui_kit::KeyBinding::new(
            "escape",
            CancelGridCellEdit,
            Some("DataGridCellEditor > Input"),
        ),
        gpui_kit::KeyBinding::new(
            "cmd-enter",
            CommitGridCellEdit,
            Some("DataGridLongEditor > Input"),
        ),
        gpui_kit::KeyBinding::new(
            "ctrl-enter",
            CommitGridCellEdit,
            Some("DataGridLongEditor > Input"),
        ),
        gpui_kit::KeyBinding::new(
            "escape",
            CancelGridCellEdit,
            Some("DataGridLongEditor > Input"),
        ),
        gpui_kit::KeyBinding::new("cmd-s", SaveGridChanges, Some("DataGridItem")),
        gpui_kit::KeyBinding::new("ctrl-s", SaveGridChanges, Some("DataGridItem")),
        gpui_kit::KeyBinding::new("cmd-z", UndoGridChanges, Some("DataGrid")),
        gpui_kit::KeyBinding::new("ctrl-z", UndoGridChanges, Some("DataGrid")),
        gpui_kit::KeyBinding::new("cmd-c", CopyGridSelection, Some("DataGrid")),
        gpui_kit::KeyBinding::new("ctrl-c", CopyGridSelection, Some("DataGrid")),
        gpui_kit::KeyBinding::new("cmd-v", PasteGridSelection, Some("DataGrid")),
        gpui_kit::KeyBinding::new("ctrl-v", PasteGridSelection, Some("DataGrid")),
        gpui_kit::KeyBinding::new("enter", ApplyGridFilter, Some("DataGridFilter > Input")),
    ]);
}

struct ActiveCellEditor {
    target: CellEditorTarget,
    column: GridColumn,
    editor: Entity<Editor>,
    initial_value: Option<Value>,
    initial_text: String,
    null_requested: bool,
    modified: bool,
    expanded: bool,
    error: Option<String>,
    _observation: Subscription,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellEditorTarget {
    Existing(GridCell),
    Draft { draft_id: u64, column: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum GridPasteError {
    NoTarget,
    Empty,
    InvalidDelimited(String),
    UnevenRows,
    OutOfBounds,
    InvalidCell {
        row: usize,
        column: usize,
        error: GridCellInputError,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum GridNotice {
    Success(String),
    Warning(String),
    Error(String),
}

impl GridNotice {
    fn presentation(self) -> (Color, IconName, String) {
        match self {
            Self::Success(message) => (Color::Success, IconName::Check, message),
            Self::Warning(message) => (Color::Warning, IconName::TriangleAlert, message),
            Self::Error(message) => (Color::Error, IconName::TriangleAlert, message),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GridExportFormat {
    Csv,
    Json,
    Xlsx,
}

#[derive(Default)]
struct ChartLoadCancellation {
    cancelled: Arc<AtomicBool>,
}

impl Drop for ChartLoadCancellation {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

pub(super) struct DataGridItem {
    application: Arc<Application>,
    state: GridSession,
    focus_handle: FocusHandle,
    active_cell: Option<GridCell>,
    context_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    editing: Option<ActiveCellEditor>,
    operation_notice: Option<GridNotice>,
    export_in_progress: bool,
    transaction: Option<crate::application::GridTransaction>,
    manual_transaction: bool,
    transaction_busy: bool,
    save_recovery_sql: Option<String>,
    transaction_isolation: crate::db::TransactionIsolation,
    filter_editor: Entity<Editor>,
    sort_editor: Entity<Editor>,
    column_widths: Vec<f32>,
    rows_scroll_handle: gpui_kit::UniformListScrollHandle,
    data_table: Entity<TableState<table::GridTableDelegate>>,
    _table_subscription: Subscription,
    #[cfg(test)]
    rendered_rows: RefCell<Vec<usize>>,
    horizontal_scroll_handle: ScrollHandle,
    chart: Option<Entity<ChartView>>,
    showing_chart: bool,
    chart_generation: u64,
    chart_load: Option<ChartLoadCancellation>,
    chart_loading: bool,
    chart_error: Option<String>,
    settings: Entity<ShellSettings>,
    _filter_observation: Subscription,
    _sort_observation: Subscription,
    _settings_observation: Subscription,
}

impl DataGridItem {
    pub(super) fn new(
        application: Arc<Application>,
        target: QueryTarget,
        table: TableRef,
        settings: Entity<ShellSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut item = Self::new_unloaded(application, target, table, settings, window, cx);
        item.load(cx);
        item
    }

    fn new_unloaded(
        application: Arc<Application>,
        target: QueryTarget,
        table: TableRef,
        settings: Entity<ShellSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let filter_editor = cx.new(|cx| {
            let mut editor = Editor::inline_single_line("WHERE", px(11.0), window, cx);
            editor.set_placeholder_text("status = 'active'", window, cx);
            editor
        });
        let filter_observation = cx.subscribe(&filter_editor, |_, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::Change) {
                cx.notify();
            }
        });
        let sort_editor = cx.new(|cx| {
            let mut editor = Editor::inline_single_line("ORDER BY", px(11.0), window, cx);
            editor.set_placeholder_text("id ASC", window, cx);
            editor
        });
        let sort_observation = cx.subscribe(&sort_editor, |_, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::Change) {
                cx.notify();
            }
        });
        let owner = cx.entity().downgrade();
        let data_table = cx.new(|cx| {
            TableState::new(table::GridTableDelegate::new(owner), window, cx)
                .row_selectable(false)
                .col_selectable(false)
                .cell_selectable(false)
                .row_header(false)
                .col_movable(false)
        });
        let rows_scroll_handle = data_table.read(cx).vertical_scroll_handle.clone();
        let horizontal_scroll_handle = data_table
            .read(cx)
            .horizontal_scroll_handle
            .base_handle()
            .clone();
        let table_subscription = cx.subscribe(&data_table, |item, _, event: &TableEvent, cx| {
            if let TableEvent::ColumnWidthsChanged(widths) = event {
                item.column_widths = widths
                    .iter()
                    .skip(1)
                    .map(|width| f32::from(*width))
                    .collect();
                cx.notify();
            }
        });
        Self {
            application,
            state: GridSession::new(target, table, DEFAULT_GRID_PAGE_SIZE)
                .expect("default grid page size must be valid"),
            focus_handle: cx.focus_handle(),
            active_cell: None,
            context_menu: None,
            editing: None,
            operation_notice: None,
            export_in_progress: false,
            transaction: None,
            manual_transaction: false,
            transaction_busy: false,
            save_recovery_sql: None,
            transaction_isolation: crate::db::TransactionIsolation::DatabaseDefault,
            filter_editor,
            sort_editor,
            column_widths: Vec::new(),
            rows_scroll_handle,
            data_table,
            _table_subscription: table_subscription,
            #[cfg(test)]
            rendered_rows: RefCell::new(Vec::new()),
            horizontal_scroll_handle,
            chart: None,
            showing_chart: false,
            chart_generation: 0,
            chart_load: None,
            chart_loading: false,
            chart_error: None,
            settings,
            _filter_observation: filter_observation,
            _sort_observation: sort_observation,
            _settings_observation: settings_observation,
        }
    }

    pub(super) fn label(&self) -> String {
        self.state.table().name().to_string()
    }

    pub(super) fn has_unsaved_changes(&self) -> bool {
        self.save_recovery_sql.is_some()
            || self
                .transaction
                .as_ref()
                .is_some_and(|transaction| transaction.has_pending_changes())
            || self.has_local_changes()
    }

    pub(super) fn has_local_changes(&self) -> bool {
        self.state.has_changes()
            || self
                .editing
                .as_ref()
                .is_some_and(|editing| editing.modified)
    }

    pub(super) fn table_name(&self) -> String {
        self.state.table().to_string()
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editing) = &self.editing {
            window.focus(&editing.editor.read(cx).focus_handle(cx), cx);
        } else {
            window.focus(&self.focus_handle, cx);
        }
    }

    fn sync_chart(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.state.page() else {
            return;
        };
        let columns = page
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let rows = page.rows.clone();
        if let Some(chart) = &self.chart {
            chart.update(cx, |chart, cx| chart.replace_data(columns, rows, cx));
        } else {
            let model = ChartModel::from_names(columns, rows);
            self.chart = Some(cx.new(|cx| ChartView::new(model, self.settings.clone(), cx)));
        }
    }
}

mod context_menu;
mod editing;
mod export;
mod grid_view;
mod interactions;
mod presentation;
mod table;
mod transaction;
mod view;

use presentation::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod scroll_tests;
