use super::*;

impl Render for DataGridItem {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let language = self.settings.read(cx).language();
        let status = self.state.status();
        let page = self.state.page();
        let loading = matches!(status, GridSessionStatus::Loading);
        let saving = matches!(status, GridSessionStatus::Saving);
        let unavailable = matches!(status, GridSessionStatus::Unavailable { .. });
        let has_changes = self.state.has_changes();
        let has_unsaved_changes = self.has_unsaved_changes();
        let selected_rows = self.state.selected_row_count();
        let navigation_locked = has_unsaved_changes || saving;
        let query = self.state.query();
        let filter_text = self.filter_editor.read(cx).text(cx);
        let normalized_filter = filter_text.trim();
        let filter_changed = query.filter.as_deref().unwrap_or_default() != normalized_filter;
        let filter_active = query.filter.is_some();
        let filter_blocked = loading || unavailable || navigation_locked;
        let selection_available = self.state.has_selection();
        let export_in_progress = self.export_in_progress;
        let can_previous =
            page.is_some() && query.page > 1 && !loading && !unavailable && !navigation_locked;
        let can_next = page.is_some_and(|page| can_advance(page, query.page, query.page_size))
            && !loading
            && !unavailable
            && !navigation_locked;
        let summary =
            page.map(|page| page_summary(language, query.page, page.rows.len(), page.total_rows));
        let editing_error = self
            .editing
            .as_ref()
            .and_then(|editing| editing.error.clone());
        let editing_null = self
            .editing
            .as_ref()
            .and_then(|editing| editing.column.nullable.then_some(editing.null_requested));
        let editing_default = self.editing.as_ref().and_then(|editing| {
            matches!(editing.target, CellEditorTarget::Draft { .. })
                .then_some(editing.initial_value.is_none() && !editing.modified)
        });
        let expanded_editor = self
            .editing
            .as_ref()
            .filter(|editing| editing.expanded)
            .map(|editing| {
                (
                    editing.editor.clone(),
                    editing.column.name.clone(),
                    editing.column.data_type.clone(),
                    editing.null_requested,
                )
            });
        let notice = match status {
            GridSessionStatus::Failed { error } => {
                Some((Color::Error, IconName::Warning, error.to_string()))
            }
            GridSessionStatus::Unavailable { reason } => {
                Some((Color::Warning, IconName::Warning, reason.to_string()))
            }
            GridSessionStatus::SaveFailed { error } => {
                Some((Color::Error, IconName::Warning, error.to_string()))
            }
            _ => editing_error
                .map(|error| (Color::Error, IconName::Warning, error))
                .or_else(|| self.operation_notice.clone().map(GridNotice::presentation)),
        };
        let editability = self.state.editability();
        let editable = matches!(editability, GridEditability::Editable { .. });
        let editability_warning = !editable || unavailable;
        let editability_message = page.map(|_| {
            if unavailable {
                text(
                    language,
                    "只读 · 连接会话已失效",
                    "Read-only · Connection session is unavailable",
                )
                .to_string()
            } else {
                editability_label(editability, language)
            }
        });
        let changes_message =
            has_changes.then(|| change_summary_message(self.state.change_summary(), language));
        let show_edit_controls = editable || has_unsaved_changes;
        let show_change_bar = page.is_some();
        let show_filter_bar = page.is_some() || filter_active;
        let grid_focused = self.focus_handle.is_focused(window);
        let content = match page {
            Some(page) => self.render_grid(page, grid_focused, cx),
            None => centered_grid_state(status, language),
        };
        let grid_label = page.map_or_else(
            || {
                format!(
                    "{}: {}",
                    self.state.table(),
                    text(language, "表数据", "table data")
                )
            },
            |page| {
                format!(
                    "{}: {}. {} {}, {} {}.",
                    self.state.table(),
                    text(language, "表数据", "table data"),
                    page.rows.len(),
                    text(language, "行", "rows"),
                    page.columns.len(),
                    text(language, "列", "columns")
                )
            },
        );

