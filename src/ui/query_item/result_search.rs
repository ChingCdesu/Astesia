use super::*;
use gpui_kit::component::{
    input::{Input, InputState},
    Sizable as _,
};
use std::ops::Range;

#[cfg(test)]
#[path = "result_search_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq)]
struct ResultMatch {
    row: usize,
    column: usize,
    range: Range<usize>,
}

pub(super) struct ResultSearch {
    input: Entity<InputState>,
    open: bool,
    case_sensitive: bool,
    query: String,
    source: Option<std::sync::Weak<StatementResult>>,
    matches: Vec<ResultMatch>,
    current: usize,
    pub(super) rows: UniformListScrollHandle,
    pub(super) horizontal: ScrollHandle,
}

impl ResultSearch {
    pub(super) fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            input: cx.new(|cx| InputState::new(window, cx)),
            open: false,
            case_sensitive: false,
            query: String::new(),
            source: None,
            matches: vec![],
            current: 0,
            rows: UniformListScrollHandle::new(),
            horizontal: ScrollHandle::new(),
        }
    }

    pub(super) fn observe(&self, window: &mut Window, cx: &mut Context<QueryItem>) -> Subscription {
        cx.observe_in(&self.input, window, |item, input, window, cx| {
            if input.update(cx, |input, cx| {
                input.marked_text_range(window, cx).is_some()
            }) {
                return;
            }
            if item.result_search.query != input.read(cx).value().as_ref() {
                item.refresh_result_search(cx);
                cx.notify();
            }
        })
    }

    fn active(&self) -> Option<&ResultMatch> {
        if !self.open {
            return None;
        }
        self.matches.get(self.current)
    }

    fn reveal(&self) {
        let Some(active) = self.active() else { return };
        self.rows.scroll_to_item(active.row, ScrollStrategy::Center);
        let offset = self.horizontal.offset();
        let width = self.horizontal.bounds().size.width;
        let start = px(48.0 + active.column as f32 * 180.0);
        let end = start + px(180.0);
        let x = if start < -offset.x {
            -start
        } else if end > -offset.x + width {
            width - end
        } else {
            offset.x
        };
        self.horizontal.set_offset(point(x.min(px(0.0)), offset.y));
    }
}

fn find_matches(result: &StatementResult, query: &str, case_sensitive: bool) -> Vec<ResultMatch> {
    if query.is_empty() {
        return vec![];
    }
    let needle = if case_sensitive {
        query.to_owned()
    } else {
        query.to_ascii_lowercase()
    };
    let mut matches = Vec::new();
    for (row, values) in result.rows.iter().enumerate() {
        for (column, value) in values.iter().take(result.columns.len()).enumerate() {
            let value = display_value(value);
            let haystack = if case_sensitive {
                value
            } else {
                value.to_ascii_lowercase()
            };
            matches.extend(
                haystack
                    .match_indices(&needle)
                    .map(|(start, matched)| ResultMatch {
                        row,
                        column,
                        range: start..start + matched.len(),
                    }),
            );
        }
    }
    matches
}

impl QueryItem {
    pub(super) fn refresh_result_search(&mut self, cx: &mut Context<Self>) {
        let source = self
            .state
            .shared_active_result()
            .filter(|r| r.success && !self.state.is_running());
        let query = self.result_search.input.read(cx).value().to_string();
        let search = &mut self.result_search;
        let same_source = match (&source, &search.source) {
            (Some(a), Some(b)) => std::sync::Weak::ptr_eq(&Arc::downgrade(a), b),
            (None, None) => true,
            _ => false,
        };
        if same_source && query == search.query {
            return;
        }
        search.source = source.as_ref().map(Arc::downgrade);
        search.query = query;
        search.matches = source
            .as_ref()
            .map(|r| find_matches(r, &search.query, search.case_sensitive))
            .unwrap_or_default();
        search.current = 0;
        search.reveal();
    }

    pub(super) fn open_result_search(
        &mut self,
        _: &FindQueryResults,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.is_running()
            || self
                .state
                .active_result()
                .is_none_or(|r| !r.success || r.columns.is_empty())
        {
            return;
        }
        self.showing_chart = false;
        self.result_search.open = true;
        self.refresh_result_search(cx);
        self.result_search
            .input
            .update(cx, |input, cx| input.select_all(window, cx));
        window.focus(&self.result_search.input.focus_handle(cx), cx);
        self.result_search.reveal();
        cx.notify();
    }

    fn close_result_search(
        &mut self,
        _: &CloseResultSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.result_search.open = false;
        window.focus(&self.result_focus, cx);
        cx.notify();
    }

