use crate::ui::components::{prelude::*, Tooltip};
use gpui_kit::FontWeight;

use super::{ConnectionProfilesPanel, NoticeTone, OpenProfileMenu, PanelNotice};
use crate::application::connection_workspace::{
    ConnectionWorkspaceError, DatabaseListState, ProfileOperationKind,
};
use crate::application::{ConnectionProfileSnapshot, ConnectionWorkspaceSnapshot};
use crate::ui::engine_presentation::{engine_label, profile_endpoint};
use crate::ui::localization::text;

impl ConnectionProfilesPanel {
    fn render_initial_error(
        &self,
        error: &ConnectionWorkspaceError,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.settings.read(cx).language();
        v_flex()
            .flex_1()
            .justify_center()
            .items_center()
            .gap_3()
            .p_4()
            .text_center()
            .child(Label::new(error.message.clone()).size(LabelSize::Small))
            .child(
                Label::new(error.remediation.clone())
                    .size(LabelSize::XSmall)
                    .line_clamp(3),
            )
            .child(
                Label::new(format!(
                    "{}{}",
                    text(language, "错误码：", "Error code: "),
                    error.code
                ))
                .size(LabelSize::XSmall),
            )
            .child(
                Button::new("retry-connection-profiles", text(language, "重试", "Retry"))
                    .size(ButtonSize::Compact)
                    .on_click(cx.listener(Self::refresh)),
            )
            .into_any_element()
    }