        v_flex()
            .key_context("DataGridItem")
            .on_action(cx.listener(Self::save_grid_changes))
            .on_action(cx.listener(Self::undo_grid_changes))
            .on_action(cx.listener(Self::discard_grid_changes))
            .on_action(cx.listener(Self::apply_grid_filter))
            .on_action(cx.listener(Self::copy_grid_selection))
            .on_action(cx.listener(Self::paste_grid_selection))
            .size_full()
            .overflow_hidden()
            .bg(colors.background)
            .child(
                h_flex()
                    .h(px(40.0))
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        h_flex()
                            .min_w_0()
                            .flex_1()
                            .gap_2()
                            .child(
                                div().max_w(px(220.0)).child(
                                    Label::new(self.state.table().to_string())
                                        .size(LabelSize::Small)
                                        .weight(FontWeight::SEMIBOLD)
                                        .truncate(),
                                ),
                            )
                            .child(
                                Label::new(format!(
                                    "{} / {}",
                                    self.state.target().connection_name,
                                    self.state.target().database
                                ))
                                .flex_1()
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                            ),
                    )
                    .children(summary.map(|summary| {
                        Label::new(summary)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                    }))
                    .child(
                        Button::new(
                            "previous-data-grid-page",
                            text(language, "上一页", "Previous"),
                        )
                        .size(ButtonSize::Compact)
                        .disabled(!can_previous)
                        .on_click(cx.listener(Self::previous_page)),
                    )
                    .child(
                        Button::new("next-data-grid-page", text(language, "下一页", "Next"))
                            .size(ButtonSize::Compact)
                            .disabled(!can_next)
                            .on_click(cx.listener(Self::next_page)),
                    )
                    .child(
                        Button::new("refresh-data-grid", text(language, "刷新", "Refresh"))
                            .size(ButtonSize::Compact)
                            .loading(loading)
                            .disabled(loading || unavailable || navigation_locked)
                            .on_click(cx.listener(Self::refresh)),
                    ),
            )
            .when(show_filter_bar, |element| {
                element.child(
                    h_flex()
                        .h(px(34.0))
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .border_b_1()
                        .border_color(colors.border)
                        .bg(colors.panel_background)
                        .child(Icon::new(IconName::Filter).size(IconSize::XSmall).color(
                            if filter_active {
                                Color::Accent
                            } else {
                                Color::Muted
                            },
                        ))
                        .child(
                            Label::new("WHERE")
                                .size(LabelSize::XSmall)
                                .weight(FontWeight::SEMIBOLD)
                                .color(Color::Muted),
                        )
                        .child(
                            div()
                                .key_context("DataGridFilter")
                                .h(px(24.0))
                                .min_w(px(160.0))
                                .max_w(px(520.0))
                                .flex_1()
                                .px_1()
                                .border_1()
                                .border_color(colors.border)
                                .bg(colors.editor_background)
                                .child(self.filter_editor.clone()),
                        )
                        .child(
                            Button::new("apply-data-grid-filter", text(language, "应用", "Apply"))
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Filled)
                                .disabled(filter_blocked || !filter_changed)
                                .key_binding(zed_ui::KeyBinding::for_action(&ApplyGridFilter, cx))
                                .on_click(cx.listener(Self::apply_filter_click)),
                        )
                        .child(
                            Button::new("clear-data-grid-filter", text(language, "清除", "Clear"))
                                .size(ButtonSize::Compact)
                                .disabled(
                                    filter_blocked || (!filter_active && filter_text.is_empty()),
                                )
                                .on_click(cx.listener(Self::clear_filter_click)),
                        ),
                )
            })
            .children(notice.map(|(color, icon, message)| {
                h_flex()
                    .min_h(px(30.0))
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(Icon::new(icon).size(IconSize::XSmall).color(color))
                    .child(
                        Label::new(message)
                            .size(LabelSize::XSmall)
                            .color(color)
                            .line_clamp(2),
                    )
            }))
            .when(show_change_bar, |element| {
                element.child(
                    h_flex()
                        .h(px(32.0))
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .border_b_1()
                        .border_color(colors.border)
                        .bg(colors.panel_background)
                        .child(
                            h_flex()
                                .min_w_0()
                                .flex_1()
                                .gap_2()
                                .overflow_hidden()
                                .children(editability_message.map(|message| {
                                    Label::new(message)
                                        .size(LabelSize::XSmall)
                                        .color(if editability_warning {
                                            Color::Warning
                                        } else {
                                            Color::Muted
                                        })
                                        .truncate()
                                }))
                                .children(changes_message.map(|message| {
                                    Label::new(message)
                                        .size(LabelSize::XSmall)
                                        .weight(FontWeight::MEDIUM)
                                        .truncate()
                                })),
                        )
                        .when(show_edit_controls, |bar| {
                            bar.children(editing_default.map(|selected| {
                                Button::new(
                                    "use-default-for-data-grid-cell",
                                    text(language, "使用 DEFAULT", "Use DEFAULT"),
                                )
                                .size(ButtonSize::Compact)
                                .toggle_state(selected)
                                .on_click(cx.listener(Self::use_default_for_draft_click))
                            }))
                            .children(editing_null.map(|selected| {
                                Button::new(
                                    "set-data-grid-cell-null",
                                    text(language, "设为 NULL", "Set NULL"),
                                )
                                .size(ButtonSize::Compact)
                                .toggle_state(selected)
                                .on_click(cx.listener(Self::toggle_cell_null))
                            }))
                            .child(
                                Button::new(
                                    "undo-data-grid-change",
                                    text(language, "撤销", "Undo"),
                                )
                                .size(ButtonSize::Compact)
                                .disabled(!self.state.can_undo() || saving)
                                .key_binding(zed_ui::KeyBinding::for_action(&UndoGridChanges, cx))
                                .on_click(cx.listener(Self::undo_changes_click)),
                            )
                            .child(
                                Button::new(
                                    "discard-data-grid-changes",
                                    text(language, "放弃", "Discard"),
                                )
                                .size(ButtonSize::Compact)
                                .disabled(!has_unsaved_changes || saving)
                                .on_click(cx.listener(Self::discard_changes_click)),
                            )
                            .child(
                                Button::new(
                                    "save-data-grid-changes",
                                    text(language, "保存更改", "Save Changes"),
                                )
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Filled)
                                .loading(saving)
                                .disabled(!has_unsaved_changes || saving || unavailable)
                                .key_binding(zed_ui::KeyBinding::for_action(&SaveGridChanges, cx))
                                .on_click(cx.listener(Self::save_changes_click)),
                            )
                        }),
                )
            })
            .when(show_change_bar, |element| {
                element.child(
                    h_flex()
                        .h(px(32.0))
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .border_b_1()
                        .border_color(colors.border)
                        .bg(colors.panel_background)
                        .child(
                            Label::new(text(language, "选择与行", "Selection & Rows"))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("copy-data-grid-selection", text(language, "复制", "Copy"))
                                .size(ButtonSize::Compact)
                                .disabled(!selection_available)
                                .key_binding(zed_ui::KeyBinding::for_action(&CopyGridSelection, cx))
                                .on_click(cx.listener(Self::copy_selection_click)),
                        )
                        .child(
                            Button::new(
                                "copy-data-grid-selection-with-headers",
                                text(language, "复制含表头", "Copy + Headers"),
                            )
                            .size(ButtonSize::Compact)
                            .disabled(!selection_available)
                            .on_click(cx.listener(Self::copy_selection_with_headers_click)),
                        )
                        .when(self.state.target().db_type == DbType::ClickHouse, |bar| {
                            bar.child(
                                Button::new(
                                    "export-clickhouse-grid-csv",
                                    text(language, "导出 CSV", "Export CSV"),
                                )
                                .size(ButtonSize::Compact)
                                .loading(export_in_progress)
                                .disabled(export_in_progress || page.is_none())
                                .on_click(cx.listener(Self::export_clickhouse_csv)),
                            )
                        })
                        .when(editable, |bar| {
                            bar.child(
                                Button::new(
                                    "paste-data-grid-selection",
                                    text(language, "粘贴", "Paste"),
                                )
                                .size(ButtonSize::Compact)
                                .disabled(saving || unavailable)
                                .key_binding(zed_ui::KeyBinding::for_action(
                                    &PasteGridSelection,
                                    cx,
                                ))
                                .on_click(cx.listener(Self::paste_selection_click)),
                            )
                            .child(
                                Button::new(
                                    "add-data-grid-row",
                                    text(language, "新增行", "Add Row"),
                                )
                                .size(ButtonSize::Compact)
                                .disabled(saving || unavailable)
                                .on_click(cx.listener(Self::add_row_click)),
                            )
                            .child(
                                Button::new(
                                    "delete-selected-data-grid-rows",
                                    text(language, "删除所选行", "Delete Selected"),
                                )
                                .size(ButtonSize::Compact)
                                .disabled(selected_rows == 0 || saving || unavailable)
                                .on_click(cx.listener(Self::delete_selected_rows_click)),
                            )
                        }),
                )
            })
            .children(
                expanded_editor.map(|(editor, column, data_type, null_requested)| {
                    v_flex()
                        .h(px(190.0))
                        .flex_none()
                        .border_b_1()
                        .border_color(colors.border)
                        .bg(colors.editor_background)
                        .child(
                            h_flex()
                                .h(px(32.0))
                                .flex_none()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .border_b_1()
                                .border_color(colors.border)
                                .child(
                                    Label::new(column)
                                        .size(LabelSize::Small)
                                        .weight(FontWeight::SEMIBOLD),
                                )
                                .child(
                                    Label::new(data_type)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .children(null_requested.then(|| {
                                    Label::new("NULL")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Warning)
                                }))
                                .child(div().flex_1())
                                .child(
                                    Label::new(text(language, "⌘↵ 暂存", "⌘↵ Stage"))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .child(
                                    Button::new(
                                        "cancel-expanded-grid-editor",
                                        text(language, "取消", "Cancel"),
                                    )
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(Self::cancel_cell_edit_click)),
                                )
                                .child(
                                    Button::new(
                                        "stage-expanded-grid-editor",
                                        text(language, "暂存值", "Stage Value"),
                                    )
                                    .size(ButtonSize::Compact)
                                    .style(ButtonStyle::Filled)
                                    .on_click(cx.listener(Self::commit_cell_edit_click)),
                                ),
                        )
                        .child(
                            div()
                                .key_context("DataGridLongEditor")
                                .flex_1()
                                .min_h_0()
                                .p_2()
                                .when(null_requested, |element| element.opacity(0.45))
                                .child(editor),
                        )
                }),
            )
            .child(
                div()
                    .id("data-grid-focus-surface")
                    .role(gpui::Role::Grid)
                    .aria_label(grid_label)
                    .tab_index(0)
                    .track_focus(&self.focus_handle)
                    .key_context("DataGrid")
                    .on_action(cx.listener(Self::move_grid_up))
                    .on_action(cx.listener(Self::move_grid_down))
                    .on_action(cx.listener(Self::move_grid_left))
                    .on_action(cx.listener(Self::move_grid_right))
                    .on_action(cx.listener(Self::extend_grid_up))
                    .on_action(cx.listener(Self::extend_grid_down))
                    .on_action(cx.listener(Self::extend_grid_left))
                    .on_action(cx.listener(Self::extend_grid_right))
                    .on_action(cx.listener(Self::begin_active_grid_cell_edit))
                    .on_action(cx.listener(Self::commit_grid_cell_edit))
                    .on_action(cx.listener(Self::cancel_grid_cell_edit))
                    .on_action(cx.listener(Self::select_active_grid_cell))
                    .on_action(cx.listener(Self::select_active_grid_row))
                    .on_action(cx.listener(Self::clear_grid_selection))
                    .flex_1()
                    .min_h_0()
                    .child(content),
            )
    }
}