    fn next_result_match(
        &mut self,
        _: &NextResultMatch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_result_match(false, window, cx);
    }
    fn previous_result_match(
        &mut self,
        _: &PreviousResultMatch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_result_match(true, window, cx);
    }
    fn move_result_match(&mut self, previous: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.result_search.input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        }) {
            return;
        }
        let search = &mut self.result_search;
        let len = search.matches.len();
        if len > 0 {
            search.current = (search.current + if previous { len - 1 } else { 1 }) % len;
            search.reveal();
            cx.notify();
        }
    }

    pub(super) fn render_result_search(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.result_search.open || self.showing_chart {
            return None;
        }
        let language = self.settings.read(cx).language();
        let search = &self.result_search;
        let count = if search.query.is_empty() {
            String::new()
        } else if search.matches.is_empty() {
            text(language, "无匹配", "No matches").to_owned()
        } else {
            format!("{} / {}", search.current + 1, search.matches.len())
        };
        Some(
            h_flex()
                .key_context("QueryResultSearch")
                .h(px(40.0))
                .flex_none()
                .gap_2()
                .px_3()
                .bg(cx.theme().colors().surface_background)
                .border_b_1()
                .border_color(cx.theme().colors().border)
                .on_action(cx.listener(Self::open_result_search))
                .on_action(cx.listener(Self::close_result_search))
                .on_action(cx.listener(Self::next_result_match))
                .on_action(cx.listener(Self::previous_result_match))
                .child(
                    Label::new(text(language, "当前结果", "Current result"))
                        .size(LabelSize::XSmall),
                )
                .child(
                    Input::new(&search.input)
                        .prefix(
                            gpui_kit::component::Icon::new(gpui_kit::component::IconName::Search)
                                .size(px(14.0)),
                        )
                        .suffix(div().text_size(px(11.0)).child(count))
                        .small()
                        .w(px(240.0))
                        .aria_label(text(language, "在当前结果中查找", "Find in current result")),
                )
                .child(
                    Button::new("result-find-case", "Aa")
                        .size(ButtonSize::Compact)
                        .toggle_state(search.case_sensitive)
                        .tooltip(Tooltip::text(text(language, "区分大小写", "Match case")))
                        .on_click(cx.listener(|item, _, _, cx| {
                            item.result_search.case_sensitive = !item.result_search.case_sensitive;
                            item.result_search.source = None;
                            item.refresh_result_search(cx);
                            cx.notify();
                        })),
                )
                .child(
                    query_toolbar_action(
                        "result-find-previous",
                        IconName::ChevronUp,
                        text(
                            language,
                            "上一个匹配 · Shift+Enter",
                            "Previous match · Shift+Enter",
                        ),
                    )
                    .disabled(search.matches.is_empty())
                    .on_click(cx.listener(|item, _, w, cx| {
                        item.previous_result_match(&PreviousResultMatch, w, cx)
                    })),
                )
                .child(
                    query_toolbar_action(
                        "result-find-next",
                        IconName::ChevronDown,
                        text(language, "下一个匹配 · Enter", "Next match · Enter"),
                    )
                    .disabled(search.matches.is_empty())
                    .on_click(
                        cx.listener(|item, _, w, cx| {
                            item.next_result_match(&NextResultMatch, w, cx)
                        }),
                    ),
                )
                .child(
                    query_toolbar_action(
                        "result-find-close",
                        IconName::Close,
                        text(language, "关闭查找 · Esc", "Close search · Esc"),
                    )
                    .on_click(cx.listener(|item, _, w, cx| {
                        item.close_result_search(&CloseResultSearch, w, cx)
                    })),
                )
                .child(
                    div().flex_1().min_w_0().overflow_hidden().child(
                        Label::new(format!(
                            "{} {} {}",
                            text(language, "已加载", "Loaded"),
                            search
                                .source
                                .as_ref()
                                .and_then(std::sync::Weak::upgrade)
                                .map_or(0, |r| r.rows.len()),
                            text(language, "行", "rows")
                        ))
                        .size(LabelSize::XSmall),
                    ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn result_cell_text(
        &self,
        row: usize,
        column: usize,
        value: String,
        cx: &App,
    ) -> AnyElement {
        let search = &self.result_search;
        let highlights = if search.open {
            let start = search
                .matches
                .partition_point(|m| (m.row, m.column) < (row, column));
            search.matches[start..]
                .iter()
                .take_while(|m| (m.row, m.column) == (row, column))
                .map(|m| {
                    let active = search.active() == Some(m);
                    (
                        m.range.clone(),
                        HighlightStyle {
                            background_color: Some(if active {
                                cx.theme().warning.opacity(0.42)
                            } else {
                                cx.theme().colors().ghost_element_selected
                            }),
                            ..Default::default()
                        },
                    )
                })
                .collect::<Vec<_>>()
        } else {
            vec![]
        };
        div()
            .text_size(px(11.0))
            .overflow_hidden()
            .child(StyledText::new(value).with_highlights(highlights))
            .into_any_element()
    }

    pub(super) fn is_current_result_match(&self, row: usize, column: usize) -> bool {
        self.result_search
            .active()
            .is_some_and(|m| m.row == row && m.column == column)
    }
}
