use std::{collections::HashSet, sync::Arc};

use crate::ui::components::prelude::*;
use crate::ui::input_field::InputField;
use gpui_kit::{ClickEvent, Entity, FocusHandle, Subscription};
use serde_json::Value;

use crate::application::{Application, DocumentSession, DocumentSessionStatus, QueryTarget};
use crate::db::{DocumentPage, TableRef};

use super::{localization::text, shell::ShellSettings};

const DOCUMENT_PAGE_SIZE: u32 = 50;

pub(super) struct DocumentItem {
    application: Arc<Application>,
    session: DocumentSession,
    filter: Entity<InputField>,
    collapsed_documents: HashSet<usize>,
    focus_handle: FocusHandle,
    settings: Entity<ShellSettings>,
    _filter_observation: Subscription,
    _settings_observation: Subscription,
}

impl DocumentItem {
    pub(super) fn new(
        application: Arc<Application>,
        target: QueryTarget,
        collection: TableRef,
        settings: Entity<ShellSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let language = settings.read(cx).language();
        let filter = cx.new(|cx| {
            InputField::new(
                window,
                cx,
                text(
                    language,
                    "JSON 筛选条件，例如 {\"status\":\"open\"}",
                    "JSON filter, for example {\"status\":\"open\"}",
                ),
            )
            .label(text(
                language,
                "MongoDB JSON 筛选条件",
                "MongoDB JSON filter",
            ))
        });
        let filter_observation = cx.observe(&filter, |_, _, cx| cx.notify());
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let session = DocumentSession::new(target, collection, DOCUMENT_PAGE_SIZE)
            .expect("MongoDB collection events must create document sessions");
        let mut item = Self {
            application,
            session,
            filter,
            collapsed_documents: HashSet::new(),
            focus_handle: cx.focus_handle(),
            settings,
            _filter_observation: filter_observation,
            _settings_observation: settings_observation,
        };
        item.load(cx);
        item
    }

