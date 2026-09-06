pub use gpui_kit::component::input::InputEvent as EditorEvent;
use gpui_kit::component::input::{
    Editor as CodeEditor, EditorState, Input, InputEvent, InputState,
};
use gpui_kit::{prelude::*, *};

enum State {
    Code(Entity<EditorState>),
    Line(Entity<InputState>),
}
pub(super) struct Editor {
    search: Option<Entity<super::editor_search::SearchBar>>,
    state: State,
    readonly: bool,
    inline_style: Option<(&'static str, Pixels)>,
    last_value: SharedString,
    _observation: Subscription,
    _subscription: Subscription,
}
impl Editor {
    pub(super) fn code(
        text: &str,
        language: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language.to_owned())
                .line_number(true)
                .folding(true)
                .default_value(text.to_owned())
        });
        let subscription = cx.subscribe(&state, |_, _, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                cx.emit(event.clone());
            }
            cx.notify();
        });
        let search = cx.new(|cx| super::editor_search::SearchBar::new(state.clone(), window, cx));
        let last_value = state.read(cx).value();
        let observation = cx.observe_in(&state, window, |this, state, window, cx| {
            let value = state.read(cx).value();
            if this.last_value != value {
                let refresh_completion = value.len() < this.last_value.len()
                    && state.read(cx).lsp().completion_provider.is_some()
                    && state.read(cx).focus_handle(cx).is_focused(window);
                this.last_value = value;
                cx.emit(InputEvent::Change);
                cx.notify();
                if refresh_completion {
                    let refresh = state.update(cx, |editor, cx| {
                        if editor.marked_text_range(window, cx).is_some() {
                            return false;
                        }
                        let cursor = editor.cursor();
                        let source = editor.text().to_string();
                        let refresh = source[..cursor].chars().last().is_some_and(|c| {
                            c.is_alphanumeric() || matches!(c, '_' | '$' | '@' | '.')
                        });
                        if !refresh {
                            editor.present_completion_items(cursor, "", Vec::new(), cx);
                        }
                        refresh
                    });
                    if refresh {
                        this.show_completions(&ShowCompletions, window, cx);
                    }
                }
            }
        });
        Self {
            search: Some(search),
            state: State::Code(state),
            readonly: false,
            inline_style: None,
            last_value,
            _observation: observation,
            _subscription: subscription,
        }
    }
    pub(super) fn single_line(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe(&state, |_, _, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                cx.emit(event.clone());
            }
            cx.notify();
        });
        let last_value = state.read(cx).value();
        let observation = cx.observe(&state, |this, state, cx| {
            let value = state.read(cx).value();
            if this.last_value != value {
                this.last_value = value;
                cx.emit(InputEvent::Change);
                cx.notify();
            }
        });
        Self {
            search: None,
            state: State::Line(state),
            readonly: false,
            inline_style: None,
            last_value,
            _observation: observation,
            _subscription: subscription,
        }
    }
    pub(super) fn inline_single_line(
        label: &'static str,
        font_size: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut editor = Self::single_line(window, cx);
        editor.inline_style = Some((label, font_size));
        editor
    }
    #[cfg(test)]
    pub(super) fn input_bounds(&self, cx: &App) -> Bounds<Pixels> {
        match &self.state {
            State::Code(state) => state.read(cx).input_bounds(),
            State::Line(state) => state.read(cx).input_bounds(),
        }
    }
    pub(super) fn code_state(&self) -> Option<&Entity<EditorState>> {
        match &self.state {
            State::Code(s) => Some(s),
            _ => None,
        }
    }
    pub(super) fn text(&self, cx: &App) -> String {
        match &self.state {
            State::Code(s) => s.read(cx).value().to_string(),
            State::Line(s) => s.read(cx).value().to_string(),
        }
    }
    pub(super) fn set_text(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = text.into();
        match &self.state {
            State::Code(s) => s.update(cx, |s, cx| s.set_value(text, window, cx)),
            State::Line(s) => s.update(cx, |s, cx| s.set_value(text, window, cx)),
        };
        cx.notify();
    }
    pub(super) fn selected_range(&self, cx: &App) -> std::ops::Range<usize> {
        match &self.state {
            State::Code(s) => s.read(cx).selected_range(),
            State::Line(s) => s.read(cx).selected_range(),
        }
    }
    pub(super) fn set_placeholder_text(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = text.into();
        match &self.state {
            State::Code(s) => s.update(cx, |s, cx| s.set_placeholder(text, window, cx)),
            State::Line(s) => s.update(cx, |s, cx| s.set_placeholder(text, window, cx)),
        };
    }
    pub(super) fn set_read_only(&mut self, readonly: bool) {
        self.readonly = readonly;
    }
    pub(super) fn open_search(
        &mut self,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(search) = self.search.clone() {
            search.update(cx, |search, cx| search.show(replace, window, cx));
        }
        cx.notify();
    }
    #[cfg(test)]
    pub(super) fn search_bar(&self) -> Option<&Entity<super::editor_search::SearchBar>> {
        self.search.as_ref()
    }
    fn show_find(
        &mut self,
        _: &gpui_kit::component::input::Search,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_search(false, window, cx);
        cx.stop_propagation();
    }
    fn show_replace(
        &mut self,
        _: &gpui_kit::component::input::Replace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_search(true, window, cx);
        cx.stop_propagation();
    }
}
impl EventEmitter<InputEvent> for Editor {}
impl Focusable for Editor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.state {
            State::Code(s) => s.focus_handle(cx),
            State::Line(s) => s.focus_handle(cx),
        }
    }
}
impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language = cx
            .try_global::<super::shell::UiLocale>()
            .map_or(crate::platform::UiLanguage::Chinese, |locale| locale.0);
        match &self.state {
            State::Code(s) => gpui_kit::component::v_flex()
                .size_full()
                .capture_action(cx.listener(Self::show_find))
                .capture_action(cx.listener(Self::show_replace))
                .capture_action(cx.listener(Self::confirm_completion_enter))
                .capture_action(cx.listener(Self::confirm_completion_tab))
                .on_action(cx.listener(Self::show_completions))
                .on_action(cx.listener(Self::next_match))
                .on_action(cx.listener(Self::previous_match))
                .children(self.search.clone())
                .child(
                    div().flex_1().min_h_0().child(
                        CodeEditor::new(s)
                            .aria_label(super::localization::text(
                                language,
                                "代码编辑器",
                                "Code editor",
                            ))
                            .readonly(self.readonly)
                            .bordered(false)
                            .rounded_none()
                            .p_0()
                            .text_size(px(14.0))
                            .line_height(px(20.0))
                            .size_full(),
                    ),
                )
                .into_any_element(),
            State::Line(s) => Input::new(s)
                .readonly(self.readonly)
                .w_full()
                .when_some(self.inline_style, |input, (label, font_size)| {
                    input
                        .aria_label(label)
                        .appearance(false)
                        .bordered(false)
                        .h_full()
                        .px_0()
                        .py_0()
                        .text_size(font_size)
                })
                .into_any_element(),
        }
    }
}

