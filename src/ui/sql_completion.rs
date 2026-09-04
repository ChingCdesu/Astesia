use std::{
    rc::Rc,
    sync::{Arc, RwLock},
};

#[cfg(not(test))]
use anyhow::anyhow;
use editor::{CompletionContext, CompletionProvider, Editor};
use gpui::{AppContext, Context, Entity, Task, Window};
use language::{Anchor, Buffer, CodeLabel, ToOffset as _};
use project::{Completion, CompletionDisplayOptions, CompletionResponse, CompletionSource};

use crate::application::{
    QueryCompletion, QueryCompletionRequest, QueryCompletionService, QueryTarget,
};

#[derive(Clone)]
pub(super) struct SqlCompletionHandle {
    state: Arc<RwLock<CompletionState>>,
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
    cx: &mut impl AppContext,
) -> SqlCompletionHandle {
    let state = Arc::new(RwLock::new(CompletionState::default()));
    let provider = Rc::new(SqlCompletionProvider {
        service,
        state: state.clone(),
    });
    editor.update(cx, |editor, _| {
        editor.set_completion_provider(Some(provider));
        editor.set_show_completions_on_input(Some(true));
    });
    SqlCompletionHandle { state }
}

#[derive(Clone, Default)]
struct CompletionState {
    epoch: u64,
    target: Option<QueryTarget>,
}

struct SqlCompletionProvider {
    service: QueryCompletionService,
    state: Arc<RwLock<CompletionState>>,
}

impl CompletionProvider for SqlCompletionProvider {
    fn completions(
        &self,
        buffer: &Entity<Buffer>,
        buffer_position: Anchor,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut Context<Editor>,
    ) -> Task<anyhow::Result<Vec<CompletionResponse>>> {
        let state = self
            .state
            .read()
            .expect("SQL completion state poisoned")
            .clone();
        let Some(target) = state.target else {
            return Task::ready(Ok(Vec::new()));
        };

        let buffer = buffer.read(cx);
        let offset = buffer_position.to_offset(buffer);
        let prefix_length = buffer
            .reversed_chars_at(buffer_position)
            .take_while(|character| is_identifier_character(*character))
            .map(char::len_utf8)
            .sum::<usize>();
        let replace_start = buffer.anchor_before(offset.saturating_sub(prefix_length));
        let text_before_cursor = buffer.text_for_range(0..offset).collect::<String>();
        let request = QueryCompletionRequest {
            target,
            text_before_cursor,
        };
        let service = self.service.clone();
        let current_state = self.state.clone();
        #[cfg(not(test))]
        let load = {
            let task = gpui_tokio::Tokio::spawn(cx, async move { service.complete(request).await });
            cx.background_spawn(async move {
                task.await
                    .map_err(|error| anyhow!("SQL completion task ended unexpectedly: {error}"))
            })
        };
        #[cfg(test)]
        let load =
            cx.background_spawn(
                async move { Ok::<_, anyhow::Error>(service.complete(request).await) },
            );

        cx.spawn(async move |_, _| {
            let items = load.await?;
            let is_current = current_state
                .read()
                .expect("SQL completion state poisoned")
                .epoch
                == state.epoch;
            if !is_current {
                return Ok(Vec::new());
            }
            let replace_range = replace_start..buffer_position;
            let completions = items
                .into_iter()
                .map(|item| to_zed_completion(item, replace_range.clone(), replace_start))
                .collect();
            Ok(vec![CompletionResponse {
                completions,
                display_options: CompletionDisplayOptions::default(),
                is_incomplete: false,
            }])
        })
    }

    fn is_completion_trigger(
        &self,
        _: &Entity<Buffer>,
        _: Anchor,
        text: &str,
        _: bool,
        _: &mut Context<Editor>,
    ) -> bool {
        text.chars()
            .last()
            .is_some_and(|character| character == '.' || is_identifier_character(character))
    }
}

fn to_zed_completion(
    item: QueryCompletion,
    replace_range: std::ops::Range<Anchor>,
    match_start: Anchor,
) -> Completion {
    let display = format!("{}    {}", item.label, item.detail);
    let label = CodeLabel::filtered(display, item.label.len(), Some(&item.label), Vec::new());
    Completion {
        replace_range,
        new_text: item.new_text,
        label,
        documentation: None,
        source: CompletionSource::Custom,
        icon_path: None,
        icon_color: None,
        match_start: Some(match_start),
        snippet_deduplication_key: None,
        insert_text_mode: None,
        confirm: None,
        group: None,
    }
}

fn is_identifier_character(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '$' | '@')
}

#[cfg(test)]
mod tests {
    use assets::Assets;
    use gpui::{EntityInputHandler as _, Focusable as _, TestAppContext};

    use super::*;
    use crate::{
        application::Application,
        connection_repository::SharedConnectionRepository,
        credential_vault::test_support::MemoryCredentialVault,
        db::DbType,
        ui::{bind_editor_keys, sql_language},
    };

    #[gpui::test]
    fn completion_keyboard_flow_filters_accepts_and_dismisses(cx: &mut TestAppContext) {
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
                editor.replace_text_in_range(None, "SEL", window, cx);
            })
            .expect("editor window");

        cx.simulate_keystrokes(window.into(), "ctrl-space");
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "enter");
        cx.run_until_parked();
        assert_eq!(editor.read_with(cx, |editor, cx| editor.text(cx)), "SELECT");

        window
            .update(cx, |editor, window, cx| {
                editor.replace_text_in_range(None, " FRO", window, cx);
            })
            .expect("editor window");
        cx.simulate_keystrokes(window.into(), "ctrl-space");
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

        cx.simulate_keystrokes(window.into(), "ctrl-space");
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "tab");
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor.text(cx)),
            "SELECT FROM"
        );
    }
}