    pub(super) fn label(&self) -> String {
        format!(
            "{} · {}/{}",
            self.session.collection(),
            self.session.target().connection_name,
            self.session.target().database
        )
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    pub(super) fn invalidate_session(
        &mut self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<Self>,
    ) {
        let language = self.settings.read(cx).language();
        if self.session.invalidate_session(
            connection_id,
            session_generation,
            text(
                language,
                "连接会话已更改。请从侧边栏重新打开集合。",
                "The connection session changed. Reopen the collection from the sidebar.",
            ),
        ) {
            cx.notify();
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Ok(request) = self.session.begin_load() else {
            return;
        };
        cx.notify();
        let application = self.application.clone();
        let failed_request = request.clone();
        let load = crate::ui::runtime::spawn(cx, async move {
            let result = application.documents().load(&request).await;
            (request, result)
        });
        cx.spawn(async move |item, cx| {
            let (request, result) = match load.await {
                Ok(outcome) => outcome,
                Err(error) => (
                    failed_request,
                    Err(format!("MongoDB document task ended unexpectedly: {error}")),
                ),
            };
            item.update(cx, |item, cx| {
                let applied = item.session.finish_load(&request, result);
                if applied {
                    item.collapsed_documents.clear();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn refresh(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.load(cx);
    }

    fn apply_filter(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let filter = self.filter.read(cx).text(cx);
        match self.session.set_filter(filter) {
            Ok(_) => {
                self.filter
                    .update(cx, |field, cx| field.set_error(None::<SharedString>, cx));
                self.load(cx);
            }
            Err(error) => {
                self.filter
                    .update(cx, |field, cx| field.set_error(Some(error), cx));
            }
        }
    }

    fn previous_page(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let page = self.session.query().page;
        if page > 1 && self.session.set_page(page - 1).unwrap_or(false) {
            self.load(cx);
        }
    }

    fn next_page(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let query = self.session.query();
        let can_advance = self.session.page().is_some_and(|page| {
            u64::from(query.page) * u64::from(query.page_size) < page.total_documents
        });
        if can_advance && self.session.set_page(query.page + 1).unwrap_or(false) {
            self.load(cx);
        }
    }

    fn toggle_document(
        &mut self,
        index: usize,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.collapsed_documents.remove(&index) {
            self.collapsed_documents.insert(index);
        }
        cx.notify();
    }

    fn render_documents(&self, page: &DocumentPage, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings.read(cx).language();
        if page.documents.is_empty() {
            return v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .gap_1()
                .child(Label::new(text(
                    language,
                    "没有匹配的文档",
                    "No matching documents",
                )))
                .child(
                    Label::new(text(
                        language,
                        "集合可用，但当前筛选条件和页面没有返回数据。",
                        "The collection is available, but this filter and page returned no data.",
                    ))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                )
                .into_any_element();
        }
        let colors = cx.theme().colors();
        v_flex()
            .id("document-page")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3()
            .gap_2()
            .children(page.documents.iter().enumerate().map(|(index, document)| {
                let collapsed = self.collapsed_documents.contains(&index);
                let type_name = json_type_name(document);
                v_flex()
                    .id(("document-card", index))
                    .flex_none()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        h_flex()
                            .id(("toggle-document", index))
                            .role(gpui_kit::Role::Button)
                            .cursor_pointer()
                            .px_3()
                            .py_2()
                            .gap_2()
                            .on_click(cx.listener(move |item, event, window, cx| {
                                item.toggle_document(index, event, window, cx);
                            }))
                            .child(
                                Icon::new(if collapsed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronDown
                                })
                                .size(IconSize::XSmall),
                            )
                            .child(
                                Label::new(format!(
                                    "{} {}",
                                    text(language, "文档", "Document"),
                                    index + 1
                                ))
                                .size(LabelSize::Small)
                                .weight(gpui_kit::FontWeight::MEDIUM),
                            )
                            .child(div().flex_1())
                            .child(
                                Label::new(type_name)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .when(!collapsed, |element| {
                        element.child(
                            v_flex()
                                .border_t_1()
                                .border_color(colors.border)
                                .p_3()
                                .gap_1()
                                .children(document_rows(document)),
                        )
                    })
            }))
            .into_any_element()
    }
}

impl Render for DocumentItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let language = self.settings.read(cx).language();
        let status = self.session.status();
        let loading = matches!(status, DocumentSessionStatus::Loading);
        let page_number = self.session.query().page;
        let page_size = self.session.query().page_size;
        let total = self.session.page().map(|page| page.total_documents);
        let can_advance =
            total.is_some_and(|total| u64::from(page_number) * u64::from(page_size) < total);
        let content = match status {
            DocumentSessionStatus::Idle | DocumentSessionStatus::Loading
                if self.session.page().is_none() =>
            {
                centered_state(
                    text(language, "正在加载文档…", "Loading documents…"),
                    Color::Muted,
                )
            }
            DocumentSessionStatus::Failed(error) if self.session.page().is_none() => {
                centered_state(error, Color::Error)
            }
            DocumentSessionStatus::Unavailable(reason) => centered_state(reason, Color::Warning),
            _ => self
                .session
                .page()
                .map(|page| self.render_documents(page, cx))
                .unwrap_or_else(|| {
                    centered_state(text(language, "没有文档", "No documents"), Color::Muted)
                }),
        };

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("DocumentItem")
            .size_full()
            .overflow_hidden()
            .child(
                h_flex()
                    .h(px(44.0))
                    .flex_none()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        Label::new(self.label())
                            .size(LabelSize::Small)
                            .weight(gpui_kit::FontWeight::MEDIUM)
                            .truncate()
                            .flex_1(),
                    )
                    .when_some(total, |element, total| {
                        element.child(
                            Label::new(format!("{total} {}", text(language, "条", "documents")))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                    })
                    .child(
                        Button::new("refresh-documents", text(language, "刷新", "Refresh"))
                            .size(ButtonSize::Compact)
                            .loading(loading)
                            .disabled(
                                loading || matches!(status, DocumentSessionStatus::Unavailable(_)),
                            )
                            .on_click(cx.listener(Self::refresh)),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .p_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(div().flex_1().child(self.filter.clone()))
                    .child(
                        Button::new(
                            "apply-document-filter",
                            text(language, "应用筛选", "Apply Filter"),
                        )
                        .size(ButtonSize::Compact)
                        .disabled(
                            loading || matches!(status, DocumentSessionStatus::Unavailable(_)),
                        )
                        .on_click(cx.listener(Self::apply_filter)),
                    ),
            )
            .child(content)
            .child(
                h_flex()
                    .h(px(38.0))
                    .flex_none()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_t_1()
                    .border_color(colors.border)
                    .child(
                        Button::new(
                            "previous-document-page",
                            text(language, "上一页", "Previous"),
                        )
                        .size(ButtonSize::Compact)
                        .disabled(loading || page_number == 1)
                        .on_click(cx.listener(Self::previous_page)),
                    )
                    .child(
                        Label::new(format!("{} {page_number}", text(language, "第", "Page")))
                            .size(LabelSize::XSmall),
                    )
                    .child(
                        Button::new("next-document-page", text(language, "下一页", "Next"))
                            .size(ButtonSize::Compact)
                            .disabled(loading || !can_advance)
                            .on_click(cx.listener(Self::next_page)),
                    ),
            )
    }
}

fn centered_state(message: impl Into<SharedString>, color: Color) -> AnyElement {
    v_flex()
        .flex_1()
        .justify_center()
        .items_center()
        .p_6()
        .text_center()
        .child(Label::new(message).size(LabelSize::Small).color(color))
        .into_any_element()
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn document_rows(document: &Value) -> Vec<AnyElement> {
    let Value::Object(fields) = document else {
        return vec![json_row("value", document)];
    };
    fields
        .iter()
        .map(|(name, value)| json_row(name, value))
        .collect()
}

fn json_row(name: impl Into<SharedString>, value: &Value) -> AnyElement {
    let rendered = match value {
        Value::String(value) => value.clone(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    };
    v_flex()
        .gap_0p5()
        .py_1()
        .child(
            h_flex()
                .gap_1()
                .child(
                    Label::new(name)
                        .size(LabelSize::XSmall)
                        .weight(gpui_kit::FontWeight::MEDIUM),
                )
                .child(
                    Label::new(json_type_name(value))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        )
        .child(
            Label::new(rendered)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .into_any_element()
}
