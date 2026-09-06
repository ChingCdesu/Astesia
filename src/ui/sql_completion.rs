use super::text_editor::Editor;
use crate::application::{QueryCompletionRequest, QueryCompletionService, QueryTarget};
use gpui_kit::component::input::{CompletionProvider, Rope};
#[cfg(test)]
use gpui_kit::AppContext as _;
use gpui_kit::{App, Entity, Task, Window};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionResponse, CompletionTextEdit, Position, Range,
    TextEdit,
};
use std::{
    rc::Rc,
    sync::{Arc, RwLock},
};

#[derive(Clone)]
pub(super) struct SqlCompletionHandle {
    state: Arc<RwLock<CompletionState>>,
}
#[derive(Clone, Default)]
struct CompletionState {
    epoch: u64,
    target: Option<QueryTarget>,
}
impl SqlCompletionHandle {
    pub(super) fn set_target(&self, target: Option<QueryTarget>) {
        let mut state = self.state.write().expect("SQL completion state poisoned");
        if state.target != target {
            state.epoch = state.epoch.wrapping_add(1);
            state.target = target;
        }
    }
}
pub(super) fn install(
    service: QueryCompletionService,
    editor: &Entity<Editor>,
    cx: &mut App,
) -> SqlCompletionHandle {
    let state = Arc::new(RwLock::new(CompletionState::default()));
    let provider = Rc::new(SqlCompletionProvider {
        service,
        state: state.clone(),
    });
    if let Some(editor) = editor.read(cx).code_state().cloned() {
        editor.update(cx, |editor, _| {
            editor.lsp_mut().completion_provider = Some(provider)
        });
    }
    SqlCompletionHandle { state }
}
struct SqlCompletionProvider {
    service: QueryCompletionService,
    state: Arc<RwLock<CompletionState>>,
}
impl CompletionProvider for SqlCompletionProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let state = self
            .state
            .read()
            .expect("SQL completion state poisoned")
            .clone();
        let Some(target) = state.target else {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        };
        let text = text.to_string();
        let prefix = &text[..offset];
        let prefix_length = prefix
            .chars()
            .rev()
            .take_while(|c| is_identifier_character(*c))
            .map(char::len_utf8)
            .sum::<usize>();
        let filter = prefix[prefix.len() - prefix_length..].to_lowercase();
        let range = Range::new(
            lsp_position(&text, offset - prefix_length),
            lsp_position(&text, offset),
        );
        let request = QueryCompletionRequest {
            target,
            text_before_cursor: prefix.to_owned(),
        };
        let service = self.service.clone();
        let current_state = self.state.clone();
        #[cfg(not(test))]
        let load = super::runtime::spawn(cx, async move { service.complete(request).await });
        #[cfg(test)]
        let load =
            cx.background_spawn(
                async move { Ok::<_, anyhow::Error>(service.complete(request).await) },
            );
        cx.spawn(async move |_| {
            let mut items = load.await?;
            items.retain(|item| completion_matches(&item.label, &filter));
            items.sort_by_key(|item| !item.label.to_lowercase().starts_with(&filter));
            if current_state
                .read()
                .expect("SQL completion state poisoned")
                .epoch
                != state.epoch
            {
                return Ok(CompletionResponse::Array(vec![]));
            }
            Ok(CompletionResponse::Array(
                items
                    .into_iter()
                    .map(|item| CompletionItem {
                        filter_text: Some(
                            item.label.chars().take(filter.chars().count()).collect(),
                        ),
                        label: item.label,
                        detail: Some(item.detail),
                        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                            range,
                            new_text: item.new_text,
                        })),
                        ..Default::default()
                    })
                    .collect(),
            ))
        })
    }
    fn is_completion_trigger(&self, _: usize, text: &str, _: &mut App) -> bool {
        text.chars()
            .last()
            .is_some_and(|c| c == '.' || is_identifier_character(c))
    }
}
fn is_identifier_character(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '$' | '@')
}
fn lsp_position(text: &str, byte_offset: usize) -> Position {
    let prefix = &text[..byte_offset];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
    let character = prefix
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .encode_utf16()
        .count() as u32;
    Position::new(line, character)
}
fn completion_matches(label: &str, query: &str) -> bool {
    let label = label.to_lowercase();
    let mut remaining = label.chars();
    query
        .chars()
        .all(|wanted| remaining.any(|candidate| candidate == wanted))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_positions_use_utf16_columns() {
        let text = "SELECT '😀';\nSELECT 名称";
        assert_eq!(lsp_position(text, "SELECT '😀".len()), Position::new(0, 10));
        assert_eq!(lsp_position(text, text.len()), Position::new(1, 9));
    }
    use gpui_kit::{EntityInputHandler as _, Focusable as _, TestAppContext};

    use crate::{
        application::Application, connection_repository::SharedConnectionRepository,
        credential_vault::test_support::MemoryCredentialVault, db::DbType, ui::sql_language,
    };

    #[gpui_kit::test]
    fn completion_keyboard_flow_filters_accepts_and_dismisses(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx)
        });

        let directory = tempfile::tempdir().expect("completion repository directory");
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        );
        let application = Arc::new(Application::with_repository(repository));
        let window = cx.add_window(|window, cx| sql_language::editor("", window, cx));
        let editor = window.root(cx).expect("editor root");
        let completion =
            cx.update(|cx| install(application.query_completions().clone(), &editor, cx));
        completion.set_target(Some(QueryTarget {
            connection_id: "disconnected-test-profile".to_string(),
            connection_name: "Test".to_string(),
            database: "test".to_string(),
            db_type: DbType::PostgreSQL,
            session_generation: 1,
        }));
        window
            .update(cx, |editor, window, cx| {
                window.focus(&editor.focus_handle(cx), cx);
                editor
                    .code_state()
                    .unwrap()
                    .clone()
                    .update(cx, |state, cx| {
                        state.replace_text_in_range(None, "SEL", window, cx)
                    });
            })
            .expect("editor window");

        window
            .update(cx, |editor, window, cx| {
                editor.show_completions(&super::super::text_editor::ShowCompletions, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "enter");
        cx.run_until_parked();
        assert_eq!(editor.read_with(cx, |editor, cx| editor.text(cx)), "SELECT");

        window
            .update(cx, |editor, window, cx| {
                editor
                    .code_state()
                    .unwrap()
                    .clone()
                    .update(cx, |state, cx| {
                        state.replace_text_in_range(None, " FRO", window, cx)
                    });
            })
            .expect("editor window");
        window
            .update(cx, |editor, window, cx| {
                editor.show_completions(&super::super::text_editor::ShowCompletions, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "down");
        cx.simulate_keystrokes(window.into(), "escape");
        cx.simulate_keystrokes(window.into(), "enter");
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor.text(cx)),
            "SELECT FRO\n"
        );
        cx.simulate_keystrokes(window.into(), "backspace");
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor.text(cx)),
            "SELECT FRO"
        );

        window
            .update(cx, |editor, window, cx| {
                editor.show_completions(&super::super::text_editor::ShowCompletions, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "tab");
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor.text(cx)),
            "SELECT FROM"
        );
    }
}
