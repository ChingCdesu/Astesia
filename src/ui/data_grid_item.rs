use std::sync::Arc;

use editor::{Editor, EditorEvent};
use gpui::{
    actions, point, App, ClickEvent, ClipboardItem, DragMoveEvent, Entity, FocusHandle,
    Focusable as _, FontWeight, PromptButton, PromptLevel, ScrollHandle, ScrollStrategy,
    Subscription, UniformListScrollHandle,
};
use language::Buffer;
use serde_json::Value;
use zed_ui::{prelude::*, Indicator, Tooltip};

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

#[derive(Clone)]
struct GridColumnResize {
    column: usize,
}

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
        gpui::KeyBinding::new("enter", menu::Confirm, Some("DataGridColumnHeader")),
        gpui::KeyBinding::new("space", menu::Confirm, Some("DataGridColumnHeader")),
        gpui::KeyBinding::new("up", MoveGridUp, Some("DataGrid")),
        gpui::KeyBinding::new("down", MoveGridDown, Some("DataGrid")),
        gpui::KeyBinding::new("left", MoveGridLeft, Some("DataGrid")),
        gpui::KeyBinding::new("right", MoveGridRight, Some("DataGrid")),
        gpui::KeyBinding::new("shift-up", ExtendGridUp, Some("DataGrid")),
        gpui::KeyBinding::new("shift-down", ExtendGridDown, Some("DataGrid")),
        gpui::KeyBinding::new("shift-left", ExtendGridLeft, Some("DataGrid")),
        gpui::KeyBinding::new("shift-right", ExtendGridRight, Some("DataGrid")),
        gpui::KeyBinding::new("enter", BeginActiveGridCellEdit, Some("DataGrid")),
        gpui::KeyBinding::new("space", SelectActiveGridCell, Some("DataGrid")),
        gpui::KeyBinding::new("shift-space", SelectActiveGridRow, Some("DataGrid")),
        gpui::KeyBinding::new("escape", ClearGridSelection, Some("DataGrid")),
        gpui::KeyBinding::new(
            "enter",
            CommitGridCellEdit,
            Some("DataGridCellEditor > Editor"),
        ),
        gpui::KeyBinding::new(
            "escape",
            CancelGridCellEdit,
            Some("DataGridCellEditor > Editor"),
        ),
        gpui::KeyBinding::new(
            "cmd-enter",
            CommitGridCellEdit,
            Some("DataGridLongEditor > Editor"),
        ),
        gpui::KeyBinding::new(
            "ctrl-enter",
            CommitGridCellEdit,
            Some("DataGridLongEditor > Editor"),
        ),
        gpui::KeyBinding::new(
            "escape",
            CancelGridCellEdit,
            Some("DataGridLongEditor > Editor"),
        ),
        gpui::KeyBinding::new("cmd-s", SaveGridChanges, Some("DataGridItem")),
        gpui::KeyBinding::new("ctrl-s", SaveGridChanges, Some("DataGridItem")),
        gpui::KeyBinding::new("cmd-z", UndoGridChanges, Some("DataGrid")),
        gpui::KeyBinding::new("ctrl-z", UndoGridChanges, Some("DataGrid")),
        gpui::KeyBinding::new("cmd-c", CopyGridSelection, Some("DataGrid")),
        gpui::KeyBinding::new("ctrl-c", CopyGridSelection, Some("DataGrid")),
        gpui::KeyBinding::new("cmd-v", PasteGridSelection, Some("DataGrid")),
        gpui::KeyBinding::new("ctrl-v", PasteGridSelection, Some("DataGrid")),
        gpui::KeyBinding::new("enter", ApplyGridFilter, Some("DataGridFilter > Editor")),
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
            Self::Warning(message) => (Color::Warning, IconName::Warning, message),
            Self::Error(message) => (Color::Error, IconName::Warning, message),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GridExportFormat {
    Csv,
    Json,
    Xlsx,
}

pub(super) struct DataGridItem {
    application: Arc<Application>,
    state: GridSession,
    focus_handle: FocusHandle,
    active_cell: Option<GridCell>,
    editing: Option<ActiveCellEditor>,
    operation_notice: Option<GridNotice>,
    export_in_progress: bool,
    filter_editor: Entity<Editor>,
    column_widths: Vec<f32>,
    rows_scroll_handle: UniformListScrollHandle,
    horizontal_scroll_handle: ScrollHandle,
    chart: Option<Entity<ChartView>>,
    showing_chart: bool,
    chart_generation: u64,
    chart_loading: bool,
    chart_error: Option<String>,
    settings: Entity<ShellSettings>,
    _filter_observation: Subscription,
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
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let filter_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("status = 'active'", window, cx);
            editor
        });
        let filter_observation = cx.subscribe(&filter_editor, |_, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::BufferEdited) {
                cx.notify();
            }
        });
        let mut item = Self {
            application,
            state: GridSession::new(target, table, DEFAULT_GRID_PAGE_SIZE)
                .expect("default grid page size must be valid"),
            focus_handle: cx.focus_handle(),
            active_cell: None,
            editing: None,
            operation_notice: None,
            export_in_progress: false,
            filter_editor,
            column_widths: Vec::new(),
            rows_scroll_handle: UniformListScrollHandle::new(),
            horizontal_scroll_handle: ScrollHandle::new(),
            chart: None,
            showing_chart: false,
            chart_generation: 0,
            chart_loading: false,
            chart_error: None,
            settings,
            _filter_observation: filter_observation,
            _settings_observation: settings_observation,
        };
        item.load(cx);
        item
    }

    pub(super) fn label(&self, cx: &App) -> String {
        let language = self.settings.read(cx).language();
        format!(
            "{} · {} · {}/{}",
            self.state.table(),
            text(language, "数据", "Data"),
            self.state.target().connection_name,
            self.state.target().database,
        )
    }

    pub(super) fn has_unsaved_changes(&self) -> bool {
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
            chart.update(cx, |chart, cx| chart.replace_data(columns, &rows, cx));
        } else {
            let model = ChartModel::from_names(columns, &rows);
            self.chart = Some(cx.new(|cx| ChartView::new(model, self.settings.clone(), cx)));
        }
    }
}

mod editing;
mod export;
mod grid_view;
mod interactions;
mod presentation;
mod view;

use presentation::*;

#[cfg(test)]
mod tests;