gpui_kit::actions!(astesia_editor, [ShowCompletions, NextMatch, PreviousMatch]);
pub(super) fn bind_keys(cx: &mut App) {
    use gpui_kit::component::input::Replace;
    cx.bind_keys([
        KeyBinding::new("ctrl-space", ShowCompletions, Some("QueryEditor > Input")),
        KeyBinding::new("cmd-alt-f", Replace, Some("QueryEditor > Input")),
        KeyBinding::new("ctrl-h", Replace, Some("QueryEditor > Input")),
        KeyBinding::new("cmd-g", NextMatch, Some("QueryEditor")),
        KeyBinding::new("cmd-shift-g", PreviousMatch, Some("QueryEditor")),
    ]);
}
impl Editor {
    pub(super) fn show_completions(
        &mut self,
        _: &ShowCompletions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.code_state().cloned() else {
            return;
        };
        let state = editor.read(cx);
        let Some(provider) = state.lsp().completion_provider.clone() else {
            return;
        };
        if !state.focus_handle(cx).is_focused(window) {
            return;
        }
        let text = state.text().clone();
        let offset = state.cursor();
        let selection = state.selected_range();
        let source = text.to_string();
        let prefix_length = source[..offset]
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '@'))
            .map(char::len_utf8)
            .sum::<usize>();
        let query = source[offset - prefix_length..offset].to_owned();
        let request = provider.completions(
            &text,
            offset,
            lsp_types::CompletionContext {
                trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
                trigger_character: None,
            },
            window,
            cx,
        );
        cx.spawn_in(window, async move |_, cx| {
            let Ok(response) = request.await else {
                return;
            };
            let items = match response {
                lsp_types::CompletionResponse::Array(items) => items,
                lsp_types::CompletionResponse::List(list) => list.items,
            };
            editor
                .update_in(cx, |editor, window, cx| {
                    if editor.text() != &text
                        || editor.selected_range() != selection
                        || !editor.focus_handle(cx).is_focused(window)
                    {
                        return;
                    }
                    editor.present_completion_items(offset - prefix_length, query, items, cx);
                })
                .ok();
        })
        .detach();
    }
    fn next_match(&mut self, _: &NextMatch, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.search.clone() {
            search.update(cx, |search, cx| search.navigate(false, cx));
        }
    }
    fn previous_match(&mut self, _: &PreviousMatch, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.search.clone() {
            search.update(cx, |search, cx| search.navigate(true, cx));
        }
    }
}

