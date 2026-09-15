use super::localization::text;
use crate::platform::UiLanguage;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{EditorState, Input, InputState, TextDecoration, TextDecorationCollection},
    v_flex, ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
};
use gpui_kit::{prelude::*, *};
gpui_kit::actions!(
    astesia_search,
    [
        CloseSearch,
        ToggleReplace,
        NextMatch,
        PreviousMatch,
        ReplaceOne,
        ReplaceAll,
        SwitchField
    ]
);
pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CloseSearch, Some("AstesiaSearch")),
        KeyBinding::new("cmd-shift-h", ToggleReplace, Some("AstesiaSearch")),
        KeyBinding::new("ctrl-h", ToggleReplace, Some("AstesiaSearch")),
        KeyBinding::new("enter", NextMatch, Some("AstesiaSearch")),
        KeyBinding::new("shift-enter", PreviousMatch, Some("AstesiaSearch")),
        KeyBinding::new("enter", ReplaceOne, Some("AstesiaReplaceInput")),
        KeyBinding::new(
            "cmd-enter",
            ReplaceAll,
            Some("QueryItem > QueryEditor > AstesiaSearch > Input"),
        ),
        KeyBinding::new(
            "ctrl-enter",
            ReplaceAll,
            Some("QueryItem > QueryEditor > AstesiaSearch > Input"),
        ),
        KeyBinding::new("tab", SwitchField, Some("AstesiaSearch")),
        KeyBinding::new("shift-tab", SwitchField, Some("AstesiaSearch")),
    ]);
}
pub(super) struct SearchBar {
    editor: Entity<EditorState>,
    query: Entity<InputState>,
    replacement: Entity<InputState>,
    last_query: SharedString,
    last_source: SharedString,
    open: bool,
    replace_mode: bool,
    case_insensitive: bool,
    is_sql: bool,
    highlights: TextDecorationCollection,
    _subscriptions: Vec<Subscription>,
}
impl SearchBar {
    pub(super) fn new(
        editor: Entity<EditorState>,
        is_sql: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx));
        let replacement = cx.new(|cx| InputState::new(window, cx));
        let highlights = editor.update(cx, |editor, cx| {
            editor.create_decorations_collection(vec![], cx)
        });
        let subscriptions = vec![
            cx.observe_in(&query, window, |bar, query, window, cx| {
                if query.update(cx, |input, cx| {
                    input.marked_text_range(window, cx).is_some()
                }) {
                    return;
                }
                let value = query.read(cx).value();
                if bar.last_query != value {
                    bar.last_query = value;
                    bar.refresh(cx);
                }
            }),
            cx.observe(&editor, |bar, editor, cx| {
                let value = editor.read(cx).value();
                if bar.last_source != value {
                    bar.last_source = value;
                    if bar.open {
                        bar.refresh(cx);
                    }
                }
            }),
        ];
        Self {
            last_query: query.read(cx).value(),
            last_source: editor.read(cx).value(),
            editor,
            query,
            replacement,
            open: false,
            replace_mode: false,
            case_insensitive: true,
            is_sql,
            highlights,
            _subscriptions: subscriptions,
        }
    }
    #[cfg(test)]
    pub(super) fn is_open(&self) -> bool {
        self.open
    }
    #[cfg(test)]
    pub(super) fn query(&self, cx: &App) -> String {
        self.query.read(cx).value().to_string()
    }
    #[cfg(test)]
    pub(super) fn replacement(&self, cx: &App) -> String {
        self.replacement.read(cx).value().to_string()
    }
    pub(super) fn show(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let was_open = self.open;
        self.open = true;
        if replace && was_open {
            self.toggle_replace(&ToggleReplace, window, cx);
            return;
        }
        self.replace_mode = replace;
        let selection = self.editor.read(cx).selected_value();
        self.query.update(cx, |query, cx| {
            if !selection.is_empty() {
                query.set_value(selection, window, cx);
            }
            query.select_all(window, cx);
        });
        window.focus(&self.query.focus_handle(cx), cx);
        self.refresh(cx);
    }
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        self.editor.update(cx, |editor, cx| {
            editor.set_search_query(query, self.case_insensitive, cx)
        });
        self.update_highlights(cx);
    }
    fn update_highlights(&mut self, cx: &mut Context<Self>) {
        let current = self
            .editor
            .read(cx)
            .search_session()
            .matcher
            .current_match_index();
        let ranges = self
            .editor
            .read(cx)
            .search_session()
            .matcher
            .matched_ranges();
        let style = HighlightStyle {
            background_color: Some(cx.theme().warning.opacity(0.16)),
            ..Default::default()
        };
        self.highlights.set(
            if self.open {
                ranges
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(index, range)| {
                        TextDecoration::new(
                            range,
                            if index == current {
                                HighlightStyle {
                                    background_color: Some(cx.theme().warning.opacity(0.42)),
                                    ..Default::default()
                                }
                            } else {
                                style
                            },
                        )
                    })
                    .collect()
            } else {
                vec![]
            },
            cx,
        );
        cx.notify();
    }
    fn close(&mut self, _: &CloseSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.highlights.clear(cx);
        window.focus(&self.editor.focus_handle(cx), cx);
        cx.notify();
    }
    fn toggle_replace(&mut self, _: &ToggleReplace, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editor.read(cx).is_replaceable() {
            return;
        }
        self.replace_mode = !self.replace_mode;
        window.focus(
            &if self.replace_mode {
                self.replacement.focus_handle(cx)
            } else {
                self.query.focus_handle(cx)
            },
            cx,
        );
        cx.notify();
    }
    fn switch_field(&mut self, _: &SwitchField, window: &mut Window, cx: &mut Context<Self>) {
        if !self.replace_mode {
            self.close(&CloseSearch, window, cx);
            return;
        }
        let next = if self.query.focus_handle(cx).is_focused(window) {
            self.replacement.focus_handle(cx)
        } else {
            self.query.focus_handle(cx)
        };
        window.focus(&next, cx);
    }
    fn next(&mut self, _: &NextMatch, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .query
            .update(cx, |q, cx| q.marked_text_range(window, cx).is_some())
        {
            return;
        }
        self.navigate(false, cx);
    }
    fn previous(&mut self, _: &PreviousMatch, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .query
            .update(cx, |q, cx| q.marked_text_range(window, cx).is_some())
        {
            return;
        }
        self.navigate(true, cx);
    }
    pub(super) fn navigate(&mut self, previous: bool, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let range = if previous {
                editor.previous_search_match(cx)
            } else {
                editor.next_search_match(cx)
            };
            if let Some(range) = range {
                editor.set_selected_range(range, cx);
            }
        });
        self.update_highlights(cx);
    }
    fn replace_one(&mut self, _: &ReplaceOne, window: &mut Window, cx: &mut Context<Self>) {
        self.replace(false, window, cx);
    }
    fn replace_all(&mut self, _: &ReplaceAll, window: &mut Window, cx: &mut Context<Self>) {
        self.replace(true, window, cx);
    }
    fn replace(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.replace_mode {
            return;
        }
        let replacement = self.replacement.read(cx).value().to_string();
        self.editor.update(cx, |editor, cx| {
            if all {
                editor.replace_all_search_matches(&replacement, window, cx);
            } else {
                editor.replace_current_search_match(&replacement, window, cx);
            }
        });
        self.refresh(cx);
    }
}
impl Render for SearchBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let language = cx
            .try_global::<super::shell::UiLocale>()
            .map_or(UiLanguage::Chinese, |locale| locale.0);
        let matcher = &self.editor.read(cx).search_session().matcher;
        let no_matches = matcher.is_empty();
        let label = if self.query.read(cx).value().is_empty() {
            String::new()
        } else if no_matches {
            text(language, "无匹配", "No matches").to_owned()
        } else {
            matcher.label()
        };
        let can_replace = self.editor.read(cx).is_replaceable();
        v_flex()
            .key_context("AstesiaSearch")
            .capture_key_down(cx.listener(|bar, event: &KeyDownEvent, window, cx| {
                let modifiers = event.keystroke.modifiers;
                if event.keystroke.key == "enter"
                    && (modifiers.platform || modifiers.control)
                    && !modifiers.alt
                    && !modifiers.shift
                {
                    bar.replace(true, window, cx);
                    cx.stop_propagation();
                }
            }))
            .px_3()
            .py(px(6.0))
            .bg(cx.theme().sidebar)
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::toggle_replace))
            .on_action(cx.listener(Self::switch_field))
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::previous))
            .on_action(cx.listener(Self::replace_one))
            .on_action(cx.listener(Self::replace_all))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().text_size(px(11.0)).child(if self.is_sql {
                        text(language, "当前 SQL", "Current SQL")
                    } else {
                        text(language, "当前文档", "Current document")
                    }))
                    .child(
                        Input::new(&self.query)
                            .prefix(
                                gpui_kit::component::Icon::new(
                                    gpui_kit::component::IconName::Search,
                                )
                                .size(px(14.0)),
                            )
                            .suffix(div().text_size(px(11.0)).child(label))
                            .small()
                            .w(px(240.))
                            .aria_label(text(language, "查找", "Find")),
                    )
                    .child(
                        Button::new("search-previous")
                            .icon(gpui_kit::component::IconName::ChevronUp)
                            .disabled(no_matches)
                            .small()
                            .ghost()
                            .accessibility_label(text(language, "上一个匹配", "Previous match"))
                            .on_click(
                                cx.listener(|bar, _, w, cx| bar.previous(&PreviousMatch, w, cx)),
                            ),
                    )
                    .child(
                        Button::new("search-next")
                            .icon(gpui_kit::component::IconName::ChevronDown)
                            .disabled(no_matches)
                            .small()
                            .ghost()
                            .accessibility_label(text(language, "下一个匹配", "Next match"))
                            .on_click(cx.listener(|bar, _, w, cx| bar.next(&NextMatch, w, cx))),
                    )
                    .child(
                        Button::new("search-case")
                            .label("Aa")
                            .small()
                            .ghost()
                            .selected(!self.case_insensitive)
                            .accessibility_label(text(language, "区分大小写", "Match case"))
                            .on_click(cx.listener(|bar, _, _, cx| {
                                bar.case_insensitive = !bar.case_insensitive;
                                bar.refresh(cx);
                            })),
                    )
                    .child(
                        Button::new("search-replace")
                            .label(text(language, "替换", "Replace"))
                            .small()
                            .ghost()
                            .disabled(!can_replace)
                            .on_click(cx.listener(|bar, _, w, cx| {
                                bar.toggle_replace(&ToggleReplace, w, cx)
                            })),
                    )
                    .child(
                        Button::new("search-close")
                            .icon(gpui_kit::component::IconName::Close)
                            .small()
                            .ghost()
                            .accessibility_label(text(language, "关闭查找", "Close search"))
                            .on_click(cx.listener(|bar, _, w, cx| bar.close(&CloseSearch, w, cx))),
                    ),
            )
            .when(self.replace_mode, |el| {
                el.child(
                    h_flex()
                        .gap_1()
                        .child(div().key_context("AstesiaReplaceInput").child(
                            Input::new(&self.replacement).w(px(200.)).aria_label(text(
                                language,
                                "替换为",
                                "Replace with",
                            )),
                        ))
                        .child(
                            Button::new("replace-one")
                                .label(text(language, "替换", "Replace"))
                                .small()
                                .ghost()
                                .disabled(!can_replace)
                                .on_click(
                                    cx.listener(|bar, _, w, cx| {
                                        bar.replace_one(&ReplaceOne, w, cx)
                                    }),
                                ),
                        )
                        .child(
                            Button::new("replace-all")
                                .label(text(language, "全部替换", "Replace all"))
                                .small()
                                .ghost()
                                .disabled(!can_replace)
                                .on_click(
                                    cx.listener(|bar, _, w, cx| {
                                        bar.replace_all(&ReplaceAll, w, cx)
                                    }),
                                ),
                        ),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui_kit::test]
    fn programmatic_values_refresh_search(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx);
        });
        let window = cx.add_window(|window, cx| {
            let editor = cx.new(|cx| EditorState::new(window, cx).default_value("name name"));
            SearchBar::new(editor, false, window, cx)
        });
        let bar = window.root(cx).unwrap();
        window
            .update(cx, |bar, window, cx| {
                bar.show(true, window, cx);
                bar.query
                    .update(cx, |query, cx| query.set_value("name", window, cx));
                bar.replacement.update(cx, |replacement, cx| {
                    replacement.set_value("id", window, cx)
                });
            })
            .unwrap();
        cx.run_until_parked();
        bar.read_with(cx, |bar, cx| {
            assert_eq!(
                bar.editor
                    .read(cx)
                    .search_session()
                    .matcher
                    .matched_ranges()
                    .len(),
                2
            )
        });
        window
            .update(cx, |bar, window, cx| bar.replace(true, window, cx))
            .unwrap();
        cx.run_until_parked();
        bar.read_with(cx, |bar, cx| {
            assert_eq!(bar.editor.read(cx).value().as_ref(), "id id");
            assert!(bar
                .editor
                .read(cx)
                .search_session()
                .matcher
                .matched_ranges()
                .is_empty());
        });
    }
}
