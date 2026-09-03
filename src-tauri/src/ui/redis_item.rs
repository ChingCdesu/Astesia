use std::sync::Arc;

use gpui::{
    ClickEvent, Entity, EventEmitter, FocusHandle, Hsla, PromptButton, PromptLevel, Subscription,
};
use ui_input::InputField;
use zed_ui::{prelude::*, TintColor};

use crate::application::{
    Application, QueryTarget, RedisKeySnapshot, RedisListSide, RedisMutation, RedisValue,
};

use super::{localization::text, shell::ShellSettings};

#[derive(Debug)]
enum RedisItemState {
    Loading { generation: u64 },
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

    pub(super) fn matches(&self, target: &QueryTarget, key: &str) -> bool {
        &self.target == target && self.key == key
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
            cx.notify();
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, RedisItemState::Unavailable(_)) {
            return;
        }
        self.next_generation = self.next_generation.saturating_add(1);
        let generation = self.next_generation;
        self.state = RedisItemState::Loading { generation };
        cx.notify();
        let application = self.application.clone();
        let target = self.target.clone();
        let key = self.key.clone();
        let load =
            gpui_tokio::Tokio::spawn(
                cx,
                async move { application.redis().key(&target, &key).await },
            );
        cx.spawn(async move |item, cx| {
            let result = match load.await {
                Ok(result) => result,
                Err(error) => Err(format!("Redis key task ended unexpectedly: {error}")),
            };
            item.update(cx, |item, cx| {
                if !matches!(item.state, RedisItemState::Loading { generation: current } if current == generation)
                {
                    return;
                }
                item.state = match result {
                    Ok(snapshot) => RedisItemState::Ready(snapshot),
                    Err(error) => RedisItemState::Failed(error),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.load(cx);
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

    fn remove_hash_field(&mut self, field: String, cx: &mut Context<Self>) {
        self.start_mutation(RedisMutation::HashDelete { field }, false, cx);
    }

    fn remove_list_value(&mut self, value: String, cx: &mut Context<Self>) {
        self.start_mutation(RedisMutation::ListRemove { count: 1, value }, false, cx);
    }

    fn remove_set_member(&mut self, member: String, cx: &mut Context<Self>) {
        self.start_mutation(RedisMutation::SetRemove { member }, false, cx);
    }

    fn remove_sorted_set_member(&mut self, member: String, cx: &mut Context<Self>) {
        self.start_mutation(RedisMutation::SortedSetRemove { member }, false, cx);
    }

    fn start_mutation(
        &mut self,
        mutation: RedisMutation,
        refresh_catalog: bool,
        cx: &mut Context<Self>,
    ) {
        if self.mutation_busy || matches!(self.state, RedisItemState::Unavailable(_)) {
            return;
        }
        self.mutation_busy = true;
        self.notice = None;
        cx.notify();
        let application = self.application.clone();
        let target = self.target.clone();
        let key = self.key.clone();
        let mutation = gpui_tokio::Tokio::spawn(cx, async move {
            application.redis().mutate(&target, &key, mutation).await
        });
        cx.spawn(async move |item, cx| {
            let result = match mutation.await {
                Ok(result) => result,
                Err(error) => Err(format!("Redis mutation task ended unexpectedly: {error}")),
            };
            item.update(cx, |item, cx| {
                item.mutation_busy = false;
                let succeeded = result.is_ok();
                item.notice = Some(result.map(|affected| format!("Updated {affected} item(s)")));
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
        if self.mutation_busy || window.has_active_prompt() {
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
        let colors = cx.theme().colors().clone();
        let language = self.settings.read(cx).language();
        let rows = match &snapshot.value {
            RedisValue::Missing => {
                return centered_state(
                    text(
                        language,
                        "此键不存在或已过期",
                        "This key is missing or has expired",
                    ),
                    Color::Warning,
                );
            }
            RedisValue::String(value) => {
                vec![redis_row("value", value.clone(), None, colors.border)]
            }
            RedisValue::Hash(values) => values
                .iter()
                .enumerate()
                .map(|(index, (field, value))| {
                    let field_to_remove = field.clone();
                    redis_row(
                        field.clone(),
                        value.clone(),
                        Some(
                            IconButton::new(format!("remove-hash-field-{index}"), IconName::Trash)
                                .icon_size(IconSize::XSmall)
                                .disabled(self.mutation_busy)
                                .on_click(cx.listener(move |item, _, _, cx| {
                                    item.remove_hash_field(field_to_remove.clone(), cx);
                                }))
                                .into_any_element(),
                        ),
                        colors.border,
                    )
                })
                .collect(),
            RedisValue::List(values) => values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let value_to_remove = value.clone();
                    redis_row(
                        index.to_string(),
                        value.clone(),
                        Some(
                            IconButton::new(format!("remove-list-value-{index}"), IconName::Trash)
                                .icon_size(IconSize::XSmall)
                                .disabled(self.mutation_busy)
                                .on_click(cx.listener(move |item, _, _, cx| {
                                    item.remove_list_value(value_to_remove.clone(), cx);
                                }))
                                .into_any_element(),
                        ),
                        colors.border,
                    )
                })
                .collect(),
            RedisValue::Set(values) => values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let member = value.clone();
                    redis_row(
                        text(language, "成员", "member"),
                        value.clone(),
                        Some(
                            IconButton::new(format!("remove-set-member-{index}"), IconName::Trash)
                                .icon_size(IconSize::XSmall)
                                .disabled(self.mutation_busy)
                                .on_click(cx.listener(move |item, _, _, cx| {
                                    item.remove_set_member(member.clone(), cx);
                                }))
                                .into_any_element(),
                        ),
                        colors.border,
                    )
                })
                .collect(),
            RedisValue::SortedSet(values) => values
                .iter()
                .enumerate()
                .map(|(index, (member, score))| {
                    let member_to_remove = member.clone();
                    redis_row(
                        score.to_string(),
                        member.clone(),
                        Some(
                            IconButton::new(format!("remove-zset-member-{index}"), IconName::Trash)
                                .icon_size(IconSize::XSmall)
                                .disabled(self.mutation_busy)
                                .on_click(cx.listener(move |item, _, _, cx| {
                                    item.remove_sorted_set_member(member_to_remove.clone(), cx);
                                }))
                                .into_any_element(),
                        ),
                        colors.border,
                    )
                })
                .collect(),
        };
        v_flex()
            .id("redis-value-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .children(rows)
            .into_any_element()
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
                    .disabled(self.mutation_busy)
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
                    .disabled(self.mutation_busy)
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
                    .disabled(self.mutation_busy)
                    .on_click(cx.listener(Self::list_push_left)),
                )
                .child(
                    Button::new(
                        "push-redis-list-right",
                        text(language, "右侧添加", "Push Right"),
                    )
                    .size(ButtonSize::Compact)
                    .disabled(self.mutation_busy)
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
                    .disabled(self.mutation_busy)
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
                    .disabled(self.mutation_busy)
                    .on_click(cx.listener(Self::sorted_set_add)),
                )
                .into_any_element(),
        };
        controls
    }
}

impl Render for RedisItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let language = self.settings.read(cx).language();
        let loading = matches!(self.state, RedisItemState::Loading { .. });
        let content = match &self.state {
            RedisItemState::Loading { .. } => centered_state(
                text(language, "正在加载 Redis 键…", "Loading Redis key…"),
                Color::Muted,
            ),
            RedisItemState::Failed(error) => centered_state(error.clone(), Color::Error),
            RedisItemState::Unavailable(reason) => centered_state(reason.clone(), Color::Warning),
            RedisItemState::Ready(snapshot) => self.render_value(snapshot, cx),
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
                            .weight(gpui::FontWeight::MEDIUM)
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
                                loading || matches!(self.state, RedisItemState::Unavailable(_)),
                            )
                            .on_click(cx.listener(Self::refresh)),
                    )
                    .child(
                        Button::new("delete-redis-key", text(language, "删除", "Delete"))
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Tinted(TintColor::Error))
                            .disabled(
                                self.mutation_busy
                                    || !matches!(self.state, RedisItemState::Ready(_)),
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
        .min_h(px(34.0))
        .flex_none()
        .px_3()
        .gap_2()
        .border_b_1()
        .border_color(border)
        .child(
            div()
                .w(px(180.0))
                .child(Label::new(name).size(LabelSize::XSmall).color(Color::Muted)),
        )
        .child(
            div()
                .flex_1()
                .child(Label::new(value).size(LabelSize::XSmall)),
        )
        .when_some(action, |element, action| element.child(action))
        .into_any_element()
}