impl Editor {
    fn confirm_completion_enter(
        &mut self,
        action: &gpui_kit::component::input::Enter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !action.secondary {
            self.route_completion_action(Box::new(action.clone()), window, cx);
        }
    }
    fn confirm_completion_tab(
        &mut self,
        _: &gpui_kit::component::input::IndentInline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.route_completion_action(
            Box::new(gpui_kit::component::input::Enter {
                secondary: false,
                shift: false,
            }),
            window,
            cx,
        );
    }
    fn route_completion_action(
        &self,
        action: Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.code_state().cloned() else {
            return;
        };
        let handled = editor.update(cx, |editor, cx| {
            editor.focus_handle(cx).is_focused(window)
                && editor.completion_menu_state().open
                && editor.route_overlay_action(action, window, cx)
        });
        // Kit's completion presenter propagates its action; confirmation must not also insert text.
        if handled {
            cx.stop_propagation();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use gpui_kit::TestAppContext;
    #[gpui_kit::test]
    fn chinese_composition_is_one_undo_operation(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx)
        });
        let window = cx.add_window(|window, cx| Editor::code("", "sql", window, cx));
        let wrapper = window.root(cx).unwrap();
        let editor = wrapper.read_with(cx, |editor, _| editor.code_state().unwrap().clone());
        window
            .update(cx, |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    window.focus(&editor.focus_handle(cx), cx);
                    editor.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                    editor.replace_and_mark_text_in_range(None, "nihao", Some(5..5), window, cx);
                    assert_eq!(editor.marked_text_range(window, cx), Some(0..5));
                    editor.replace_text_in_range(None, "你好", window, cx);
                    assert_eq!(editor.value().as_ref(), "你好");
                    assert_eq!(editor.marked_text_range(window, cx), None);
                });
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "cmd-z");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.value()).as_ref(),
            ""
        );
        cx.simulate_keystrokes(window.into(), "cmd-shift-z");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.value()).as_ref(),
            "你好"
        );
    }
    #[gpui_kit::test]
    async fn native_runtime_can_request_an_in_place_restart(cx: &mut TestAppContext) {
        let restart = cx.expect_restart();
        cx.update(|cx| cx.restart());
        let (path, arguments) = restart.await.expect("restart request");
        assert_eq!(path, None);
        assert!(arguments.is_empty());
    }
}
