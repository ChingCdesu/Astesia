use std::sync::Arc;

use editor::Editor;
use gpui::{actions, App, ClickEvent, Entity, Focusable, FontWeight, Subscription};
use multi_buffer::MultiBufferOffset;
use serde_json::Value;
use zed_ui::prelude::*;

use crate::application::{
    Application, ConnectionWorkspaceSnapshot, QueryDocument, QueryExecutionRequest,
    QueryExecutionScope, QueryOperation, QueryTarget, QueryWorkspaceState,
};
use crate::db::{ExplainMode, StatementResult};

use super::localization::text;
use super::shell::ShellSettings;

actions!(
    astesia_query,
    [ExecuteQuery, ExecuteCurrentQuery, ExplainQuery]
);

pub(super) fn bind_query_item_keys(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("cmd-enter", ExecuteQuery, Some("QueryItem > Editor")),
        gpui::KeyBinding::new("ctrl-enter", ExecuteQuery, Some("QueryItem > Editor")),
        gpui::KeyBinding::new(
            "cmd-shift-enter",
            ExecuteCurrentQuery,
            Some("QueryItem > Editor"),
        ),
        gpui::KeyBinding::new(
            "ctrl-shift-enter",
            ExecuteCurrentQuery,
            Some("QueryItem > Editor"),
        ),
    ]);
}

pub(super) struct QueryItem {
    application: Arc<Application>,
    editor: Entity<Editor>,
    state: QueryWorkspaceState,
    settings: Entity<ShellSettings>,
    _settings_observation: Subscription,
}

impl QueryItem {
    pub(super) fn new(
        application: Arc<Application>,
        editor: Entity<Editor>,
        settings: Entity<ShellSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        Self {
            application,
            editor,
            state: QueryWorkspaceState::default(),
            settings,
            _settings_observation: settings_observation,
        }
    }

