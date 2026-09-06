use std::sync::Arc;

use crate::ui::components::{prelude::*, TintColor, Tooltip};
use crate::ui::input_field::InputField;
use gpui_kit::{
    ClickEvent, ClipboardItem, Entity, EventEmitter, FocusHandle, Hsla, PromptButton, PromptLevel,
    Subscription,
};

use crate::application::{
    Application, QueryTarget, RedisKeySnapshot, RedisListSide, RedisMutation, RedisPageCursor,
    RedisValue,
};

use super::{localization::text, shell::ShellSettings};

#[derive(Debug)]
enum RedisItemState {
    Loading,
    Ready(RedisKeySnapshot),
    Failed(String),
    Unavailable(String),
}

pub(super) struct RedisItem {
    application: Arc<Application>,
    target: QueryTarget,
    key: String,
    state: RedisItemState,
    next_generation: u64,
    mutation_busy: bool,
    page_loading: bool,
    current_cursor: Option<RedisPageCursor>,
    notice: Option<Result<String, String>>,
    primary_input: Entity<InputField>,
    secondary_input: Entity<InputField>,
    ttl_input: Entity<InputField>,
    focus_handle: FocusHandle,
    settings: Entity<ShellSettings>,
    _primary_observation: Subscription,
    _secondary_observation: Subscription,
    _ttl_observation: Subscription,
    _settings_observation: Subscription,
}

#[derive(Clone, Debug)]
pub(super) struct RedisKeyDeleted {
    pub(super) target: QueryTarget,
}

impl EventEmitter<RedisKeyDeleted> for RedisItem {}

impl RedisItem {
    pub(super) fn new(
        application: Arc<Application>,
        target: QueryTarget,
        key: String,
        settings: Entity<ShellSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let language = settings.read(cx).language();
        let primary_input =
            cx.new(|cx| {
                InputField::new(window, cx, text(language, "值或成员", "Value or member"))
                    .label(text(language, "值或成员", "Value or member"))
            });
        let secondary_input =
            cx.new(|cx| {
                InputField::new(window, cx, text(language, "字段或分数", "Field or score"))
                    .label(text(language, "字段或分数", "Field or score"))
            });
        let ttl_input = cx.new(|cx| {
            InputField::new(
                window,
                cx,
                text(language, "留空表示永久", "Blank means persistent"),
            )
            .label(text(language, "TTL（秒）", "TTL (seconds)"))
        });
        let primary_observation = cx.observe(&primary_input, |_, _, cx| cx.notify());
        let secondary_observation = cx.observe(&secondary_input, |_, _, cx| cx.notify());
        let ttl_observation = cx.observe(&ttl_input, |_, _, cx| cx.notify());
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let mut item = Self {
            application,
            target,
            key,
            state: RedisItemState::Failed("Redis key has not loaded".to_string()),
            next_generation: 0,
            mutation_busy: false,
            page_loading: false,
            current_cursor: None,
            notice: None,
            primary_input,
            secondary_input,
            ttl_input,
            focus_handle: cx.focus_handle(),
            settings,
            _primary_observation: primary_observation,
            _secondary_observation: secondary_observation,
            _ttl_observation: ttl_observation,
            _settings_observation: settings_observation,
        };
        item.load(cx);
        item
    }

