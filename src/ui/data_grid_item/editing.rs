use super::*;

impl DataGridItem {
    pub(super) fn begin_active_grid_cell_edit(
        &mut self,
        _: &BeginActiveGridCellEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cell) = clamped_active_cell(self.state.page(), self.active_cell) else {
            return;
        };
        self.begin_cell_edit(cell, window, cx);
    }

    pub(super) fn begin_cell_edit(
        &mut self,
        cell: GridCell,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editing) = &self.editing {
            if editing.target == CellEditorTarget::Existing(cell) {
                window.focus(&editing.editor.read(cx).focus_handle(cx), cx);
            }
            return;
        }
        let language = self.settings.read(cx).language();
        let blocked = match self.state.status() {
            GridSessionStatus::Loading => Some(text(
                language,
                "表格仍在加载。",
                "The grid is still loading.",
            )),
            GridSessionStatus::Saving => {
                Some(text(language, "更改正在保存。", "Changes are being saved."))
            }
            GridSessionStatus::Unavailable { reason } => Some(reason),
            _ => None,
        }
        .map(str::to_string);
        if let Some(message) = blocked {
            self.operation_notice = Some(GridNotice::Warning(message));
            cx.notify();
            return;
        }
        if !matches!(self.state.editability(), GridEditability::Editable { .. }) {
            self.operation_notice = Some(GridNotice::Warning(editability_label(
                self.state.editability(),
                language,
            )));
            cx.notify();
            return;
        }
        let Some((column, value)) = self.state.page().and_then(|page| {
            Some((
                page.columns.get(cell.column)?.clone(),
                self.state.cell_value(cell)?.clone(),
            ))
        }) else {
            return;
        };
        self.active_cell = Some(cell);
        self.state.select_cell(cell, false).ok();
        self.open_cell_editor(
            CellEditorTarget::Existing(cell),
            column,
            Some(value),
            window,
            cx,
        );
    }

    pub(super) fn begin_draft_cell_edit(
        &mut self,
        draft_id: u64,
        column_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            self.state.status(),
            GridSessionStatus::Loading
                | GridSessionStatus::Saving
                | GridSessionStatus::Unavailable { .. }
        ) {
            return;
        }
        if let Some(editing) = &self.editing {
            if editing.target
                == (CellEditorTarget::Draft {
                    draft_id,
                    column: column_index,
                })
            {
                window.focus(&editing.editor.read(cx).focus_handle(cx), cx);
            }
            return;
        }
        let Some((column, value)) = self.state.page().and_then(|page| {
            Some((
                page.columns.get(column_index)?.clone(),
                self.state
                    .drafts()
                    .iter()
                    .find(|draft| draft.id == draft_id)?
                    .values
                    .get(column_index)?
                    .clone(),
            ))
        }) else {
            return;
        };
        self.active_cell = None;
        self.state.clear_selection();
        self.open_cell_editor(
            CellEditorTarget::Draft {
                draft_id,
                column: column_index,
            },
            column,
            value,
            window,
            cx,
        );
    }

    pub(super) fn open_cell_editor(
        &mut self,
        target: CellEditorTarget,
        column: GridColumn,
        value: Option<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial_text = value.as_ref().map(edit_value).unwrap_or_default();
        let expanded = column.kind() == GridColumnKind::Json
            || initial_text.contains(['\n', '\r'])
            || initial_text.chars().count() > 80;
        let language = self.settings.read(cx).language();
        let editor = if expanded {
            cx.new(|cx| Editor::code(&initial_text, "json", window, cx))
        } else {
            cx.new(|cx| {
                let mut editor = Editor::inline_single_line(
                    text(language, "单元格值", "Cell value"),
                    px(12.0),
                    window,
                    cx,
                );
                editor.set_text(initial_text.clone(), window, cx);
                editor
            })
        };
        let observation = cx.subscribe(&editor, |item, editor, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::Change) {
                let current_text = editor.read(cx).text(cx);
                if let Some(editing) = &mut item.editing {
                    editing.null_requested = false;
                    editing.modified = cell_editor_modified(
                        editing.initial_value.as_ref(),
                        &editing.initial_text,
                        &current_text,
                        false,
                    );
                    editing.error = None;
                    cx.notify();
                }
            }
        });
        self.editing = Some(ActiveCellEditor {
            target,
            column,
            editor: editor.clone(),
            initial_value: value.clone(),
            initial_text,
            null_requested: value.as_ref().is_some_and(Value::is_null),
            modified: false,
            expanded,
            error: None,
            _observation: observation,
        });
        self.operation_notice = None;
        window.focus(&editor.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn commit_grid_cell_edit(
        &mut self,
        _: &CommitGridCellEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_cell_edit(window, cx);
    }

    pub(super) fn commit_cell_edit_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_cell_edit(window, cx);
    }

    pub(super) fn commit_cell_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(editing) = self.editing.as_ref() else {
            return true;
        };
        if !editing.modified {
            self.editing = None;
            window.focus(&self.focus_handle, cx);
            cx.notify();
            return true;
        }
        let language = self.settings.read(cx).language();
        let target = editing.target;
        let column = editing.column.clone();
        let editor = editing.editor.clone();
        let input = editor.read(cx).text(cx);
        let value = match column.parse_input(&input, editing.null_requested) {
            Ok(value) => value,
            Err(error) => {
                if let Some(editing) = &mut self.editing {
                    editing.error = Some(cell_input_error_message(error, language));
                }
                window.focus(&editor.read(cx).focus_handle(cx), cx);
                cx.notify();
                return false;
            }
        };
        let staged = match target {
            CellEditorTarget::Existing(cell) => self.state.stage_cell_value(cell, value),
            CellEditorTarget::Draft { draft_id, column } => {
                self.state.set_draft_value(draft_id, column, value)
            }
        };
        match staged {
            Ok(_) => {
                self.editing = None;
                self.operation_notice = None;
                window.focus(&self.focus_handle, cx);
                cx.notify();
                true
            }
            Err(error) => {
                if let Some(editing) = &mut self.editing {
                    editing.error = Some(grid_error_message(error, language));
                }
                window.focus(&editor.read(cx).focus_handle(cx), cx);
                cx.notify();
                false
            }
        }
    }

    pub(super) fn cancel_grid_cell_edit(
        &mut self,
        _: &CancelGridCellEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_cell_edit(window, cx);
    }

    pub(super) fn cancel_cell_edit_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_cell_edit(window, cx);
    }

    pub(super) fn cancel_cell_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.take().is_some() {
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    pub(super) fn toggle_cell_null(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self
            .editing
            .as_ref()
            .filter(|editing| editing.column.nullable)
            .map(|editing| editing.editor.clone())
        else {
            return;
        };
        let current_text = editor.read(cx).text(cx);
        let editing = self.editing.as_mut().expect("active editor must exist");
        editing.null_requested = !editing.null_requested;
        editing.modified = cell_editor_modified(
            editing.initial_value.as_ref(),
            &editing.initial_text,
            &current_text,
            editing.null_requested,
        );
        editing.error = None;
        window.focus(&editor.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn use_default_for_draft_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((draft_id, column)) =
            self.editing
                .as_ref()
                .and_then(|editing| match editing.target {
                    CellEditorTarget::Draft { draft_id, column } => Some((draft_id, column)),
                    CellEditorTarget::Existing(_) => None,
                })
        else {
            return;
        };
        match self.state.unset_draft_value(draft_id, column) {
            Ok(_) => {
                self.editing = None;
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

    pub(super) fn save_grid_changes(
        &mut self,
        _: &SaveGridChanges,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_changes(window, cx);
    }

    pub(super) fn save_changes_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_changes(window, cx);
    }

    pub(super) fn save_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.commit_cell_edit(window, cx) || !self.state.has_changes() {
            return;
        }
        let primary_key_edits = self.state.primary_key_edit_count();
        if primary_key_edits > 0 {
            if window.has_active_prompt() {
                return;
            }
            let language = self.settings.read(cx).language();
            let answer = window.prompt(
                PromptLevel::Warning,
                text(
                    language,
                    "保存主键更改？",
                    "Save primary-key changes?",
                ),
                Some(text(
                    language,
                    "主键用于定位原始行。Astesia 将先保存其他字段，再更改主键。",
                    "Primary keys identify the original rows. Astesia will save other fields before changing their keys.",
                )),
                &[
                    PromptButton::ok(text(language, "保存更改", "Save Changes")),
                    PromptButton::cancel(text(language, "取消", "Cancel")),
                ],
                cx,
            );
            cx.spawn_in(window, async move |item, cx| {
                if answer.await.ok() != Some(0) {
                    return;
                }
                item.update(cx, |item, cx| item.start_save(cx)).ok();
            })
            .detach();
            return;
        }
        self.start_save(cx);
    }

    pub(super) fn start_save(&mut self, cx: &mut Context<Self>) {
        if self.save_recovery_sql.is_some()
            || self.transaction_busy
            || (self.manual_transaction
                && self
                    .transaction
                    .as_ref()
                    .is_none_or(|transaction| transaction.is_closed()))
        {
            return;
        }
        let request = match self.state.begin_save() {
            Ok(request) => request,
            Err(error) => {
                let language = self.settings.read(cx).language();
                self.operation_notice =
                    Some(GridNotice::Error(grid_error_message(error, language)));
                cx.notify();
                return;
            }
        };
        self.operation_notice = None;
        cx.notify();

        let application = self.application.clone();
        let save_request = request.clone();
        let transaction = self.transaction.clone();
        let isolation = self.transaction_isolation;
        let save = crate::ui::runtime::spawn(cx, async move {
            if let Some(transaction) = transaction.as_ref() {
                application
                    .grids()
                    .save_in(&save_request, Some(transaction))
                    .await
            } else {
                application
                    .grids()
                    .save_with_isolation(&save_request, isolation)
                    .await
            }
        });
        cx.spawn(async move |item, cx| {
            let result = match save.await {
                Ok(result) => result,
                Err(error) => Err(GridSaveFailure::before_execution(
                    0,
                    format!("Grid background task failed: {error}"),
                )),
            };
            item.update(cx, |item, cx| {
                let outcome = result.as_ref().ok().copied();
                item.save_recovery_sql = result
                    .as_ref()
                    .err()
                    .and_then(|error| error.recovery_sql.clone());
                if !item.state.finish_save(&request, result.map(|_| ())) {
                    return;
                }
                if let Some(outcome) = outcome {
                    let language = item.settings.read(cx).language();
                    item.operation_notice = Some(if item.manual_transaction {
                        GridNotice::Warning(
                            text(
                                language,
                                "更改已写入事务，尚未提交。",
                                "Changes applied to the transaction; not committed.",
                            )
                            .to_string(),
                        )
                    } else {
                        GridNotice::Success(save_outcome_message(outcome, language))
                    });
                    item.load(cx);
                } else {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn undo_grid_changes(
        &mut self,
        _: &UndoGridChanges,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_changes(window, cx);
    }

    pub(super) fn undo_changes_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_changes(window, cx);
    }

    pub(super) fn undo_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_cell_edit(window, cx);
        if self.state.undo() {
            self.operation_notice = None;
            cx.notify();
        }
    }

    pub(super) fn discard_grid_changes(
        &mut self,
        _: &DiscardGridChanges,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_discard_changes(window, cx);
    }

    pub(super) fn discard_changes_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_discard_changes(window, cx);
    }

    pub(super) fn confirm_discard_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .transaction
            .as_ref()
            .is_some_and(|transaction| transaction.has_pending_changes())
        {
            self.confirm_rollback_transaction(window, cx);
            return;
        }
        if !self.has_unsaved_changes() || window.has_active_prompt() {
            return;
        }
        let language = self.settings.read(cx).language();
        let answer = window.prompt(
            PromptLevel::Warning,
            text(language, "放弃所有表格更改？", "Discard all grid changes?"),
            Some(text(
                language,
                "尚未保存的编辑、新增行和待删除行都将丢失。",
                "Unsaved edits, new rows, and pending deletions will be lost.",
            )),
            &[
                PromptButton::ok(text(language, "放弃更改", "Discard Changes")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |item, cx| {
            if answer.await.ok() != Some(0) {
                return;
            }
            item.update_in(cx, |item, window, cx| {
                item.editing = None;
                item.operation_notice = None;
                let unknown_outcome = item.save_recovery_sql.take().is_some();
                let reload =
                    item.state.discard_changes() && item.state.page().is_none() || unknown_outcome;
                window.focus(&item.focus_handle, cx);
                if reload {
                    item.load(cx);
                } else {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(in crate::ui) fn invalidate_session(
        &mut self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<Self>,
    ) {
        let language = self.settings.read(cx).language();
        if self.state.invalidate_session(
            connection_id,
            session_generation,
            text(
                language,
                "连接会话已更改。请从侧边栏重新打开表数据。",
                "The connection session changed. Reopen the table data from the sidebar.",
            ),
        ) {
            self.cancel_chart_load();
            cx.notify();
        }
    }
}
