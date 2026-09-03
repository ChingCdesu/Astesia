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
use crate::db::{DbType, ExplainMode, StatementResult};

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

const QUERY_EDITOR_CONTEXT: &str = "QueryItem > QueryEditor > Editor";

pub(super) fn bind_query_item_keys(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("cmd-enter", ExecuteQuery, Some(QUERY_EDITOR_CONTEXT)),
        gpui::KeyBinding::new("ctrl-enter", ExecuteQuery, Some(QUERY_EDITOR_CONTEXT)),
        gpui::KeyBinding::new(
            "cmd-shift-enter",
            ExecuteCurrentQuery,
            Some(QUERY_EDITOR_CONTEXT),
        ),
        gpui::KeyBinding::new(
            "ctrl-shift-enter",
            ExecuteCurrentQuery,
            Some(QUERY_EDITOR_CONTEXT),
        ),
        gpui::KeyBinding::new("cmd-o", OpenQueryFile, Some(QUERY_EDITOR_CONTEXT)),
        gpui::KeyBinding::new("ctrl-o", OpenQueryFile, Some(QUERY_EDITOR_CONTEXT)),
        gpui::KeyBinding::new("cmd-s", SaveQueryFile, Some(QUERY_EDITOR_CONTEXT)),
        gpui::KeyBinding::new("ctrl-s", SaveQueryFile, Some(QUERY_EDITOR_CONTEXT)),
        gpui::KeyBinding::new("cmd-f", Deploy::find(), Some(QUERY_EDITOR_CONTEXT)),
        gpui::KeyBinding::new("ctrl-f", Deploy::find(), Some(QUERY_EDITOR_CONTEXT)),
        gpui::KeyBinding::new("cmd-alt-f", DeployReplace, Some(QUERY_EDITOR_CONTEXT)),
        gpui::KeyBinding::new("ctrl-h", DeployReplace, Some(QUERY_EDITOR_CONTEXT)),
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

mod result_view;

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
        self.completion.set_target(
            target
                .as_ref()
                .filter(|target| target.db_type.capabilities().sql)
                .cloned(),
        );
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
        let target = request.target.clone();
        let operation = request.operation.clone();
        let language = self.settings.read(cx).language();
        let execution = gpui_tokio::Tokio::spawn(cx, async move {
            match operation {
                QueryOperation::Statements(statements) => {
                    application
                        .queries()
                        .execute_statements(&target.connection_id, &target.database, statements)
                        .await
                }
                QueryOperation::Explain(statement) => application
                    .queries()
                    .explain(&target.connection_id, &target.database, statement)
                    .await
                    .map(|result| vec![result]),
                QueryOperation::Redis { source, command } => application
                    .redis()
                    .execute(&target, command)
                    .await
                    .map(|result| vec![StatementResult::from_query_result(source, result)]),
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
        let can_execute = target.is_some_and(|target| supports_query_execution(target.db_type));
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
                    .key_context("QueryEditor")
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

fn supports_query_execution(db_type: DbType) -> bool {
    db_type.capabilities().sql || db_type == DbType::Redis
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

pub(super) fn display_value(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests;
