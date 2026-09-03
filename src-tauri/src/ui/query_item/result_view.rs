use super::*;

impl QueryItem {
    pub(super) fn select_result(
        &mut self,
        index: usize,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.select_result(index) {
            cx.notify();
        }
    }

    pub(super) fn select_result_cell(
        &mut self,
        row: usize,
        column: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.result_focus, cx);
        if self
            .state
            .select_result_cell(row, column, event.modifiers().shift)
        {
            cx.notify();
        }
        cx.stop_propagation();
    }

    pub(super) fn select_result_row(
        &mut self,
        row: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.result_focus, cx);
        let modifiers = event.modifiers();
        if self
            .state
            .select_result_row(row, modifiers.shift, modifiers.secondary())
        {
            cx.notify();
        }
        cx.stop_propagation();
    }

    pub(super) fn copy_query_results(
        &mut self,
        _: &CopyQueryResults,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_result_selection(false, cx);
    }

    pub(super) fn copy_query_results_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_result_selection(false, cx);
    }

    pub(super) fn copy_query_results_with_headers_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_result_selection(true, cx);
    }

    pub(super) fn copy_result_selection(&self, include_headers: bool, cx: &mut Context<Self>) {
        if let Some(tsv) = self.state.result_selection_tsv(include_headers) {
            cx.write_to_clipboard(ClipboardItem::new_string(tsv));
        }
    }

    pub(super) fn select_all_query_results(
        &mut self,
        _: &SelectAllQueryResults,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.select_all_result_rows() {
            cx.notify();
        }
    }

    pub(super) fn select_all_query_results_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.result_focus, cx);
        if self.state.select_all_result_rows() {
            cx.notify();
        }
    }

    pub(super) fn clear_query_result_selection(
        &mut self,
        _: &ClearQueryResultSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_result_selection(cx);
    }

    pub(super) fn clear_query_result_selection_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_result_selection(cx);
    }

    pub(super) fn clear_result_selection(&mut self, cx: &mut Context<Self>) {
        if self.state.clear_result_selection() {
            cx.notify();
        }
    }

    pub(super) fn render_result_tabs(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let language = self.settings.read(cx).language();
        (self.state.results().len() > 1).then(|| {
            h_flex()
                .id("query-result-tabs")
                .h(px(34.0))
                .flex_none()
                .items_center()
                .gap_1()
                .px_2()
                .overflow_x_scroll()
                .border_b_1()
                .border_color(cx.theme().colors().border)
                .children(
                    self.state
                        .results()
                        .iter()
                        .enumerate()
                        .map(|(index, result)| {
                            let label = if result.success {
                                format!("#{}  {}ms", index + 1, result.execution_time_ms)
                            } else {
                                format!("#{}  {}", index + 1, text(language, "失败", "Failed"))
                            };
                            Button::new(format!("query-result-{index}"), label)
                                .size(ButtonSize::Compact)
                                .toggle_state(self.state.active_result_index() == index)
                                .on_click(cx.listener(move |item, event, window, cx| {
                                    item.select_result(index, event, window, cx);
                                }))
                        }),
                )
                .into_any_element()
        })
    }

    pub(super) fn render_results(&self, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings.read(cx).language();
        if self.state.is_running() {
            return centered_message(text(language, "正在执行查询…", "Running query…"), cx);
        }
        if let Some(error) = self.state.error() {
            return v_flex()
                .size_full()
                .justify_center()
                .items_center()
                .gap_2()
                .p_4()
                .child(
                    Label::new(error.message.clone())
                        .size(LabelSize::Small)
                        .color(Color::Error),
                )
                .child(
                    Label::new(format!(
                        "{}{}",
                        text(language, "错误码：", "Error code: "),
                        error.code
                    ))
                    .size(LabelSize::XSmall),
                )
                .into_any_element();
        }
        let Some(result) = self.state.active_result() else {
            let message = match self.state.target() {
                Some(target)
                    if target.db_type.capabilities().sql
                        || target.db_type == crate::db::DbType::Redis =>
                {
                    text(
                        language,
                        "执行查询后，结果会显示在这里",
                        "Query results will appear here",
                    )
                }
                Some(_) => text(
                    language,
                    "当前引擎不支持 SQL 查询",
                    "This engine does not support SQL queries",
                ),
                None => text(
                    language,
                    "请在左侧选择一个已连接的数据库",
                    "Select a connected database in the sidebar",
                ),
            };
            return centered_message(message, cx);
        };
        if !result.success {
            return v_flex()
                .size_full()
                .gap_2()
                .p_4()
                .child(
                    Label::new(
                        result.error.clone().unwrap_or_else(|| {
                            text(language, "查询失败", "Query failed").to_string()
                        }),
                    )
                    .size(LabelSize::Small)
                    .color(Color::Error),
                )
                .child(
                    Label::new(result.sql.clone())
                        .size(LabelSize::XSmall)
                        .line_clamp(6),
                )
                .into_any_element();
        }
        if result.columns.is_empty() {
            return centered_message(
                &format!(
                    "{} {}",
                    text(language, "已影响行数：", "Affected rows:"),
                    result.affected_rows
                ),
                cx,
            );
        }
        self.render_grid(result, cx)
    }

    pub(super) fn render_grid(
        &self,
        result: &StatementResult,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors();
        let grid_width = px(48.0 + 180.0 * result.columns.len() as f32);
        let header = h_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel_background)
            .child(
                div()
                    .w(px(48.0))
                    .flex_none()
                    .px_2()
                    .py_1()
                    .border_r_1()
                    .border_color(colors.border)
                    .child(
                        Label::new("#")
                            .size(LabelSize::XSmall)
                            .weight(FontWeight::SEMIBOLD),
                    ),
            )
            .children(result.columns.iter().map(|column| {
                div()
                    .w(px(180.0))
                    .flex_none()
                    .px_2()
                    .py_1()
                    .border_r_1()
                    .border_color(colors.border)
                    .child(
                        Label::new(column.name.clone())
                            .size(LabelSize::XSmall)
                            .weight(FontWeight::SEMIBOLD)
                            .truncate(),
                    )
            }));
        let rows = gpui::uniform_list(
            "query-result-rows",
            result.rows.len(),
            cx.processor(move |item, visible_range: std::ops::Range<usize>, _, cx| {
                let Some(result) = item.state.active_result() else {
                    return Vec::new();
                };
                let colors = cx.theme().colors();
                visible_range
                    .filter_map(|row_index| {
                        let row = result.rows.get(row_index)?;
                        let row_selected = item.state.is_result_row_selected(row_index);
                        Some(
                            h_flex()
                                .w_full()
                                .flex_none()
                                .border_b_1()
                                .border_color(colors.border)
                                .when(row_index % 2 == 1, |element| {
                                    element.bg(colors.element_background)
                                })
                                .when(row_selected, |element| {
                                    element.bg(colors.ghost_element_selected)
                                })
                                .hover(|element| element.bg(colors.ghost_element_hover))
                                .child(
                                    div()
                                        .id(format!("query-result-row-{row_index}"))
                                        .w(px(48.0))
                                        .flex_none()
                                        .px_2()
                                        .py_1()
                                        .border_r_1()
                                        .border_color(colors.border)
                                        .cursor_pointer()
                                        .child(
                                            Label::new((row_index + 1).to_string())
                                                .size(LabelSize::XSmall),
                                        )
                                        .on_click(cx.listener(move |item, event, window, cx| {
                                            item.select_result_row(row_index, event, window, cx);
                                        })),
                                )
                                .children(result.columns.iter().enumerate().map(
                                    |(column_index, _)| {
                                        let selected = item
                                            .state
                                            .is_result_cell_selected(row_index, column_index);
                                        div()
                                            .id(format!(
                                                "query-result-cell-{row_index}-{column_index}"
                                            ))
                                            .w(px(180.0))
                                            .flex_none()
                                            .px_2()
                                            .py_1()
                                            .border_r_1()
                                            .border_color(colors.border)
                                            .cursor_pointer()
                                            .when(selected, |element| {
                                                element.bg(colors.ghost_element_selected)
                                            })
                                            .child(
                                                Label::new(
                                                    row.get(column_index)
                                                        .map(display_value)
                                                        .unwrap_or_default(),
                                                )
                                                .size(LabelSize::XSmall)
                                                .truncate(),
                                            )
                                            .on_click(cx.listener(
                                                move |item, event, window, cx| {
                                                    item.select_result_cell(
                                                        row_index,
                                                        column_index,
                                                        event,
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            ))
                                    },
                                )),
                        )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .w_full()
        .flex_1();

        div()
            .id("query-result-grid")
            .track_focus(&self.result_focus)
            .key_context("QueryResultGrid")
            .on_action(cx.listener(Self::copy_query_results))
            .on_action(cx.listener(Self::select_all_query_results))
            .on_action(cx.listener(Self::clear_query_result_selection))
            .size_full()
            .overflow_x_scroll()
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
