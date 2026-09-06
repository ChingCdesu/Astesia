use super::*;

impl DataGridItem {
    pub(super) fn render_grid(
        &self,
        page: &GridPage,
        grid_focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors();
        let language = self.settings.read(cx).language();
        let query_locked =
            self.has_local_changes() || self.transaction_busy || self.save_recovery_sql.is_some();
        let grid_width = px(ROW_NUMBER_WIDTH
            + (0..page.columns.len())
                .map(|column| self.column_width(column))
                .sum::<f32>());
        let header = h_flex()
            .id("data-grid-header")
            .role(gpui_kit::Role::Row)
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.surface_background)
            .child(
                div()
                    .id("data-grid-row-number-header")
                    .role(gpui_kit::Role::ColumnHeader)
                    .aria_label(text(language, "行号", "Row number"))
                    .w(px(ROW_NUMBER_WIDTH))
                    .flex_none()
                    .px_2()
                    .py_1()
                    .border_r_1()
                    .border_color(colors.border)
                    .child(
                        Label::new("#")
                            .buffer_font(cx)
                            .size(LabelSize::XSmall)
                            .weight(FontWeight::SEMIBOLD),
                    ),
            )
            .children(
                page.columns
                    .iter()
                    .enumerate()
                    .map(|(column_index, column)| {
                        let column_name = column.name.clone();
                        let column_type = column.data_type.clone();
                        let sort_direction = self
                            .state
                            .query()
                            .sort
                            .iter()
                            .find(|sort| sort.column == column.name)
                            .map(|sort| sort.direction);
                        let action_index = column_index;
                        let click_index = column_index;
                        let resize_index = column_index;
                        let column_width = self.column_width(column_index);
                        let sort_label = match sort_direction {
                            Some(GridSortDirection::Ascending) => {
                                text(language, "升序", "ascending")
                            }
                            Some(GridSortDirection::Descending) => {
                                text(language, "降序", "descending")
                            }
                            None => text(language, "未排序", "not sorted"),
                        };
                        let aria_label = if query_locked {
                            format!(
                                "{column_name}, {sort_label}. {}",
                                text(
                                    language,
                                    "保存或放弃更改后才能排序。",
                                    "Save or discard changes before sorting."
                                )
                            )
                        } else {
                            format!("{column_name}, {sort_label}")
                        };
                        h_flex()
                            .id(format!("data-grid-column-{column_index}"))
                            .role(gpui_kit::Role::ColumnHeader)
                            .tab_index(0)
                            .key_context("DataGridColumnHeader")
                            .aria_label(aria_label)
                            .relative()
                            .w(px(column_width))
                            .flex_none()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .border_r_1()
                            .border_color(colors.border)
                            .when(!query_locked, |element| {
                                element
                                    .cursor_pointer()
                                    .hover(|element| element.bg(colors.ghost_element_hover))
                            })
                            .focus_visible(|element| element.bg(colors.ghost_element_selected))
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        Label::new(column_name)
                                            .buffer_font(cx)
                                            .size(LabelSize::XSmall)
                                            .weight(FontWeight::SEMIBOLD)
                                            .truncate(),
                                    )
                                    .child(
                                        Label::new(column_type)
                                            .buffer_font(cx)
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted)
                                            .truncate(),
                                    ),
                            )
                            .children(sort_direction.map(|direction| {
                                Icon::new(match direction {
                                    GridSortDirection::Ascending => IconName::ArrowUp,
                                    GridSortDirection::Descending => IconName::ArrowDown,
                                })
                                .size(IconSize::XSmall)
                                .color(Color::Muted)
                            }))
                            .child(
                                div()
                                    .id(format!("data-grid-column-resize-{column_index}"))
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .w(px(6.0))
                                    .h_full()
                                    .cursor_col_resize()
                                    .on_click(cx.listener(
                                        move |item, event: &ClickEvent, _, cx| {
                                            if event.click_count() >= 2 {
                                                item.reset_column_width(resize_index, cx);
                                            }
                                            cx.stop_propagation();
                                        },
                                    ))
                                    .on_drag(
                                        GridColumnResize {
                                            column: column_index,
                                        },
                                        |_, _, _, cx| cx.new(|_| gpui_kit::Empty),
                                    ),
                            )
                            .when(!query_locked, |element| {
                                element
                                    .on_action(cx.listener(
                                        move |item, _: &menu::Confirm, window, cx| {
                                            item.sort_column(action_index, window, cx);
                                        },
                                    ))
                                    .on_click(cx.listener(move |item, _, window, cx| {
                                        item.sort_column(click_index, window, cx);
                                    }))
                            })
                    }),
            );

        let rows: AnyElement = if page.rows.is_empty() && self.state.drafts().is_empty() {
            v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .gap_1()
                .child(
                    Label::new(text(language, "当前页没有数据", "No rows on this page"))
                        .size(LabelSize::Small),
                )
                .child(
                    Label::new(text(
                        language,
                        "可以返回上一页或刷新数据。",
                        "Go to the previous page or refresh the data.",
                    ))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                )
                .into_any_element()
        } else {
            gpui_kit::uniform_list(
                "data-grid-rows",
                page.rows.len() + self.state.drafts().len(),
                cx.processor(move |item, visible_range: std::ops::Range<usize>, _, cx| {
                    let Some(page) = item.state.page() else {
                        return Vec::new();
                    };
                    let colors = cx.theme().colors();
                    let status = cx.theme().status();
                    let page_offset = u64::from(item.state.query().page.saturating_sub(1))
                        * u64::from(item.state.query().page_size);
                    visible_range
                        .filter_map(|row_index| {
                            if row_index >= page.rows.len() {
                                let draft_index = row_index - page.rows.len();
                                let draft = item.state.drafts().get(draft_index)?.clone();
                                let draft_id = draft.id;
                                let remove_label = text(
                                    language,
                                    "移除这条新行",
                                    "Remove this new row",
                                );
                                return Some(
                                    h_flex()
                                        .id(format!("data-grid-draft-row-{draft_id}"))
                                        .role(gpui_kit::Role::Row)
                                        .aria_label(format!(
                                            "{} {}",
                                            text(language, "未保存的新行", "Unsaved new row"),
                                            draft_index + 1
                                        ))
                                        .w_full()
                                        .flex_none()
                                        .border_b_1()
                                        .border_color(colors.border)
                                        .bg(status.warning_background)
                                        .child(
                                            div()
                                                .id(format!(
                                                    "data-grid-draft-row-header-{draft_id}"
                                                ))
                                                .role(gpui_kit::Role::RowHeader)
                                                .aria_label(format!(
                                                    "{} {}",
                                                    text(language, "新行", "New row"),
                                                    draft_index + 1
                                                ))
                                                .w(px(ROW_NUMBER_WIDTH))
                                                .flex_none()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .border_r_1()
                                                .border_color(colors.border)
                                                .child(
                                                    IconButton::new(
                                                        format!(
                                                            "remove-data-grid-draft-{draft_id}"
                                                        ),
                                                        IconName::Close,
                                                    )
                                                    .icon_size(IconSize::XSmall)
                                                    .tooltip(Tooltip::text(remove_label))
                                                    .on_click(cx.listener(
                                                        move |item, _, window, cx| {
                                                            item.remove_draft_row(
                                                                draft_id, window, cx,
                                                            );
                                                        },
                                                    )),
                                                ),
                                        )
                                        .children(page.columns.iter().enumerate().map(
                                            |(column_index, column)| {
                                                let target = CellEditorTarget::Draft {
                                                    draft_id,
                                                    column: column_index,
                                                };
                                                let value = draft
                                                    .values
                                                    .get(column_index)
                                                    .cloned()
                                                    .flatten();
                                                let displayed = value
                                                    .as_ref()
                                                    .map(display_value)
                                                    .unwrap_or_else(|| "DEFAULT".to_string());
                                                let editor = item
                                                    .editing
                                                    .as_ref()
                                                    .filter(|editing| editing.target == target);
                                                let editing_error = editor
                                                    .and_then(|editing| editing.error.clone());
                                                let mut aria_label = format!(
                                                    "{}, {} {}, {displayed}",
                                                    column.name,
                                                    text(language, "新行", "new row"),
                                                    draft_index + 1
                                                );
                                                if let Some(error) = &editing_error {
                                                    aria_label.push_str(", ");
                                                    aria_label.push_str(error);
                                                }
                                                let cell_content: AnyElement =
                                                    if let Some(editing) = editor {
                                                        if editing.expanded {
                                                            h_flex()
                                                                .size_full()
                                                                .min_h(px(24.0))
                                                                .px_2()
                                                                .border_1()
                                                                .border_color(
                                                                    colors.border_focused,
                                                                )
                                                                .bg(colors.editor_background)
                                                                .child(
                                                                    Label::new(text(
                                                                        language,
                                                                        "正在下方编辑…",
                                                                        "Editing below…",
                                                                    ))
                                                                    .size(LabelSize::XSmall)
                                                                    .color(Color::Accent),
                                                                )
                                                                .into_any_element()
                                                        } else {
                                                            h_flex()
                                                                .key_context("DataGridCellEditor")
                                                                .size_full()
                                                                .min_h(px(24.0))
                                                                .px_1()
                                                                .border_1()
                                                                .border_color(
                                                                    if editing_error.is_some() {
                                                                        status.error_border
                                                                    } else {
                                                                        colors.border_focused
                                                                    },
                                                                )
                                                                .bg(colors.editor_background)
                                                                .when(
                                                                    editing.null_requested,
                                                                    |element| {
                                                                        element.child(
                                                                            Label::new("NULL").buffer_font(cx)
                                                                                .size(
                                                                                    LabelSize::XSmall,
                                                                                )
                                                                                .color(
                                                                                    Color::Warning,
                                                                                ),
                                                                        )
                                                                    },
                                                                )
                                                                .child(
                                                                    div()
                                                                        .min_w_0()
                                                                        .flex_1()
                                                                        .when(
                                                                            editing.null_requested,
                                                                            |element| {
                                                                                element.opacity(0.45)
                                                                            },
                                                                        )
                                                                        .child(
                                                                            editing.editor.clone(),
                                                                        ),
                                                                )
                                                                .into_any_element()
                                                        }
                                                    } else {
                                                        div()
                                                            .px_2()
                                                            .py_1()
                                                            .child(
                                                                Label::new(displayed).buffer_font(cx)
                                                                    .size(LabelSize::XSmall)
                                                                    .truncate(),
                                                            )
                                                            .into_any_element()
                                                    };
                                                div()
                                                    .id(format!(
                                                        "data-grid-draft-cell-{draft_id}-{column_index}"
                                                    ))
                                                    .role(gpui_kit::Role::GridCell)
                                                    .aria_label(aria_label)
                                                    .w(px(item.column_width(column_index)))
                                                    .flex_none()
                                                    .border_r_1()
                                                    .border_color(colors.border)
                                                    .cursor_pointer()
                                                    .child(cell_content)
                                                    .on_click(cx.listener(
                                                        move |item, _, window, cx| {
                                                            item.select_draft_cell(
                                                                draft_id,
                                                                column_index,
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    ))
                                            },
                                        )),
                                );
                            }
                            page.rows.get(row_index)?;
                            let deleted = item.state.is_row_deleted(row_index);
                            let row_selected = item.state.row_selected(row_index);
                            let displayed_row = page_offset + row_index as u64 + 1;
                            Some(
                                h_flex()
                                    .id(format!("data-grid-row-{row_index}"))
                                    .role(gpui_kit::Role::Row)
                                    .aria_selected(row_selected)
                                    .w_full()
                                    .flex_none()
                                    .border_b_1()
                                    .border_color(colors.border)
                                    .when(row_selected, |element| {
                                        element.bg(colors.ghost_element_selected)
                                    })
                                    .when(deleted, |element| {
                                        element.bg(status.warning_background).opacity(0.72)
                                    })
                                    .when(!deleted, |element| {
                                        element.hover(|element| {
                                            element.bg(colors.ghost_element_hover)
                                        })
                                    })
                                    .child(
                                        div()
                                            .id(format!("data-grid-row-header-{row_index}"))
                                            .role(gpui_kit::Role::RowHeader)
                                            .aria_label(format!(
                                                "{} {displayed_row}{}",
                                                text(language, "行", "Row"),
                                                if deleted {
                                                    text(
                                                        language,
                                                        "，已标记为待删除",
                                                        ", marked for deletion",
                                                    )
                                                } else {
                                                    ""
                                                }
                                            ))
                                            .aria_selected(row_selected)
                                            .w(px(ROW_NUMBER_WIDTH))
                                            .flex_none()
                                            .px_2()
                                            .py_1()
                                            .border_r_1()
                                            .border_color(colors.border)
                                            .when(!deleted, |element| element.cursor_pointer())
                                            .child(if deleted {
                                                Icon::new(IconName::Trash)
                                                    .size(IconSize::XSmall)
                                                    .color(Color::Warning)
                                                    .into_any_element()
                                            } else {
                                                Label::new(displayed_row.to_string()).buffer_font(cx)
                                                    .size(LabelSize::XSmall)
                                                    .into_any_element()
                                            })
                                            .when(!deleted, |element| {
                                                element.on_click(cx.listener(
                                                    move |item, event, window, cx| {
                                                        item.select_row(
                                                            row_index, event, window, cx,
                                                        );
                                                    },
                                                ))
                                            }),
                                    )
                                    .children(page.columns.iter().enumerate().map(
                                        |(column_index, _)| {
                                            let cell = GridCell {
                                                row: row_index,
                                                column: column_index,
                                            };
                                            let active = item.active_cell == Some(cell);
                                            let dirty = item.state.is_cell_dirty(cell);
                                            let selected = item
                                                .state
                                                .cell_selection()
                                                .is_some_and(|selection| selection.contains(cell));
                                            let displayed = item
                                                .state
                                                .cell_value(cell)
                                                .map(display_value)
                                                .unwrap_or_default();
                                            let mut aria_label = format!(
                                                "{}, {} {displayed_row}, {displayed}",
                                                page.columns[column_index].name,
                                                text(language, "行", "row")
                                            );
                                            if dirty {
                                                aria_label.push_str(text(
                                                    language,
                                                    "，未保存的更改",
                                                    ", unsaved change",
                                                ));
                                            }
                                            if deleted {
                                                aria_label.push_str(text(
                                                    language,
                                                    "，已标记为待删除",
                                                    ", marked for deletion",
                                                ));
                                            }
                                            let editor = item
                                                .editing
                                                .as_ref()
                                                .filter(|editing| {
                                                    editing.target
                                                        == CellEditorTarget::Existing(cell)
                                                });
                                            let editing_error =
                                                editor.and_then(|editing| editing.error.clone());
                                            if let Some(error) = &editing_error {
                                                aria_label.push_str(", ");
                                                aria_label.push_str(error);
                                            }
                                            let cell_content: AnyElement =
                                                if let Some(editing) = editor {
                                                    if editing.expanded {
                                                        h_flex()
                                                            .size_full()
                                                            .min_h(px(24.0))
                                                            .px_2()
                                                            .border_1()
                                                            .border_color(colors.border_focused)
                                                            .bg(colors.editor_background)
                                                            .child(
                                                                Label::new(text(
                                                                    language,
                                                                    "正在下方编辑…",
                                                                    "Editing below…",
                                                                ))
                                                                .size(LabelSize::XSmall)
                                                                .color(Color::Accent),
                                                            )
                                                            .into_any_element()
                                                    } else {
                                                        h_flex()
                                                            .key_context("DataGridCellEditor")
                                                            .size_full()
                                                            .min_h(px(24.0))
                                                            .px_1()
                                                            .border_1()
                                                            .border_color(
                                                                if editing_error.is_some() {
                                                                    status.error_border
                                                                } else {
                                                                    colors.border_focused
                                                                },
                                                            )
                                                            .bg(colors.editor_background)
                                                            .when(
                                                                editing.null_requested,
                                                                |element| {
                                                                    element.child(
                                                                        Label::new("NULL").buffer_font(cx)
                                                                            .size(
                                                                                LabelSize::XSmall,
                                                                            )
                                                                            .color(
                                                                                Color::Warning,
                                                                            ),
                                                                    )
                                                                },
                                                            )
                                                            .child(
                                                                div()
                                                                    .min_w_0()
                                                                    .flex_1()
                                                                    .when(
                                                                        editing.null_requested,
                                                                        |element| {
                                                                            element.opacity(0.45)
                                                                        },
                                                                    )
                                                                    .child(
                                                                        editing.editor.clone(),
                                                                    ),
                                                            )
                                                            .into_any_element()
                                                    }
                                                } else {
                                                    div()
                                                        .px_2()
                                                        .py_1()
                                                        .child(
                                                            Label::new(displayed).buffer_font(cx)
                                                                .size(LabelSize::XSmall)
                                                                .truncate(),
                                                        )
                                                        .into_any_element()
                                                };
                                            div()
                                                .id(format!(
                                                    "data-grid-cell-{row_index}-{column_index}"
                                                ))
                                                .relative()
                                                .role(gpui_kit::Role::GridCell)
                                                .aria_label(aria_label)
                                                .aria_selected(selected)
                                                .when(active, |element| {
                                                    element.aria_active_descendant()
                                                })
                                                .w(px(item.column_width(column_index)))
                                                .flex_none()
                                                .border_r_1()
                                                .border_color(colors.border)
                                                .when(!deleted, |element| {
                                                    element.cursor_pointer()
                                                })
                                                .when(dirty, |element| {
                                                    element.bg(status.warning_background)
                                                })
                                                .when(selected, |element| {
                                                    element.bg(colors.ghost_element_selected)
                                                })
                                                .when(active && grid_focused, |element| {
                                                    element.border_color(colors.border_focused)
                                                })
                                                .when(dirty, |element| {
                                                    element.child(
                                                        div()
                                                            .absolute()
                                                            .top_0()
                                                            .right_0()
                                                            .m_1()
                                                            .child(
                                                                Indicator::dot()
                                                                    .color(Color::Warning),
                                                            ),
                                                    )
                                                })
                                                .child(cell_content)
                                                .when(!deleted, |element| {
                                                    element.on_click(cx.listener(
                                                        move |item, event, window, cx| {
                                                            item.select_cell(
                                                                cell, event, window, cx,
                                                            );
                                                        },
                                                    ))
                                                })
                                        },
                                    )),
                            )
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .w_full()
            .flex_1()
            .track_scroll(&self.rows_scroll_handle)
            .into_any_element()
        };

        div()
            .id("data-grid")
            .size_full()
            .overflow_x_scroll()
            .track_scroll(&self.horizontal_scroll_handle)
            .on_drag_move::<GridColumnResize>(cx.listener(|item, event, _, cx| {
                item.resize_column(event, cx);
            }))
            .child(
                v_flex()
                    .w(grid_width)
                    .min_w_full()
                    .h_full()
                    .child(header)
                    .child(rows),
            )
            .into_any_element()
    }
}