    pub(super) fn set_target(
        &mut self,
        target: Option<QueryTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let should_focus = target.is_some();
        if self.state.set_target(target) {
            cx.notify();
        }
        if should_focus {
            window.focus(&self.editor.read(cx).focus_handle(cx), cx);
        }
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.editor.read(cx).focus_handle(cx), cx);
    }

    pub(super) fn invalidate_target(&mut self, target: &QueryTarget, cx: &mut Context<Self>) {
        if self.state.target() == Some(target) {
            self.clear_target(cx);
        }
    }

    pub(super) fn invalidate_session(
        &mut self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<Self>,
    ) {
        let targets_session = self.state.target().is_some_and(|target| {
            target.connection_id == connection_id && target.session_generation == session_generation
        });
        if targets_session {
            self.clear_target(cx);
        }
    }

    pub(super) fn reconcile_sessions(
        &mut self,
        snapshot: &ConnectionWorkspaceSnapshot,
        cx: &mut Context<Self>,
    ) {
        let target_is_live = self.state.target().is_none_or(|target| {
            snapshot.profiles.iter().any(|profile| {
                profile.profile.id == target.connection_id
                    && profile.profile.db_type == target.db_type
                    && profile.session.generation == Some(target.session_generation)
            })
        });
        if !target_is_live {
            self.clear_target(cx);
        }
    }

    fn clear_target(&mut self, cx: &mut Context<Self>) {
        if self.state.set_target(None) {
            cx.notify();
        }
    }

    fn execute_all(&mut self, _: &ExecuteQuery, _: &mut Window, cx: &mut Context<Self>) {
        self.execute(QueryExecutionScope::All, cx);
    }

    fn execute_current(&mut self, _: &ExecuteCurrentQuery, _: &mut Window, cx: &mut Context<Self>) {
        self.execute(QueryExecutionScope::Current, cx);
    }

    fn explain(&mut self, _: &ExplainQuery, _: &mut Window, cx: &mut Context<Self>) {
        self.execute_explain(cx);
    }

    fn execute_all_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.execute(QueryExecutionScope::All, cx);
    }

    fn execute_current_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.execute(QueryExecutionScope::Current, cx);
    }

    fn explain_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.execute_explain(cx);
    }

    fn execute(&mut self, scope: QueryExecutionScope, cx: &mut Context<Self>) {
        if self.state.is_running() {
            return;
        }

        let document = self.query_document(cx);
        let request = match self.state.begin_execution(document, scope) {
            Ok(request) => request,
            Err(_) => {
                cx.notify();
                return;
            }
        };
        self.run_request(request, cx);
    }

    fn execute_explain(&mut self, cx: &mut Context<Self>) {
        if self.state.is_running() {
            return;
        }

        let document = self.query_document(cx);
        let request = match self.state.begin_explain(document) {
            Ok(request) => request,
            Err(_) => {
                cx.notify();
                return;
            }
        };
        self.run_request(request, cx);
    }

    fn query_document(&self, cx: &mut Context<Self>) -> QueryDocument {
        self.editor.update(cx, |editor, cx| {
            let display_snapshot = editor.display_snapshot(cx);
            let selection = editor
                .selections
                .newest::<MultiBufferOffset>(&display_snapshot);
            let selection = selection.range();
            let start: usize = selection.start.0;
            let end: usize = selection.end.0;
            QueryDocument::new(editor.text(cx), start..end)
        })
    }

    fn run_request(&mut self, request: QueryExecutionRequest, cx: &mut Context<Self>) {
        cx.notify();

        let application = self.application.clone();
        let connection_id = request.target.connection_id.clone();
        let database = request.target.database.clone();
        let operation = request.operation.clone();
        let language = self.settings.read(cx).language();
        let execution = gpui_tokio::Tokio::spawn(cx, async move {
            match operation {
                QueryOperation::Statements(statements) => {
                    application
                        .queries()
                        .execute_statements(&connection_id, &database, statements)
                        .await
                }
                QueryOperation::Explain(statement) => application
                    .queries()
                    .explain(&connection_id, &database, statement)
                    .await
                    .map(|result| vec![result]),
            }
        });
        cx.spawn(async move |item, cx| {
            let result = match execution.await {
                Ok(result) => result,
                Err(error) => Err(format!(
                    "{}: {error}",
                    text(
                        language,
                        "查询后台任务意外结束",
                        "The query task ended unexpectedly",
                    )
                )),
            };
            item.update(cx, |item, cx| {
                if item.state.finish_execution(&request, result) {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn clear(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.set_text("", window, cx));
        self.state.clear_results();
        window.focus(&self.editor.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    fn select_result(
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

    fn render_result_tabs(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
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

    fn render_results(&self, cx: &mut Context<Self>) -> AnyElement {
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
                Some(target) if target.db_type.capabilities().sql => text(
                    language,
                    "执行查询后，结果会显示在这里",
                    "Query results will appear here",
                ),
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

    fn render_grid(&self, result: &StatementResult, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let grid_width = px(180.0 * result.columns.len() as f32);
        let header = h_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel_background)
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
                        Some(
                            h_flex()
                                .w_full()
                                .flex_none()
                                .border_b_1()
                                .border_color(colors.border)
                                .when(row_index % 2 == 1, |element| {
                                    element.bg(colors.element_background)
                                })
                                .children(result.columns.iter().enumerate().map(
                                    |(column_index, _)| {
                                        div()
                                            .w(px(180.0))
                                            .flex_none()
                                            .px_2()
                                            .py_1()
                                            .border_r_1()
                                            .border_color(colors.border)
                                            .child(
                                                Label::new(
                                                    row.get(column_index)
                                                        .map(display_value)
                                                        .unwrap_or_default(),
                                                )
                                                .size(LabelSize::XSmall)
                                                .truncate(),
                                            )
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

impl Render for QueryItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let language = self.settings.read(cx).language();
        let busy = self.state.is_running();
        let target = self.state.target();
        let can_execute = target.is_some_and(|target| target.db_type.capabilities().sql);
        let can_explain =
            target.is_some_and(|target| target.db_type.capabilities().explain != ExplainMode::None);
        let target_label = target
            .map(|target| format!("{} / {}", target.connection_name, target.database))
            .unwrap_or_else(|| text(language, "未选择数据库", "No database selected").to_string());
        let result_summary = self.state.active_result().map(|result| {
            if result.success {
                format!(
                    "{} {} · {}ms",
                    result.rows.len(),
                    text(language, "行", "rows"),
                    result.execution_time_ms
                )
            } else {
                format!(
                    "{} · {}ms",
                    text(language, "失败", "Failed"),
                    result.execution_time_ms
                )
            }
        });

        v_flex()
            .key_context("QueryItem")
            .on_action(cx.listener(Self::execute_all))
            .on_action(cx.listener(Self::execute_current))
            .on_action(cx.listener(Self::explain))
            .size_full()
            .overflow_hidden()
            .child(
                h_flex()
                    .h(px(40.0))
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        Button::new("execute-query", text(language, "执行", "Run"))
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Filled)
                            .loading(busy)
                            .disabled(busy || !can_execute)
                            .key_binding(zed_ui::KeyBinding::for_action(&ExecuteQuery, cx))
                            .on_click(cx.listener(Self::execute_all_click)),
                    )
                    .child(
                        Button::new(
                            "execute-current-query",
                            text(language, "执行当前语句", "Run Current Statement"),
                        )
                        .size(ButtonSize::Compact)
                        .disabled(busy || !can_execute)
                        .key_binding(zed_ui::KeyBinding::for_action(&ExecuteCurrentQuery, cx))
                        .on_click(cx.listener(Self::execute_current_click)),
                    )
                    .child(
                        Button::new("explain-query", text(language, "执行计划", "Explain"))
                            .size(ButtonSize::Compact)
                            .disabled(busy || !can_explain)
                            .on_click(cx.listener(Self::explain_click)),
                    )
                    .child(
                        Button::new("clear-query", text(language, "清空", "Clear"))
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Outlined)
                            .disabled(busy)
                            .on_click(cx.listener(Self::clear)),
                    )
                    .child(div().flex_1())
                    .child(Label::new(target_label).size(LabelSize::XSmall).truncate()),
            )
            .child(
                div()
                    .h(px(280.0))
                    .min_h(px(120.0))
                    .flex_none()
                    .bg(colors.background)
                    .child(self.editor.clone()),
            )
            .child(
                h_flex()
                    .h(px(30.0))
                    .flex_none()
                    .items_center()
                    .px_3()
                    .border_t_1()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        Label::new(text(language, "结果", "Results"))
                            .size(LabelSize::XSmall)
                            .weight(FontWeight::SEMIBOLD),
                    )
                    .child(div().flex_1())
                    .children(
                        result_summary.map(|summary| Label::new(summary).size(LabelSize::XSmall)),
                    ),
            )
            .children(self.render_result_tabs(cx))
            .child(div().flex_1().min_h_0().child(self.render_results(cx)))
    }
}

fn centered_message(message: &str, cx: &mut Context<QueryItem>) -> AnyElement {
    v_flex()
        .size_full()
        .justify_center()
        .items_center()
        .text_color(cx.theme().colors().text_muted)
        .child(Label::new(message.to_string()).size(LabelSize::Small))
        .into_any_element()
}

fn display_value(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_cells_preserve_scalar_and_structured_values() {
        assert_eq!(display_value(&Value::Null), "NULL");
        assert_eq!(display_value(&Value::String("二".to_string())), "二");
        assert_eq!(
            display_value(&serde_json::json!({ "ok": true })),
            "{\"ok\":true}"
        );
    }
}
