use super::*;

impl DataGridItem {
    pub(super) fn load(&mut self, cx: &mut Context<Self>) {
        let request = match self.state.begin_load() {
            Ok(request) => request,
            Err(_) => return,
        };
        self.cancel_chart_load();
        cx.notify();

        let application = self.application.clone();
        let load_request = request.clone();
        let transaction = self.transaction.clone();
        let load = crate::ui::runtime::spawn(cx, async move {
            application
                .grids()
                .load_in(&load_request, transaction.as_ref())
                .await
        });
        cx.spawn(async move |item, cx| {
            let result = match load.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(format!("Grid background task failed: {error}")),
            };
            item.update(cx, |item, cx| {
                if item.state.finish_load(&request, result) {
                    item.normalize_active_cell();
                    item.normalize_column_widths();
                    if item.showing_chart {
                        item.load_chart(cx);
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn prepare_for_navigation(&mut self) -> bool {
        if self.has_local_changes() || self.transaction_busy || self.save_recovery_sql.is_some() {
            return false;
        }
        self.editing = None;
        true
    }

    pub(super) fn refresh(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_active(cx);
    }

    pub(in crate::ui) fn refresh_active(&mut self, cx: &mut Context<Self>) {
        if !self.prepare_for_navigation() {
            return;
        }
        self.operation_notice = None;
        if self.showing_chart {
            self.load_chart(cx);
        } else {
            self.load(cx);
        }
    }

    fn load_chart(&mut self, cx: &mut Context<Self>) {
        self.cancel_chart_load();
        let generation = self.chart_generation;
        self.chart_loading = true;
        self.chart_error = None;
        cx.notify();
        let service = self.application.charts().clone();
        let target = self.state.target().clone();
        let table = self.state.table().clone();
        let query = self.state.query().clone();
        let cancellation = ChartLoadCancellation::default();
        let cancelled = cancellation.cancelled.clone();
        self.chart_load = Some(cancellation);
        let load = crate::ui::runtime::spawn(cx, async move {
            service.table_data(target, table, query, &cancelled).await
        });
        cx.spawn(async move |item, cx| {
            let result = match load.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(format!("Chart refresh ended unexpectedly: {error}")),
            };
            item.update(cx, |item, cx| {
                if item.chart_generation != generation {
                    return;
                }
                item.chart_loading = false;
                match result {
                    Ok(Some(data)) => {
                        item.chart_error = None;
                        if let Some(chart) = &item.chart {
                            chart.update(cx, |chart, cx| {
                                chart.replace_data(data.columns, data.rows, cx)
                            });
                        } else {
                            let model = ChartModel::from_names(data.columns, data.rows);
                            item.chart =
                                Some(cx.new(|cx| ChartView::new(model, item.settings.clone(), cx)));
                        }
                    }
                    Ok(None) => {}
                    Err(error) => item.chart_error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn cancel_chart_load(&mut self) {
        self.chart_generation = self.chart_generation.saturating_add(1);
        self.chart_load = None;
        self.chart_loading = false;
    }

    pub(super) fn toggle_chart(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.showing_chart && self.has_unsaved_changes() {
            return;
        }
        if self.chart.is_none() {
            self.sync_chart(cx);
        }
        self.showing_chart = !self.showing_chart;
        if self.showing_chart {
            self.load_chart(cx);
            if let Some(chart) = &self.chart {
                window.focus(&chart.read(cx).focus_handle(cx), cx);
            }
        } else {
            self.cancel_chart_load();
            window.focus(&self.focus_handle, cx);
        }
        cx.notify();
    }

    pub(super) fn previous_page(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.prepare_for_navigation() {
            return;
        }
        let page = self.state.query().page;
        if page > 1 && self.state.set_page(page - 1).unwrap_or(false) {
            self.operation_notice = None;
            self.load(cx);
        }
    }

    pub(super) fn next_page(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.prepare_for_navigation() {
            return;
        }
        let page = self.state.query().page;
        if self.state.set_page(page.saturating_add(1)).unwrap_or(false) {
            self.operation_notice = None;
            self.load(cx);
        }
    }

    pub(super) fn sort_column(
        &mut self,
        column_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.prepare_for_navigation() {
            return;
        }
        let Some(column) = self
            .state
            .page()
            .and_then(|page| page.columns.get(column_index))
            .map(|column| column.name.clone())
        else {
            return;
        };
        let sort = next_sort(&column, &self.state.query().sort);
        let filter = self.state.query().filter.clone();
        let sort_text = GridSort::format_list(&sort);
        if self.state.set_query_options(filter, sort).unwrap_or(false) {
            self.sort_editor
                .update(cx, |editor, cx| editor.set_text(sort_text, window, cx));
            self.operation_notice = None;
            self.load(cx);
        }
    }

    pub(super) fn apply_grid_filter(
        &mut self,
        _: &ApplyGridFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_filter(window, cx);
    }

    pub(super) fn apply_filter_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_filter(window, cx);
    }

    pub(super) fn apply_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.commit_cell_edit(window, cx) || self.state.has_changes() {
            return;
        }
        let filter = self.filter_editor.read(cx).text(cx);
        let sort = match GridSort::parse_list(&self.sort_editor.read(cx).text(cx)) {
            Ok(sort) => sort,
            Err(error) => {
                self.operation_notice = Some(GridNotice::Error(format!(
                    "{}: {error}",
                    text(
                        self.settings.read(cx).language(),
                        "排序无效",
                        "Invalid ordering"
                    )
                )));
                cx.notify();
                return;
            }
        };
        match self.state.set_query_options(Some(filter), sort) {
            Ok(true) => {
                self.operation_notice = None;
                self.load(cx);
            }
            Ok(false) => {}
            Err(error) => {
                let language = self.settings.read(cx).language();
                self.operation_notice =
                    Some(GridNotice::Error(grid_error_message(error, language)));
                cx.notify();
            }
        }
    }

    pub(super) fn clear_filter_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.commit_cell_edit(window, cx) || self.state.has_changes() {
            return;
        }
        match self.state.set_filter(None) {
            Ok(changed) => {
                self.filter_editor.update(cx, |editor, cx| {
                    editor.set_text("", window, cx);
                });
                self.operation_notice = None;
                if changed {
                    self.load(cx);
                } else {
                    cx.notify();
                }
            }
            Err(error) => {
                let language = self.settings.read(cx).language();
                self.operation_notice =
                    Some(GridNotice::Error(grid_error_message(error, language)));
                cx.notify();
            }
        }
    }

    pub(super) fn copy_grid_selection(
        &mut self,
        _: &CopyGridSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing.is_some() {
            cx.propagate();
            return;
        }
        self.copy_selection(false, cx);
    }

    pub(super) fn copy_selection_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selection(false, cx);
    }

    pub(super) fn copy_selection_with_headers_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selection(true, cx);
    }

    pub(super) fn copy_selection(&self, include_headers: bool, cx: &mut Context<Self>) {
        if let Some(tsv) = self.state.selection_tsv(include_headers) {
            cx.write_to_clipboard(ClipboardItem::new_string(tsv));
        }
    }

    pub(super) fn paste_grid_selection(
        &mut self,
        _: &PasteGridSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing.is_some() {
            cx.propagate();
            return;
        }
        self.paste_selection(cx);
    }

    pub(super) fn paste_selection_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.paste_selection(cx);
    }

    pub(super) fn paste_selection(&mut self, cx: &mut Context<Self>) {
        let language = self.settings.read(cx).language();
        let Some(text_value) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            self.operation_notice = Some(GridNotice::Warning(
                text(
                    language,
                    "剪贴板中没有可粘贴的文本。",
                    "The clipboard does not contain text to paste.",
                )
                .to_string(),
            ));
            cx.notify();
            return;
        };
        let Some(page) = self.state.page() else {
            return;
        };
        let anchor = self
            .state
            .cell_selection()
            .map(GridCellSelection::top_left)
            .or(self.active_cell);
        let assignments = match grid_paste_assignments(page, anchor, &text_value) {
            Ok(assignments) => assignments,
            Err(error) => {
                self.operation_notice =
                    Some(GridNotice::Error(grid_paste_error_message(error, language)));
                cx.notify();
                return;
            }
        };
        match self.state.stage_cell_values(assignments) {
            Ok(changed) if changed > 0 => {
                self.operation_notice = Some(GridNotice::Success(format!(
                    "{} {}",
                    changed,
                    text(
                        language,
                        "个单元格已暂存。保存更改后才会写入数据库。",
                        "cell(s) staged. Save Changes to apply."
                    )
                )));
                cx.notify();
            }
            Ok(_) => {}
            Err(error) => {
                self.operation_notice =
                    Some(GridNotice::Error(grid_error_message(error, language)));
                cx.notify();
            }
        }
    }

    pub(super) fn normalize_active_cell(&mut self) {
        self.active_cell = clamped_active_cell(self.state.page(), self.active_cell);
    }

    pub(super) fn normalize_column_widths(&mut self) {
        let count = self.state.page().map_or(0, |page| page.columns.len());
        self.column_widths.resize(count, COLUMN_WIDTH);
        self.column_widths.truncate(count);
    }

    pub(super) fn column_width(&self, column: usize) -> f32 {
        self.column_widths
            .get(column)
            .copied()
            .unwrap_or(COLUMN_WIDTH)
    }

    pub(super) fn resize_column(
        &mut self,
        event: &DragMoveEvent<GridColumnResize>,
        cx: &mut Context<Self>,
    ) {
        let column = event.drag(cx).column;
        let left = ROW_NUMBER_WIDTH
            + (0..column)
                .map(|index| self.column_width(index))
                .sum::<f32>();
        let content_x = (event.event.position.x
            - event.bounds.left()
            - self.horizontal_scroll_handle.offset().x)
            .as_f32();
        let width = (content_x - left).clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
        if let Some(current) = self.column_widths.get_mut(column) {
            if (*current - width).abs() > f32::EPSILON {
                *current = width;
                cx.notify();
            }
        }
    }

    pub(super) fn reset_column_width(&mut self, column: usize, cx: &mut Context<Self>) {
        if let Some(width) = self.column_widths.get_mut(column) {
            if (*width - COLUMN_WIDTH).abs() > f32::EPSILON {
                *width = COLUMN_WIDTH;
                cx.notify();
            }
        }
    }

    pub(super) fn move_active_cell(
        &mut self,
        row_delta: isize,
        column_delta: isize,
        extend_selection: bool,
        cx: &mut Context<Self>,
    ) {
        if self.editing.is_some() {
            return;
        }
        let Some(page) = self.state.page() else {
            return;
        };
        let Some(current) = clamped_active_cell(Some(page), self.active_cell) else {
            return;
        };
        let destination = moved_grid_cell(page, current, row_delta, column_delta);
        let active_changed = self.active_cell != Some(destination);
        self.active_cell = Some(destination);

        let mut selection_changed = false;
        if extend_selection {
            if self.state.cell_selection().is_none() {
                selection_changed |= self.state.select_cell(current, false).unwrap_or(false);
            }
            selection_changed |= self.state.select_cell(destination, true).unwrap_or(false);
        }
        if row_delta != 0 {
            self.rows_scroll_handle.scroll_to_item(
                destination.row,
                if row_delta < 0 {
                    ScrollStrategy::Top
                } else {
                    ScrollStrategy::Bottom
                },
            );
        }
        if column_delta != 0 {
            self.reveal_column(destination.column);
        }
        if active_changed || selection_changed {
            cx.notify();
        }
    }

    pub(super) fn reveal_column(&self, column: usize) {
        let current = self.horizontal_scroll_handle.offset();
        let viewport_width = self.horizontal_scroll_handle.bounds().size.width;
        let max_offset = self.horizontal_scroll_handle.max_offset().x;
        let column_start = px(ROW_NUMBER_WIDTH
            + (0..column)
                .map(|index| self.column_width(index))
                .sum::<f32>());
        let column_end = column_start + px(self.column_width(column));
        let visible_start = -current.x;
        let visible_end = visible_start + viewport_width;
        let next_x = if column_start < visible_start {
            -column_start
        } else if column_end > visible_end {
            viewport_width - column_end
        } else {
            current.x
        }
        .clamp(-max_offset, px(0.0));
        if next_x != current.x {
            self.horizontal_scroll_handle
                .set_offset(point(next_x, current.y));
        }
    }

    pub(super) fn move_grid_up(&mut self, _: &MoveGridUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_active_cell(-1, 0, false, cx);
    }

    pub(super) fn move_grid_down(
        &mut self,
        _: &MoveGridDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_active_cell(1, 0, false, cx);
    }

    pub(super) fn move_grid_left(
        &mut self,
        _: &MoveGridLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_active_cell(0, -1, false, cx);
    }

    pub(super) fn move_grid_right(
        &mut self,
        _: &MoveGridRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_active_cell(0, 1, false, cx);
    }

    pub(super) fn extend_grid_up(
        &mut self,
        _: &ExtendGridUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_active_cell(-1, 0, true, cx);
    }

    pub(super) fn extend_grid_down(
        &mut self,
        _: &ExtendGridDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_active_cell(1, 0, true, cx);
    }

    pub(super) fn extend_grid_left(
        &mut self,
        _: &ExtendGridLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_active_cell(0, -1, true, cx);
    }

    pub(super) fn extend_grid_right(
        &mut self,
        _: &ExtendGridRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_active_cell(0, 1, true, cx);
    }

    pub(super) fn select_active_grid_cell(
        &mut self,
        _: &SelectActiveGridCell,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cell) = clamped_active_cell(self.state.page(), self.active_cell) else {
            return;
        };
        self.active_cell = Some(cell);
        if self.state.select_cell(cell, false).unwrap_or(false) {
            cx.notify();
        }
    }

    pub(super) fn select_active_grid_row(
        &mut self,
        _: &SelectActiveGridRow,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cell) = clamped_active_cell(self.state.page(), self.active_cell) else {
            return;
        };
        self.active_cell = Some(cell);
        if self
            .state
            .select_row(cell.row, GridRowSelectionMode::Replace)
            .unwrap_or(false)
        {
            cx.notify();
        }
    }

    pub(super) fn clear_grid_selection(
        &mut self,
        _: &ClearGridSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.clear_selection() {
            cx.notify();
        }
    }

    pub(super) fn select_row(
        &mut self,
        row: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.commit_cell_edit(window, cx) {
            cx.stop_propagation();
            return;
        }
        window.focus(&self.focus_handle, cx);
        if let Some(page) = self.state.page() {
            self.active_cell = clamped_active_cell(
                Some(page),
                Some(GridCell {
                    row,
                    column: self.active_cell.map_or(0, |cell| cell.column),
                }),
            );
        }
        let modifiers = event.modifiers();
        let mode = if modifiers.shift {
            GridRowSelectionMode::Extend
        } else if modifiers.secondary() {
            GridRowSelectionMode::Toggle
        } else {
            GridRowSelectionMode::Replace
        };
        if self.state.select_row(row, mode).unwrap_or(false) {
            cx.notify();
        }
        cx.stop_propagation();
    }

    pub(super) fn select_cell(
        &mut self,
        cell: GridCell,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editing
            .as_ref()
            .is_some_and(|editing| editing.target == CellEditorTarget::Existing(cell))
        {
            cx.stop_propagation();
            return;
        }
        if !self.commit_cell_edit(window, cx) {
            cx.stop_propagation();
            return;
        }
        window.focus(&self.focus_handle, cx);
        self.active_cell = Some(cell);
        if self
            .state
            .select_cell(cell, event.modifiers().shift)
            .unwrap_or(false)
        {
            cx.notify();
        }
        if event.click_count() >= 2 {
            self.begin_cell_edit(cell, window, cx);
        }
        cx.stop_propagation();
    }

    pub(super) fn add_row_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.commit_cell_edit(window, cx) {
            return;
        }
        let draft_id = match self.state.stage_insert() {
            Ok(draft_id) => draft_id,
            Err(error) => {
                let language = self.settings.read(cx).language();
                self.operation_notice =
                    Some(GridNotice::Error(grid_error_message(error, language)));
                cx.notify();
                return;
            }
        };
        let first_column = self
            .state
            .page()
            .and_then(|page| {
                page.columns
                    .iter()
                    .position(|column| !column.is_primary_key)
                    .or_else(|| (!page.columns.is_empty()).then_some(0))
            })
            .unwrap_or(0);
        self.operation_notice = None;
        self.begin_draft_cell_edit(draft_id, first_column, window, cx);
    }

    pub(super) fn select_draft_cell(
        &mut self,
        draft_id: u64,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = CellEditorTarget::Draft { draft_id, column };
        if self
            .editing
            .as_ref()
            .is_some_and(|editing| editing.target == target)
        {
            return;
        }
        if !self.commit_cell_edit(window, cx) {
            return;
        }
        self.begin_draft_cell_edit(draft_id, column, window, cx);
    }

    pub(super) fn remove_draft_row(
        &mut self,
        draft_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editing
            .as_ref()
            .is_some_and(|editing| matches!(editing.target, CellEditorTarget::Draft { draft_id: active, .. } if active == draft_id))
        {
            self.cancel_cell_edit(window, cx);
        } else if !self.commit_cell_edit(window, cx) {
            return;
        }
        match self.state.remove_draft(draft_id) {
            Ok(_) => {
                self.operation_notice = None;
                window.focus(&self.focus_handle, cx);
                cx.notify();
            }
            Err(error) => {
                let language = self.settings.read(cx).language();
                self.operation_notice =
                    Some(GridNotice::Error(grid_error_message(error, language)));
                cx.notify();
            }
        }
    }

    pub(super) fn delete_selected_rows_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.commit_cell_edit(window, cx) || window.has_active_prompt() {
            return;
        }
        let count = self.state.selected_row_count();
        if count == 0 {
            return;
        }
        let language = self.settings.read(cx).language();
        let title = if count == 1 {
            text(
                language,
                "将所选行标记为待删除？",
                "Mark the selected row for deletion?",
            )
        } else {
            text(
                language,
                "将所选行标记为待删除？",
                "Mark the selected rows for deletion?",
            )
        };
        let detail = if count == 1 {
            text(
                language,
                "保存更改后，这一行将从数据库中删除。",
                "This row will be deleted from the database when you save changes.",
            )
        } else {
            text(
                language,
                "保存更改后，这些行将从数据库中删除。",
                "These rows will be deleted from the database when you save changes.",
            )
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            title,
            Some(detail),
            &[
                PromptButton::ok(text(language, "标记为待删除", "Mark for Deletion")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |item, cx| {
            if answer.await.ok() != Some(0) {
                return;
            }
            item.update_in(cx, |item, window, cx| {
                let language = item.settings.read(cx).language();
                match item.state.stage_delete_selection() {
                    Ok(true) => {
                        item.active_cell = None;
                        item.operation_notice = Some(GridNotice::Warning(if count == 1 {
                            text(
                                language,
                                "所选行已标记为待删除。保存更改后才会写入数据库。",
                                "The selected row is marked for deletion. Save Changes to apply.",
                            )
                            .to_string()
                        } else {
                            format!(
                                "{} {}",
                                count,
                                text(
                                    language,
                                    "行已标记为待删除。保存更改后才会写入数据库。",
                                    "rows are marked for deletion. Save Changes to apply."
                                )
                            )
                        }));
                        window.focus(&item.focus_handle, cx);
                        cx.notify();
                    }
                    Ok(false) => {}
                    Err(error) => {
                        item.operation_notice =
                            Some(GridNotice::Error(grid_error_message(error, language)));
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }
}