    fn render_refresh_error(
        &self,
        error: &ConnectionWorkspaceError,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.settings.read(cx).language();
        v_flex()
            .gap_1()
            .m_2()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().status().error_border)
            .bg(cx.theme().status().error.opacity(0.08))
            .child(
                Label::new(error.message.clone())
                    .size(LabelSize::XSmall)
                    .color(Color::Error),
            )
            .child(
                Label::new(error.remediation.clone())
                    .size(LabelSize::XSmall)
                    .line_clamp(2),
            )
            .child(
                Button::new(
                    "retry-stale-connection-profiles",
                    text(language, "重新加载", "Reload"),
                )
                .size(ButtonSize::Compact)
                .on_click(cx.listener(Self::refresh)),
            )
            .into_any_element()
    }

    pub(super) fn render_profile(
        &self,
        snapshot: &ConnectionProfileSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.settings.read(cx).language();
        let profile = &snapshot.profile;
        let profile_id = profile.id.clone();
        let selected = self.selected_profile_id.as_deref() == Some(profile.id.as_str());
        let operation = self.state.operation(&profile.id);
        let loading = matches!(operation, Some(ProfileOperationKind::Connecting))
            || (snapshot.session.is_connected()
                && matches!(
                    self.state.databases(&profile.id),
                    None | Some(DatabaseListState::Loading { .. })
                ));
        let session_label = match operation {
            Some(ProfileOperationKind::Connecting) => text(language, "连接中", "Connecting"),
            Some(ProfileOperationKind::Disconnecting) => text(language, "断开中", "Disconnecting"),
            Some(ProfileOperationKind::Deleting) => text(language, "删除中", "Deleting"),
            None if snapshot.session.is_connected() => text(language, "已连接", "Connected"),
            None if self.failed_profiles.contains(&profile.id) => {
                text(language, "连接失败", "Connection failed")
            }
            None => text(language, "未连接", "Disconnected"),
        };
        let mcp_label = snapshot
            .mcp_usage
            .as_ref()
            .filter(|usage| usage.mcp_in_use)
            .map(|usage| {
                if usage.mcp_session_count > 0 {
                    format!("MCP {}", usage.mcp_session_count)
                } else {
                    "MCP".to_string()
                }
            });
        let profile_id_for_action = profile_id.clone();
        let profile_id_for_click = profile_id.clone();
        let mut aria_label = format!(
            "{}，{}，{}",
            profile.name,
            engine_label(profile.db_type),
            session_label,
        );
        if let Some(label) = &mcp_label {
            aria_label.push_str(&format!(
                "，{} ({label})",
                text(language, "MCP 正在使用", "In use by MCP")
            ));
        }
        let tooltip = format!(
            "{aria_label}\n{}{}",
            profile_endpoint(profile),
            if profile.tags.is_empty() {
                String::new()
            } else {
                format!("\n{}", profile.tags.join(", "))
            }
        );
        if loading {
            aria_label.push_str(&format!(", {}", text(language, "正在加载…", "Loading…")));
        }
        let status_color = if loading {
            Color::Muted
        } else if operation.is_some() {
            Color::Warning
        } else if snapshot.session.is_connected() {
            Color::Success
        } else if self.failed_profiles.contains(&profile.id) {
            Color::Error
        } else {
            Color::Muted
        };
        let menu_profile_id = profile_id.clone();
        let keyboard_profile_id = profile_id.clone();
        let row_bounds = std::rc::Rc::new(std::cell::Cell::new(None::<gpui_kit::Bounds<Pixels>>));
        let painted_bounds = row_bounds.clone();
        let row = crate::ui::components::ListItem::new(format!("connection-profile-{profile_id}"))
            .w_full()
            .inset(false)
            .text_size(px(12.0))
            .spacing(crate::ui::components::ListItemSpacing::Dense)
            .toggle_state(
                self.selected_sidebar_row
                    .as_ref()
                    .map_or(selected && self.selected_query_target.is_none(), |key| {
                        key == &format!("profile-{profile_id}")
                    }),
            )
            .aria_role(gpui_kit::Role::Button)
            .aria_label(aria_label)
            .tooltip(Tooltip::text(tooltip))
            .start_slot(crate::ui::components::Indicator::dot().color(status_color))
            .child(
                h_flex().h_6().flex_1().min_w_0().child(
                    Label::new(if loading {
                        text(language, "正在加载…", "Loading…").to_string()
                    } else {
                        profile.name.clone()
                    })
                    .truncate()
                    .when(loading, |label| label.color(Color::Muted)),
                ),
            )
            .end_slot(
                h_flex()
                    .gap_1()
                    .when_some(mcp_label, |row, label| {
                        row.child(
                            Label::new(label)
                                .size(LabelSize::XSmall)
                                .color(Color::Warning),
                        )
                    })
                    .child(
                        Label::new(engine_label(profile.db_type))
                            .text_size(px(10.0))
                            .flex_shrink_0()
                            .color(Color::Muted),
                    ),
            )
            .on_click(cx.listener(move |panel, event, window, cx| {
                panel.select_profile(profile_id_for_click.clone(), event, window, cx);
            }))
            .on_secondary_mouse_down(cx.listener(
                move |panel, event: &gpui_kit::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    panel.open_profile_menu(menu_profile_id.clone(), event.position, window, cx);
                },
            ));
        div()
            .child(
                div()
                    .id(format!("profile-focus-{profile_id}"))
                    .relative()
                    .role(gpui_kit::Role::Group)
                    .aria_label(profile.name.clone())
                    .tab_index(0)
                    .when(selected, |row| {
                        row.track_focus(&self.selected_profile_focus)
                    })
                    .key_context("ConnectionProfileRow")
                    .on_action(cx.listener(move |panel, _: &OpenProfileMenu, window, cx| {
                        if let Some(bounds) = row_bounds.get() {
                            panel.open_profile_menu(
                                keyboard_profile_id.clone(),
                                bounds.bottom_left(),
                                window,
                                cx,
                            );
                        }
                    }))
                    .on_action(cx.listener(move |panel, _: &menu::Confirm, _, cx| {
                        panel.select_profile_id(profile_id_for_action.clone(), cx);
                    }))
                    .child(row)
                    .child(
                        gpui_kit::canvas(
                            |bounds, _, _| bounds,
                            move |_, bounds, _, _| painted_bounds.set(Some(bounds)),
                        )
                        .absolute()
                        .size_full(),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_database_list(
        &self,
        snapshot: &ConnectionProfileSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let profile = &snapshot.profile;
        let language = self.settings.read(cx).language();
        match self.state.databases(&profile.id) {
            None => h_flex()
                .pl(DynamicSpacing::Base08.rems(cx))
                .pt_1()
                .child(
                    Label::new(text(
                        language,
                        "等待加载数据库…",
                        "Waiting to load databases…",
                    ))
                    .size(LabelSize::XSmall),
                )
                .into_any_element(),
            Some(DatabaseListState::Loading { .. }) => h_flex()
                .pl(DynamicSpacing::Base08.rems(cx))
                .pt_1()
                .child(
                    Label::new(text(language, "正在加载数据库…", "Loading databases…"))
                        .size(LabelSize::XSmall),
                )
                .into_any_element(),
            Some(DatabaseListState::Failed { error, .. }) => {
                let connection_id = profile.id.clone();
                v_flex()
                    .pl(DynamicSpacing::Base08.rems(cx))
                    .pt_1()
                    .gap_1()
                    .child(
                        Label::new(error.clone())
                            .size(LabelSize::XSmall)
                            .color(Color::Error)
                            .line_clamp(2),
                    )
                    .child(
                        Button::new(
                            format!("retry-databases-{connection_id}"),
                            text(language, "重试", "Retry"),
                        )
                        .size(ButtonSize::Compact)
                        .on_click(cx.listener(
                            move |panel, event, window, cx| {
                                panel.retry_databases(connection_id.clone(), event, window, cx);
                            },
                        )),
                    )
                    .into_any_element()
            }
            Some(DatabaseListState::Ready { databases, .. }) if databases.is_empty() => h_flex()
                .pl(DynamicSpacing::Base08.rems(cx))
                .pt_1()
                .child(
                    Label::new(text(language, "未发现数据库", "No databases found"))
                        .size(LabelSize::XSmall),
                )
                .into_any_element(),
            Some(DatabaseListState::Ready { .. }) => div().into_any_element(),
        }
    }

    fn render_profiles(
        &self,
        snapshot: &ConnectionWorkspaceSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.settings.read(cx).language();
        if snapshot.profiles.is_empty() {
            return v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .gap_1()
                .p_4()
                .child(
                    Label::new(text(language, "暂无连接", "No connections")).size(LabelSize::Small),
                )
                .child(
                    Label::new(text(
                        language,
                        "创建一个连接配置开始工作",
                        "Create a connection profile to get started",
                    ))
                    .size(LabelSize::XSmall),
                )
                .child(
                    Button::new(
                        "create-first-connection-profile",
                        text(language, "新建连接", "New Connection"),
                    )
                    .size(ButtonSize::Compact)
                    .on_click(cx.listener(Self::create_profile)),
                )
                .into_any_element();
        }

        self.render_virtual_profiles(snapshot, cx)
    }

    fn render_notice(&self, notice: &PanelNotice, cx: &mut Context<Self>) -> AnyElement {
        let status = cx.theme().status();
        let (foreground, background, border) = match notice.tone {
            NoticeTone::Info => (status.info, status.info_background, status.info_border),
            NoticeTone::Warning => (
                status.warning,
                status.warning_background,
                status.warning_border,
            ),
            NoticeTone::Error => (status.error, status.error_background, status.error_border),
        };

        div()
            .mx_2()
            .mb_2()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(background)
            .child(
                Label::new(notice.message.clone())
                    .size(LabelSize::XSmall)
                    .color(Color::Custom(foreground))
                    .line_clamp(3),
            )
            .into_any_element()
    }
}

impl Render for ConnectionProfilesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_profile_menu(window, cx);
        let border = cx.theme().colors().border;
        let panel_background = cx.theme().colors().panel_background;
        let language = self.settings.read(cx).language();
        let content = match (self.state.snapshot(), self.state.error()) {
            (None, Some(error)) => self.render_initial_error(error, cx),
            (None, None) => v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .child(
                    Label::new(text(language, "正在加载连接…", "Loading connections…"))
                        .size(LabelSize::Small),
                )
                .into_any_element(),
            (Some(snapshot), error) => v_flex()
                .flex_1()
                .min_h_0()
                .when_some(error, |element, error| {
                    element.child(self.render_refresh_error(error, cx))
                })
                .child(self.render_profiles(snapshot, cx))
                .into_any_element(),
        };

        v_flex()
            .w_full()
            .min_w_0()
            .h_full()
            .flex_none()
            .min_h_0()
            .border_r_1()
            .border_color(border)
            .bg(panel_background)
            .child(
                h_flex()
                    .h(DynamicSpacing::Base32.rems(cx))
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        Label::new(text(language, "连接", "Connections"))
                            .size(LabelSize::Small)
                            .weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                IconButton::new("create-connection-profile", IconName::Plus)
                                    .aria_label(text(language, "新建连接", "New Connection"))
                                    .tooltip(Tooltip::text(text(
                                        language,
                                        "新建连接",
                                        "New Connection",
                                    )))
                                    .size(ButtonSize::Compact)
                                    .disabled(self.actions_blocked())
                                    .on_click(cx.listener(Self::create_profile)),
                            )
                            .child(
                                IconButton::new(
                                    "refresh-connection-profiles",
                                    if self.state.is_refreshing() {
                                        IconName::LoaderCircle
                                    } else {
                                        IconName::RotateCw
                                    },
                                )
                                .aria_label(text(language, "刷新连接", "Refresh Connections"))
                                .tooltip(Tooltip::text(text(
                                    language,
                                    "刷新连接",
                                    "Refresh Connections",
                                )))
                                .size(ButtonSize::Compact)
                                .disabled(self.state.is_refreshing())
                                .on_click(cx.listener(Self::refresh)),
                            ),
                    ),
            )
            .child(content)
            .when_some(self.notice.as_ref(), |element, notice| {
                element.child(self.render_notice(notice, cx))
            })
            .children(self.context_menu.as_ref().map(|(menu, position, _)| {
                gpui_kit::deferred(
                    gpui_kit::anchored()
                        .position(*position)
                        .anchor(gpui_kit::Anchor::TopLeft)
                        .child(menu.clone()),
                )
                .with_priority(3)
            }))
    }
}