    pub(super) fn label(&self) -> String {
        format!(
            "{} · {}/{}",
            self.key, self.target.connection_name, self.target.database
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
        if self.target.connection_id == connection_id
            && self.target.session_generation == session_generation
        {
            let language = self.settings.read(cx).language();
            self.next_generation = self.next_generation.saturating_add(1);
            self.state = RedisItemState::Unavailable(
                text(
                    language,
                    "连接会话已更改。请从侧边栏重新打开 Redis 键。",
                    "The connection session changed. Reopen the Redis key from the sidebar.",
                )
                .to_string(),
            );
            self.mutation_busy = false;
            self.page_loading = false;
            cx.notify();
        }
    }

    fn is_busy(&self) -> bool {
        self.mutation_busy || self.page_loading
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.load_page(None, cx);
    }

    fn load_page(&mut self, cursor: Option<RedisPageCursor>, cx: &mut Context<Self>) {
        if matches!(self.state, RedisItemState::Unavailable(_)) {
            return;
        }
        self.next_generation = self.next_generation.saturating_add(1);
        let generation = self.next_generation;
        self.page_loading = true;
        if !matches!(self.state, RedisItemState::Ready(_)) {
            self.state = RedisItemState::Loading;
        }
        cx.notify();
        let application = self.application.clone();
        let target = self.target.clone();
        let key = self.key.clone();
        let load = crate::ui::runtime::spawn(cx, async move {
            match cursor {
                None => application.redis().key(&target, &key).await,
                Some(_) => application.redis().key_page(&target, &key, cursor).await,
            }
        });
        cx.spawn(async move |item, cx| {
            let result = match load.await {
                Ok(result) => result,
                Err(error) => Err(format!("Redis key task ended unexpectedly: {error}")),
            };
            item.update(cx, |item, cx| {
                if item.next_generation != generation {
                    return;
                }
                item.page_loading = false;
                match result {
                    Ok(snapshot) => {
                        item.current_cursor = cursor;
                        item.state = RedisItemState::Ready(snapshot);
                    }
                    Err(error) if matches!(item.state, RedisItemState::Ready(_)) => {
                        item.notice = Some(Err(error));
                    }
                    Err(error) => item.state = RedisItemState::Failed(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.is_busy() {
            self.notice = None;
            self.load(cx);
        }
    }

    fn next_page(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        let RedisItemState::Ready(snapshot) = &self.state else {
            return;
        };
        if let Some(cursor) = snapshot.next_page {
            self.notice = None;
            self.load_page(Some(cursor), cx);
        }
    }

    fn input_text(field: &Entity<InputField>, cx: &Context<Self>) -> String {
        field.read(cx).text(cx)
    }

    fn set_string(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let value = Self::input_text(&self.primary_input, cx);
        let ttl = Self::input_text(&self.ttl_input, cx);
        let ttl_seconds = if ttl.trim().is_empty() {
            None
        } else {
            match ttl.trim().parse::<u64>() {
                Ok(0) | Err(_) => {
                    self.notice = Some(Err(text(
                        self.settings.read(cx).language(),
                        "TTL 必须是正整数",
                        "TTL must be a positive integer",
                    )
                    .to_string()));
                    cx.notify();
                    return;
                }
                Ok(ttl) => Some(ttl),
            }
        };
        self.start_mutation(RedisMutation::SetString { value, ttl_seconds }, false, cx);
    }

    fn hash_set(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.start_mutation(
            RedisMutation::HashSet {
                field: Self::input_text(&self.secondary_input, cx),
                value: Self::input_text(&self.primary_input, cx),
            },
            false,
            cx,
        );
    }

    fn list_push_left(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.start_mutation(
            RedisMutation::ListPush {
                side: RedisListSide::Left,
                value: Self::input_text(&self.primary_input, cx),
            },
            false,
            cx,
        );
    }

    fn list_push_right(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.start_mutation(
            RedisMutation::ListPush {
                side: RedisListSide::Right,
                value: Self::input_text(&self.primary_input, cx),
            },
            false,
            cx,
        );
    }

    fn set_add(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.start_mutation(
            RedisMutation::SetAdd {
                member: Self::input_text(&self.primary_input, cx),
            },
            false,
            cx,
        );
    }

    fn sorted_set_add(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let score = match Self::input_text(&self.secondary_input, cx)
            .trim()
            .parse::<f64>()
        {
            Ok(score) if score.is_finite() => score,
            _ => {
                self.notice = Some(Err(text(
                    self.settings.read(cx).language(),
                    "分数必须是有限数字",
                    "Score must be a finite number",
                )
                .to_string()));
                cx.notify();
                return;
            }
        };
        self.start_mutation(
            RedisMutation::SortedSetAdd {
                member: Self::input_text(&self.primary_input, cx),
                score,
            },
            false,
            cx,
        );
    }

    fn remove_entry(&mut self, index: usize, generation: u64, cx: &mut Context<Self>) {
        if self.is_busy() || self.next_generation != generation {
            return;
        }
        let RedisItemState::Ready(snapshot) = &self.state else {
            return;
        };
        if let Some(mutation) = entry_removal(&snapshot.value, index) {
            self.start_mutation(mutation, false, cx);
        }
    }

    fn copy_entry(&self, index: usize, generation: u64, cx: &mut Context<Self>) {
        if self.is_busy() || self.next_generation != generation {
            return;
        }
        let RedisItemState::Ready(snapshot) = &self.state else {
            return;
        };
        if let Some(value) = entry_text(&snapshot.value, index) {
            cx.write_to_clipboard(ClipboardItem::new_string(value));
        }
    }

    fn start_mutation(
        &mut self,
        mutation: RedisMutation,
        refresh_catalog: bool,
        cx: &mut Context<Self>,
    ) {
        if self.is_busy() || matches!(self.state, RedisItemState::Unavailable(_)) {
            return;
        }
        self.mutation_busy = true;
        self.notice = None;
        cx.notify();
        let application = self.application.clone();
        let target = self.target.clone();
        let key = self.key.clone();
        let generation = self.next_generation;
        let mutation = crate::ui::runtime::spawn(cx, async move {
            application.redis().mutate(&target, &key, mutation).await
        });
        cx.spawn(async move |item, cx| {
            let result = match mutation.await {
                Ok(result) => result,
                Err(error) => Err(format!("Redis mutation task ended unexpectedly: {error}")),
            };
            item.update(cx, |item, cx| {
                if item.next_generation != generation {
                    return;
                }
                item.mutation_busy = false;
                let succeeded = result.is_ok();
                let language = item.settings.read(cx).language();
                item.notice = Some(result.map(|affected| {
                    format!(
                        "{} {affected}",
                        text(language, "已更新项数：", "Items updated:")
                    )
                }));
                item.load(cx);
                if succeeded && refresh_catalog {
                    cx.emit(RedisKeyDeleted {
                        target: item.target.clone(),
                    });
                }
            })
            .ok();
        })
        .detach();
    }

    fn confirm_delete(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy() || window.has_active_prompt() {
            return;
        }
        let language = self.settings.read(cx).language();
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!(
                "{} “{}”?",
                text(language, "删除 Redis 键", "Delete Redis key"),
                self.key
            ),
            Some(text(
                language,
                "此操作无法撤销。",
                "This action cannot be undone.",
            )),
            &[
                PromptButton::ok(text(language, "删除", "Delete")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |item, cx| {
            if answer.await.ok() == Some(0) {
                item.update_in(cx, |item, _, cx| {
                    item.start_mutation(RedisMutation::Delete, true, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    fn render_value(&self, snapshot: &RedisKeySnapshot, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings.read(cx).language();
        if matches!(snapshot.value, RedisValue::Missing) {
            return centered_state(
                text(
                    language,
                    "此键不存在或已过期",
                    "This key is missing or has expired",
                ),
                Color::Warning,
            );
        }
        if snapshot.value.entry_count() == 0 {
            return centered_state(
                text(
                    language,
                    "当前批次没有数据。可继续下一批或刷新。",
                    "No entries in this batch. Continue to the next batch or refresh.",
                ),
                Color::Muted,
            );
        }
        gpui_kit::uniform_list(
            "redis-value-rows",
            snapshot.value.entry_count(),
            cx.processor(|item, visible_range: std::ops::Range<usize>, _, cx| {
                visible_range
                    .filter_map(|index| item.render_entry(index, cx))
                    .collect()
            }),
        )
        .flex_1()
        .min_h_0()
        .into_any_element()
    }

    fn render_entry(&self, index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let RedisItemState::Ready(snapshot) = &self.state else {
            return None;
        };
        let language = self.settings.read(cx).language();
        let (name, value, removable) = match &snapshot.value {
            RedisValue::Missing => return None,
            RedisValue::String(value) if index == 0 => ("value".to_string(), preview(value), false),
            RedisValue::String(_) => return None,
            RedisValue::Hash(values) => {
                let (field, value) = values.get(index)?;
                (preview(field), preview(value), true)
            }
            RedisValue::List(values) => (
                (snapshot.offset + index as u64).to_string(),
                preview(values.get(index)?),
                true,
            ),
            RedisValue::Set(values) => (
                text(language, "成员", "member").to_string(),
                preview(values.get(index)?),
                true,
            ),
            RedisValue::SortedSet(values) => {
                let (member, score) = values.get(index)?;
                (score.to_string(), preview(member), true)
            }
        };
        let generation = self.next_generation;
        let copy_label = text(language, "复制完整行", "Copy full row");
        let remove_label = text(language, "移除此项", "Remove this entry");
        let actions = h_flex()
            .gap_1()
            .child(
                IconButton::new(format!("copy-redis-entry-{index}"), IconName::Copy)
                    .icon_size(IconSize::XSmall)
                    .aria_label(copy_label)
                    .tooltip(Tooltip::text(copy_label))
                    .disabled(self.is_busy())
                    .on_click(
                        cx.listener(move |item, _, _, cx| item.copy_entry(index, generation, cx)),
                    ),
            )
            .when(removable, |element| {
                element.child(
                    IconButton::new(format!("remove-redis-entry-{index}"), IconName::Trash)
                        .icon_size(IconSize::XSmall)
                        .aria_label(remove_label)
                        .tooltip(Tooltip::text(remove_label))
                        .disabled(self.is_busy())
                        .on_click(cx.listener(move |item, _, _, cx| {
                            item.remove_entry(index, generation, cx)
                        })),
                )
            });
        Some(redis_row(
            name,
            value,
            Some(actions.into_any_element()),
            cx.theme().colors().border,
        ))
    }

    fn render_paging_controls(
        &self,
        snapshot: &RedisKeySnapshot,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if matches!(snapshot.value, RedisValue::Missing | RedisValue::String(_)) {
            return None;
        }
        let language = self.settings.read(cx).language();
        let count = snapshot.value.entry_count();
        let total = snapshot.total_entries;
        Some(
            h_flex()
                .flex_none()
                .px_3()
                .py_1()
                .gap_2()
                .border_t_1()
                .border_color(cx.theme().colors().border)
                .child(
                    Label::new(format!(
                        "{} {count} · {} {total}",
                        text(language, "本批", "This batch"),
                        text(language, "总计", "Total")
                    ))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted)
                    .flex_1(),
                )
                .child(
                    Button::new(
                        "first-redis-page",
                        text(language, "返回首批", "First batch"),
                    )
                    .size(ButtonSize::Compact)
                    .disabled(self.is_busy() || self.current_cursor.is_none())
                    .on_click(cx.listener(Self::refresh)),
                )
                .child(
                    Button::new("next-redis-page", text(language, "下一批", "Next batch"))
                        .size(ButtonSize::Compact)
                        .disabled(self.is_busy() || snapshot.next_page.is_none())
                        .on_click(cx.listener(Self::next_page)),
                )
                .into_any_element(),
        )
    }

    fn render_mutation_controls(&self, value: &RedisValue, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings.read(cx).language();
        let controls = match value {
            RedisValue::Missing | RedisValue::String(_) => h_flex()
                .gap_2()
                .child(div().flex_1().child(self.primary_input.clone()))
                .child(div().w(px(180.0)).child(self.ttl_input.clone()))
                .child(
                    Button::new(
                        "set-redis-string",
                        text(language, "保存字符串", "Save String"),
                    )
                    .size(ButtonSize::Compact)
                    .loading(self.mutation_busy)
                    .disabled(self.is_busy())
                    .on_click(cx.listener(Self::set_string)),
                )
                .into_any_element(),
            RedisValue::Hash(_) => h_flex()
                .gap_2()
                .child(div().flex_1().child(self.secondary_input.clone()))
                .child(div().flex_1().child(self.primary_input.clone()))
                .child(
                    Button::new(
                        "set-redis-hash-field",
                        text(language, "设置字段", "Set Field"),
                    )
                    .size(ButtonSize::Compact)
                    .loading(self.mutation_busy)
                    .disabled(self.is_busy())
                    .on_click(cx.listener(Self::hash_set)),
                )
                .into_any_element(),
            RedisValue::List(_) => h_flex()
                .gap_2()
                .child(div().flex_1().child(self.primary_input.clone()))
                .child(
                    Button::new(
                        "push-redis-list-left",
                        text(language, "左侧添加", "Push Left"),
                    )
                    .size(ButtonSize::Compact)
                    .disabled(self.is_busy())
                    .on_click(cx.listener(Self::list_push_left)),
                )
                .child(
                    Button::new(
                        "push-redis-list-right",
                        text(language, "右侧添加", "Push Right"),
                    )
                    .size(ButtonSize::Compact)
                    .disabled(self.is_busy())
                    .on_click(cx.listener(Self::list_push_right)),
                )
                .into_any_element(),
            RedisValue::Set(_) => h_flex()
                .gap_2()
                .child(div().flex_1().child(self.primary_input.clone()))
                .child(
                    Button::new(
                        "add-redis-set-member",
                        text(language, "添加成员", "Add Member"),
                    )
                    .size(ButtonSize::Compact)
                    .disabled(self.is_busy())
                    .on_click(cx.listener(Self::set_add)),
                )
                .into_any_element(),
            RedisValue::SortedSet(_) => h_flex()
                .gap_2()
                .child(div().flex_1().child(self.primary_input.clone()))
                .child(div().w(px(180.0)).child(self.secondary_input.clone()))
                .child(
                    Button::new(
                        "add-redis-zset-member",
                        text(language, "设置成员", "Set Member"),
                    )
                    .size(ButtonSize::Compact)
                    .disabled(self.is_busy())
                    .on_click(cx.listener(Self::sorted_set_add)),
                )
                .into_any_element(),
        };
        controls
    }
}

impl Render for RedisItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let language = self.settings.read(cx).language();
        let loading = self.page_loading;
        let content = match &self.state {
            RedisItemState::Loading => centered_state(
                text(language, "正在加载 Redis 键…", "Loading Redis key…"),
                Color::Muted,
            ),
            RedisItemState::Failed(error) => centered_state(error.clone(), Color::Error),
            RedisItemState::Unavailable(reason) => centered_state(reason.clone(), Color::Warning),
            RedisItemState::Ready(snapshot) => self.render_value(snapshot, cx),
        };
        let paging_controls = match &self.state {
            RedisItemState::Ready(snapshot) => self.render_paging_controls(snapshot, cx),
            _ => None,
        };
        let metadata = match &self.state {
            RedisItemState::Ready(snapshot) => Some((
                redis_type_name(&snapshot.value),
                snapshot
                    .ttl_seconds
                    .map(|ttl| format!("TTL {ttl}s"))
                    .unwrap_or_else(|| text(language, "永久", "persistent").to_string()),
            )),
            _ => None,
        };
        let mutation_controls = match &self.state {
            RedisItemState::Ready(snapshot) => {
                Some(self.render_mutation_controls(&snapshot.value, cx))
            }
            _ => None,
        };

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("RedisItem")
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
                    .when_some(metadata, |element, (kind, ttl)| {
                        element
                            .child(Label::new(kind).size(LabelSize::XSmall).color(Color::Muted))
                            .child(Label::new(ttl).size(LabelSize::XSmall).color(Color::Muted))
                    })
                    .child(
                        Button::new("refresh-redis-key", text(language, "刷新", "Refresh"))
                            .size(ButtonSize::Compact)
                            .loading(loading)
                            .disabled(
                                self.is_busy()
                                    || matches!(self.state, RedisItemState::Unavailable(_)),
                            )
                            .on_click(cx.listener(Self::refresh)),
                    )
                    .child(
                        Button::new("delete-redis-key", text(language, "删除", "Delete"))
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Tinted(TintColor::Error))
                            .disabled(
                                self.is_busy() || !matches!(self.state, RedisItemState::Ready(_)),
                            )
                            .on_click(cx.listener(Self::confirm_delete)),
                    ),
            )
            .when_some(self.notice.clone(), |element, notice| {
                let (message, color) = match notice {
                    Ok(message) => (message, Color::Success),
                    Err(message) => (message, Color::Error),
                };
                element.child(
                    h_flex()
                        .px_3()
                        .py_1()
                        .border_b_1()
                        .border_color(colors.border)
                        .child(Label::new(message).size(LabelSize::XSmall).color(color)),
                )
            })
            .when_some(mutation_controls, |element, controls| {
                element.child(
                    div()
                        .flex_none()
                        .p_2()
                        .border_b_1()
                        .border_color(colors.border)
                        .child(controls),
                )
            })
            .child(content)
            .when_some(paging_controls, |element, controls| element.child(controls))
    }
}

fn redis_type_name(value: &RedisValue) -> &'static str {
    match value {
        RedisValue::Missing => "missing",
        RedisValue::String(_) => "string",
        RedisValue::Hash(_) => "hash",
        RedisValue::List(_) => "list",
        RedisValue::Set(_) => "set",
        RedisValue::SortedSet(_) => "zset",
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

fn redis_row(
    name: impl Into<SharedString>,
    value: impl Into<SharedString>,
    action: Option<AnyElement>,
    border: Hsla,
) -> AnyElement {
    h_flex()
        .h(px(34.0))
        .overflow_hidden()
        .flex_none()
        .px_3()
        .gap_2()
        .border_b_1()
        .border_color(border)
        .child(
            div().w(px(180.0)).min_w_0().child(
                Label::new(name)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted)
                    .truncate(),
            ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Label::new(value).size(LabelSize::XSmall).truncate()),
        )
        .when_some(action, |element, action| element.child(action))
        .into_any_element()
}

fn preview(value: &str) -> String {
    const PREVIEW_CHARACTERS: usize = 512;
    let mut characters = value.chars();
    let mut result = characters
        .by_ref()
        .take(PREVIEW_CHARACTERS)
        .collect::<String>();
    if characters.next().is_some() {
        result.push('…');
    }
    result
}

fn entry_text(value: &RedisValue, index: usize) -> Option<String> {
    match value {
        RedisValue::Missing => None,
        RedisValue::String(value) => (index == 0).then(|| value.clone()),
        RedisValue::Hash(values) => values
            .get(index)
            .map(|(field, value)| format!("{field}\t{value}")),
        RedisValue::List(values) | RedisValue::Set(values) => values.get(index).cloned(),
        RedisValue::SortedSet(values) => values
            .get(index)
            .map(|(member, score)| format!("{score}\t{member}")),
    }
}

fn entry_removal(value: &RedisValue, index: usize) -> Option<RedisMutation> {
    match value {
        RedisValue::Missing | RedisValue::String(_) => None,
        RedisValue::Hash(values) => values
            .get(index)
            .map(|(field, _)| RedisMutation::HashDelete {
                field: field.clone(),
            }),
        RedisValue::List(values) => values.get(index).map(|value| RedisMutation::ListRemove {
            count: 1,
            value: value.clone(),
        }),
        RedisValue::Set(values) => values.get(index).map(|member| RedisMutation::SetRemove {
            member: member.clone(),
        }),
        RedisValue::SortedSet(values) => {
            values
                .get(index)
                .map(|(member, _)| RedisMutation::SortedSetRemove {
                    member: member.clone(),
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_previews_are_bounded_without_changing_copy_or_mutation_identity() {
        let member = "数据\n".repeat(600);
        assert_eq!(preview(&member).chars().count(), 513);
        assert!(preview(&member).ends_with('…'));
        let value = RedisValue::Set(vec![member.clone()]);
        assert_eq!(entry_text(&value, 0), Some(member.clone()));
        assert_eq!(
            entry_removal(&value, 0),
            Some(RedisMutation::SetRemove { member })
        );
        assert_eq!(entry_removal(&value, 1), None);
    }

    #[test]
    fn hash_copy_and_removal_preserve_full_field_and_value() {
        let field = "字段".repeat(600);
        let value = "值".repeat(600);
        let hash = RedisValue::Hash(vec![(field.clone(), value.clone())]);
        assert_eq!(entry_text(&hash, 0), Some(format!("{field}\t{value}")));
        assert_eq!(
            entry_removal(&hash, 0),
            Some(RedisMutation::HashDelete { field })
        );
        assert_eq!(preview(""), "");
        assert_eq!(preview(&"a".repeat(512)), "a".repeat(512));
    }
}
