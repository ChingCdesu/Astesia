use std::{path::PathBuf, sync::Arc};

use editor::{Editor, EditorEvent};
use gpui::{
    actions, App, ClickEvent, ClipboardItem, Entity, FocusHandle, Focusable, FontWeight,
    PathPromptOptions, PromptButton, PromptLevel, Subscription,
};
use multi_buffer::MultiBufferOffset;
use search::{
    buffer_search::{Deploy, DeployReplace, Dismiss, DivRegistrar, FocusEditor},
    BufferSearchBar, ReplaceAll, ReplaceNext, SelectNextMatch, SelectPreviousMatch, ToggleReplace,
};
use serde_json::Value;
use workspace::ToolbarItemView as _;
use zed_ui::prelude::*;

use crate::application::{
    Application, ConnectionWorkspaceSnapshot, QueryDocument, QueryExecutionRequest,
    QueryExecutionScope, QueryFileCompletion, QueryFileError, QueryFileRequest, QueryOperation,
    QueryTarget, QueryWorkspaceState,
};
use crate::db::{ExplainMode, StatementResult};

use super::localization::text;
use super::shell::ShellSettings;
use super::sql_completion::{self, SqlCompletionHandle};

actions!(
    astesia_query,
    [
        ExecuteQuery,
        ExecuteCurrentQuery,
        ExplainQuery,
        OpenQueryFile,
        SaveQueryFile,
        CopyQueryResults,
        SelectAllQueryResults,
        ClearQueryResultSelection
    ]
);

pub(super) struct QueryDocumentStateChanged;

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
        gpui::KeyBinding::new("cmd-o", OpenQueryFile, Some("QueryItem > Editor")),
        gpui::KeyBinding::new("ctrl-o", OpenQueryFile, Some("QueryItem > Editor")),
        gpui::KeyBinding::new("cmd-s", SaveQueryFile, Some("QueryItem > Editor")),
        gpui::KeyBinding::new("ctrl-s", SaveQueryFile, Some("QueryItem > Editor")),
        gpui::KeyBinding::new("cmd-f", Deploy::find(), Some("QueryItem > Editor")),
        gpui::KeyBinding::new("ctrl-f", Deploy::find(), Some("QueryItem > Editor")),
        gpui::KeyBinding::new("cmd-alt-f", DeployReplace, Some("QueryItem > Editor")),
        gpui::KeyBinding::new("ctrl-h", DeployReplace, Some("QueryItem > Editor")),
        gpui::KeyBinding::new("cmd-g", SelectNextMatch, Some("QueryItem")),
        gpui::KeyBinding::new("ctrl-g", SelectNextMatch, Some("QueryItem")),
        gpui::KeyBinding::new("cmd-shift-g", SelectPreviousMatch, Some("QueryItem")),
        gpui::KeyBinding::new("ctrl-shift-g", SelectPreviousMatch, Some("QueryItem")),
        gpui::KeyBinding::new("cmd-c", CopyQueryResults, Some("QueryResultGrid")),
        gpui::KeyBinding::new("ctrl-c", CopyQueryResults, Some("QueryResultGrid")),
        gpui::KeyBinding::new("cmd-a", SelectAllQueryResults, Some("QueryResultGrid")),
        gpui::KeyBinding::new("ctrl-a", SelectAllQueryResults, Some("QueryResultGrid")),
        gpui::KeyBinding::new("escape", ClearQueryResultSelection, Some("QueryResultGrid")),
        gpui::KeyBinding::new("escape", Dismiss, Some("BufferSearchBar")),
        gpui::KeyBinding::new("tab", FocusEditor, Some("BufferSearchBar")),
        gpui::KeyBinding::new("enter", SelectNextMatch, Some("BufferSearchBar")),
        gpui::KeyBinding::new("shift-enter", SelectPreviousMatch, Some("BufferSearchBar")),
        gpui::KeyBinding::new("cmd-shift-h", ToggleReplace, Some("BufferSearchBar")),
        gpui::KeyBinding::new("ctrl-h", ToggleReplace, Some("BufferSearchBar")),
        gpui::KeyBinding::new(
            "enter",
            ReplaceNext,
            Some("BufferSearchBar && in_replace > Editor"),
        ),
        gpui::KeyBinding::new(
            "cmd-enter",
            ReplaceAll,
            Some("BufferSearchBar && in_replace > Editor"),
        ),
        gpui::KeyBinding::new(
            "ctrl-enter",
            ReplaceAll,
            Some("BufferSearchBar && in_replace > Editor"),
        ),
    ]);
}

pub(super) struct QueryItem {
    application: Arc<Application>,
    editor: Entity<Editor>,
    search: Entity<BufferSearchBar>,
    result_focus: FocusHandle,
    completion: SqlCompletionHandle,
    state: QueryWorkspaceState,
    file_prompt_active: bool,
    settings: Entity<ShellSettings>,
    _editor_subscription: Subscription,
    _search_observation: Subscription,
    _settings_observation: Subscription,
}

impl QueryItem {
    pub(super) fn new(
        application: Arc<Application>,
        editor: Entity<Editor>,
        settings: Entity<ShellSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_text = editor.read(cx).text(cx);
        let search = cx.new(|cx| {
            let mut search = BufferSearchBar::new(None, window, cx);
            search.set_active_pane_item(Some(&editor), window, cx);
            search
        });
        let completion =
            sql_completion::install(application.query_completions().clone(), &editor, cx);
        let editor_subscription = cx.subscribe(&editor, |item, editor, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::Edited { .. }) {
                let was_dirty = item.state.is_file_dirty();
                let text = editor.read(cx).text(cx);
                if item.state.update_document_text(text) {
                    if was_dirty != item.state.is_file_dirty() {
                        cx.emit(QueryDocumentStateChanged);
                    }
                    cx.notify();
                }
            }
        });
        let search_observation = cx.observe(&search, |_, _, cx| cx.notify());
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        Self {
            application,
            editor,
            search,
            result_focus: cx.focus_handle(),
            completion,
            state: QueryWorkspaceState::new(initial_text),
            file_prompt_active: false,
            settings,
            _editor_subscription: editor_subscription,
            _search_observation: search_observation,
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
        self.completion.set_target(target.clone());
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

    pub(super) fn file_display_name(&self) -> Option<String> {
        self.state.file_display_name()
    }

    pub(super) fn has_unsaved_changes(&self) -> bool {
        self.state.is_file_dirty()
    }

    pub(super) fn document_label(&self, fallback: &str) -> String {
        let label = self
            .file_display_name()
            .unwrap_or_else(|| fallback.to_string());
        if self.has_unsaved_changes() {
            format!("{label} •")
        } else {
            label
        }
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
        self.completion.set_target(None);
        if self.state.set_target(None) {
            cx.notify();
        }
    }

    fn open_query_file_action(
        &mut self,
        _: &OpenQueryFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_to_open_query_file(window, cx);
    }

    fn open_query_file_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_to_open_query_file(window, cx);
    }

    fn prompt_to_open_query_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_operation_busy() {
            return;
        }
        if !self.state.is_file_dirty() {
            self.show_open_query_file_picker(window, cx);
            return;
        }

        self.file_prompt_active = true;
        cx.notify();
        let language = self.settings.read(cx).language();
        let answer = window.prompt(
            PromptLevel::Warning,
            text(
                language,
                "打开其他查询并放弃未保存的更改？",
                "Open another query and discard unsaved changes?",
            ),
            Some(text(
                language,
                "当前查询的未保存更改将会丢失。",
                "Unsaved changes in the current query will be lost.",
            )),
            &[
                PromptButton::ok(text(language, "放弃并打开", "Discard and Open")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |item, cx| {
            let should_open = answer.await.ok() == Some(0);
            item.update_in(cx, |item, window, cx| {
                item.file_prompt_active = false;
                if should_open {
                    item.show_open_query_file_picker(window, cx);
                } else {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn show_open_query_file_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_operation_busy() {
            return;
        }
        self.file_prompt_active = true;
        cx.notify();
        let language = self.settings.read(cx).language();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(text(language, "打开 SQL 查询", "Open SQL Query").into()),
        });
        cx.spawn_in(window, async move |item, cx| {
            let response = prompt.await;
            item.update_in(cx, |item, window, cx| {
                item.file_prompt_active = false;
                let paths = match response {
                    Ok(Ok(paths)) => paths,
                    Ok(Err(error)) => {
                        item.set_file_prompt_error("query_file_open_prompt_failed", error, cx);
                        return;
                    }
                    Err(error) => {
                        item.set_file_prompt_error("query_file_open_prompt_failed", error, cx);
                        return;
                    }
                };
                let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else {
                    cx.notify();
                    return;
                };
                item.start_open_query_file(path, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn save_query_file_action(
        &mut self,
        _: &SaveQueryFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_query_file(window, cx);
    }

    fn save_query_file_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_query_file(window, cx);
    }

    fn show_find_click(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| {
            search.deploy(&Deploy::find(), None, window, cx);
        });
    }

    fn save_query_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_operation_busy() {
            return;
        }
        if let Some(path) = self.state.file_path().map(PathBuf::from) {
            self.start_save_query_file(path, window, cx);
            return;
        }

        self.file_prompt_active = true;
        cx.notify();
        let prompt = cx.prompt_for_new_path(&PathBuf::default(), Some("query.sql"));
        cx.spawn_in(window, async move |item, cx| {
            let response = prompt.await;
            item.update_in(cx, |item, window, cx| {
                item.file_prompt_active = false;
                let path = match response {
                    Ok(Ok(path)) => path,
                    Ok(Err(error)) => {
                        item.set_file_prompt_error("query_file_save_prompt_failed", error, cx);
                        return;
                    }
                    Err(error) => {
                        item.set_file_prompt_error("query_file_save_prompt_failed", error, cx);
                        return;
                    }
                };
                let Some(path) = path else {
                    cx.notify();
                    return;
                };
                item.start_save_query_file(path, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn start_open_query_file(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(request) = self.state.begin_open_file(path) {
            self.run_file_request(request, window, cx);
        }
    }

    fn start_save_query_file(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(request) = self.state.begin_save_file(path) {
            self.run_file_request(request, window, cx);
        }
    }

    fn run_file_request(
        &mut self,
        request: QueryFileRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
        let execution_request = request.clone();
        let task = gpui_tokio::Tokio::spawn(cx, async move { execution_request.execute().await });
        cx.spawn_in(window, async move |item, cx| {
            let result = match task.await {
                Ok(result) => result,
                Err(error) => Err(QueryFileError::task(error)),
            };
            item.update_in(cx, |item, window, cx| {
                let completion = item.state.finish_file_operation(&request, result);
                let document_state_changed = completion.is_some();
                match completion {
                    Some(QueryFileCompletion::Opened(text)) => {
                        item.state.clear_results();
                        item.editor
                            .update(cx, |editor, cx| editor.set_text(text, window, cx));
                        window.focus(&item.editor.read(cx).focus_handle(cx), cx);
                    }
                    Some(QueryFileCompletion::Saved) | None => {}
                }
                if document_state_changed {
                    cx.emit(QueryDocumentStateChanged);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_file_prompt_error(
        &mut self,
        code: &'static str,
        error: impl std::fmt::Display,
        cx: &mut Context<Self>,
    ) {
        self.state
            .set_file_error(QueryFileError::prompt(code, error));
        cx.notify();
    }

    fn file_operation_busy(&self) -> bool {
        self.file_prompt_active || self.state.is_file_busy()
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

    fn select_result_cell(
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

    fn select_result_row(
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

    fn copy_query_results(&mut self, _: &CopyQueryResults, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_result_selection(false, cx);
    }

    fn copy_query_results_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_result_selection(false, cx);
    }

    fn copy_query_results_with_headers_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_result_selection(true, cx);
    }

    fn copy_result_selection(&self, include_headers: bool, cx: &mut Context<Self>) {
        if let Some(tsv) = self.state.result_selection_tsv(include_headers) {
            cx.write_to_clipboard(ClipboardItem::new_string(tsv));
        }
    }

    fn select_all_query_results(
        &mut self,
        _: &SelectAllQueryResults,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.select_all_result_rows() {
            cx.notify();
        }
    }

    fn select_all_query_results_click(
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

    fn clear_query_result_selection(
        &mut self,
        _: &ClearQueryResultSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_result_selection(cx);
    }

    fn clear_query_result_selection_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_result_selection(cx);
    }

    fn clear_result_selection(&mut self, cx: &mut Context<Self>) {
        if self.state.clear_result_selection() {
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

    fn search_for_actions(
        &self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Entity<BufferSearchBar>> {
        Some(self.search.clone())
    }
}

impl gpui::EventEmitter<QueryDocumentStateChanged> for QueryItem {}

impl Render for QueryItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let language = self.settings.read(cx).language();
        let busy = self.state.is_running();
        let file_busy = self.file_operation_busy();
        let file_label = self.document_label(text(language, "未命名查询", "Untitled Query"));
        let file_error = self.state.file_error().map(|error| {
            format!(
                "{}: {} ({})",
                text(language, "查询文件操作失败", "Query file operation failed"),
                error.message,
                error.code
            )
        });
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
        let can_select_result_rows = self
            .state
            .active_result()
            .is_some_and(|result| !result.columns.is_empty() && !result.rows.is_empty());
        let has_result_selection = self.state.has_result_selection();
        let search_visible = !self.search.read(cx).is_dismissed();

        let content = v_flex()
            .key_context("QueryItem")
            .on_action(cx.listener(Self::execute_all))
            .on_action(cx.listener(Self::execute_current))
            .on_action(cx.listener(Self::explain))
            .on_action(cx.listener(Self::open_query_file_action))
            .on_action(cx.listener(Self::save_query_file_action))
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
                        Button::new("open-query-file", text(language, "打开", "Open"))
                            .size(ButtonSize::Compact)
                            .disabled(file_busy)
                            .key_binding(zed_ui::KeyBinding::for_action(&OpenQueryFile, cx))
                            .on_click(cx.listener(Self::open_query_file_click)),
                    )
                    .child(
                        Button::new("save-query-file", text(language, "保存", "Save"))
                            .size(ButtonSize::Compact)
                            .disabled(file_busy)
                            .key_binding(zed_ui::KeyBinding::for_action(&SaveQueryFile, cx))
                            .on_click(cx.listener(Self::save_query_file_click)),
                    )
                    .child(
                        Button::new("find-query", text(language, "查找", "Find"))
                            .size(ButtonSize::Compact)
                            .key_binding(zed_ui::KeyBinding::for_action(&Deploy::find(), cx))
                            .on_click(cx.listener(Self::show_find_click)),
                    )
                    .child(Label::new(file_label).size(LabelSize::XSmall).truncate())
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
            .children(file_error.map(|error| {
                h_flex()
                    .min_h(px(28.0))
                    .flex_none()
                    .items_center()
                    .px_3()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        Label::new(error)
                            .size(LabelSize::XSmall)
                            .color(Color::Error),
                    )
            }))
            .children(search_visible.then(|| {
                div()
                    .flex_none()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(self.search.clone())
            }))
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
                    .gap_1()
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
                    )
                    .when(can_select_result_rows, |bar| {
                        bar.child(
                            Button::new(
                                "select-all-query-results",
                                text(language, "全选", "Select All"),
                            )
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(Self::select_all_query_results_click)),
                        )
                    })
                    .when(has_result_selection, |bar| {
                        bar.child(
                            Button::new("copy-query-results", text(language, "复制", "Copy"))
                                .size(ButtonSize::Compact)
                                .on_click(cx.listener(Self::copy_query_results_click)),
                        )
                        .child(
                            Button::new(
                                "copy-query-results-with-headers",
                                text(language, "复制含表头", "Copy with Headers"),
                            )
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(Self::copy_query_results_with_headers_click)),
                        )
                        .child(
                            Button::new(
                                "clear-query-result-selection",
                                text(language, "取消选择", "Clear Selection"),
                            )
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(Self::clear_query_result_selection_click)),
                        )
                    }),
            )
            .children(self.render_result_tabs(cx))
            .child(div().flex_1().min_h_0().child(self.render_results(cx)));

        let mut search_actions = DivRegistrar::new(Self::search_for_actions, cx);
        BufferSearchBar::register(&mut search_actions);
        search_actions.into_div().size_full().child(content)
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
    use assets::Assets;
    use gpui::TestAppContext;

    use super::*;
    use crate::{
        connection_repository::SharedConnectionRepository,
        credential_vault::test_support::MemoryCredentialVault,
        platform::DesktopPreferences,
        ui::{bind_editor_keys, sql_language},
    };

    #[test]
    fn result_cells_preserve_scalar_and_structured_values() {
        assert_eq!(display_value(&Value::Null), "NULL");
        assert_eq!(display_value(&Value::String("二".to_string())), "二");
        assert_eq!(
            display_value(&serde_json::json!({ "ok": true })),
            "{\"ok\":true}"
        );
    }

    #[gpui::test]
    fn native_find_replace_preserves_focus_and_grouped_undo(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Assets.load_test_fonts(cx);
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(release_channel::AppVersion::load("0.0.0", None, None), cx);
            gpui_tokio::init(cx);
            editor::init(cx);
            sql_language::init(cx);
            bind_editor_keys(cx);
            bind_query_item_keys(cx);
        });

        let directory = tempfile::tempdir().expect("query search repository directory");
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        );
        let application = Arc::new(Application::with_repository(repository));
        let settings = cx.new(|_| ShellSettings::new(DesktopPreferences::default(), None));
        let mut editor = None;
        let window = cx.add_window(|window, cx| {
            let query_editor =
                cx.new(|cx| sql_language::editor("SELECT 1;\nSELECT 1;", window, cx));
            editor = Some(query_editor.clone());
            QueryItem::new(application, query_editor, settings, window, cx)
        });
        let item = window.root(cx).expect("query item root");
        let editor = editor.expect("query editor");
        let search = item.read_with(cx, |item, _| item.search.clone());
        window
            .update(cx, |item, window, cx| item.focus(window, cx))
            .expect("query window");

        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "cmd-f");
        cx.simulate_keystrokes(window.into(), "s e l e c t");
        cx.run_until_parked();
        assert!(!search.read_with(cx, |search, _| search.is_dismissed()));
        assert_eq!(
            search.read_with(cx, |search, cx| search.query(cx)),
            "select"
        );

        cx.simulate_keystrokes(window.into(), "cmd-shift-h");
        cx.simulate_keystrokes(window.into(), "u p d a t e");
        assert_eq!(
            search.update(cx, |search, cx| search.replacement(cx)),
            "update"
        );
        cx.simulate_keystrokes(window.into(), "cmd-enter");
        cx.run_until_parked();
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor.text(cx)),
            "update 1;\nupdate 1;"
        );

        cx.simulate_keystrokes(window.into(), "escape cmd-z");
        assert!(search.read_with(cx, |search, _| search.is_dismissed()));
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor.text(cx)),
            "SELECT 1;\nSELECT 1;"
        );
    }
}
