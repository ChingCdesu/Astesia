use gpui::{rgb, FontWeight};
use zed_ui::prelude::*;

use crate::application::connection_workspace::{
    ConnectionWorkspaceError, DatabaseListState, ProfileOperationKind,
};
use crate::application::{ConnectionProfileSnapshot, ConnectionWorkspaceSnapshot};
use crate::connection_repository::SharedConnectionProfile;

use super::presentation::grouped_profiles;
use super::{ConnectionProfilesPanel, NoticeTone, PanelNotice, SIDEBAR_WIDTH};
use crate::ui::engine_presentation::{engine_label, profile_color, profile_endpoint};

impl ConnectionProfilesPanel {
    fn render_initial_error(
        &self,
        error: &ConnectionWorkspaceError,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
            .child(Label::new(format!("错误码：{}", error.code)).size(LabelSize::XSmall))
            .child(
                Button::new("retry-connection-profiles", "重试")
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
                Button::new("retry-stale-connection-profiles", "重新加载")
                    .size(ButtonSize::Compact)
                    .on_click(cx.listener(Self::refresh)),
            )
            .into_any_element()
    }

    fn render_profile(
        &self,
        snapshot: &ConnectionProfileSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let profile = &snapshot.profile;
        let profile_id = profile.id.clone();
        let selected = self.selected_profile_id.as_deref() == Some(profile.id.as_str());
        let colors = cx.theme().colors();
        let operation = self.state.operation(&profile.id);
        let session_label = match operation {
            Some(ProfileOperationKind::Connecting) => "连接中",
            Some(ProfileOperationKind::Disconnecting) => "断开中",
            Some(ProfileOperationKind::Deleting) => "删除中",
            None if snapshot.session.is_connected() => "已连接",
            None => "未连接",
        };
        let mcp_label = snapshot.mcp_usage.as_ref().and_then(|usage| {
            usage.mcp_in_use.then(|| {
                if usage.mcp_session_count > 0 {
                    format!("MCP {}", usage.mcp_session_count)
                } else {
                    "MCP".to_string()
                }
            })
        });
        let tags = profile
            .tags
            .iter()
            .take(2)
            .map(|tag| {
                div()
                    .max_w(px(80.0))
                    .px_1()
                    .rounded_sm()
                    .bg(colors.element_background)
                    .child(Label::new(tag.clone()).size(LabelSize::XSmall).truncate())
                    .into_any_element()
            })
            .chain((profile.tags.len() > 2).then(|| {
                Label::new(format!("+{}", profile.tags.len() - 2))
                    .size(LabelSize::XSmall)
                    .into_any_element()
            }))
            .collect::<Vec<_>>();

        let profile_id_for_action = profile_id.clone();
        let profile_id_for_click = profile_id.clone();
        let aria_label = format!(
            "{}，{}，{}",
            profile.name,
            engine_label(profile.db_type),
            session_label,
        );
        let profile_header = v_flex()
            .id(format!("connection-profile-{profile_id}"))
            .px_2p5()
            .py_2()
            .gap_1()
            .rounded_md()
            .border_1()
            .border_color(colors.border.opacity(0.0))
            .cursor_pointer()
            .role(gpui::Role::Button)
            .aria_label(aria_label)
            .aria_toggled(if selected {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .tab_index(0)
            .key_context("ConnectionProfileRow")
            .focus_visible(|element| element.border_color(colors.border_focused))
            .hover(|element| element.bg(colors.ghost_element_hover))
            .on_action(cx.listener(move |panel, _: &menu::Confirm, _, cx| {
                panel.select_profile_id(profile_id_for_action.clone(), cx);
            }))
            .on_click(cx.listener(move |panel, event, window, cx| {
                panel.select_profile(profile_id_for_click.clone(), event, window, cx);
            }))
            .child(
                h_flex()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div()
                            .size(px(9.0))
                            .flex_none()
                            .rounded_full()
                            .bg(profile_color(profile)),
                    )
                    .child(
                        Label::new(profile.name.clone())
                            .size(LabelSize::Small)
                            .weight(FontWeight::MEDIUM)
                            .truncate()
                            .flex_1(),
                    )
                    .when_some(mcp_label, |element, label| {
                        element.child(
                            div()
                                .px_1()
                                .rounded_sm()
                                .bg(colors.element_background)
                                .child(Label::new(label).size(LabelSize::XSmall)),
                        )
                    })
                    .child(Label::new(session_label).size(LabelSize::XSmall)),
            )
            .child(
                h_flex()
                    .min_w_0()
                    .pl(px(17.0))
                    .gap_2()
                    .child(
                        Label::new(profile_endpoint(profile))
                            .size(LabelSize::XSmall)
                            .truncate()
                            .flex_1(),
                    )
                    .child(Label::new(engine_label(profile.db_type)).size(LabelSize::XSmall))
                    .children(tags),
            );

        v_flex()
            .mx_1()
            .rounded_md()
            .when(selected, |element| {
                element.bg(colors.ghost_element_selected)
            })
            .child(profile_header)
            .when(selected && snapshot.session.is_connected(), |element| {
                element.child(self.render_database_list(profile, cx))
            })
            .into_any_element()
    }

    fn render_database_list(
        &self,
        profile: &SharedConnectionProfile,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.state.databases(&profile.id) {
            None => h_flex()
                .pl(px(17.0))
                .pt_1()
                .child(Label::new("等待加载数据库…").size(LabelSize::XSmall))
                .into_any_element(),
            Some(DatabaseListState::Loading { .. }) => h_flex()
                .pl(px(17.0))
                .pt_1()
                .child(Label::new("正在加载数据库…").size(LabelSize::XSmall))
                .into_any_element(),
            Some(DatabaseListState::Failed { error, .. }) => {
                let connection_id = profile.id.clone();
                v_flex()
                    .pl(px(17.0))
                    .pt_1()
                    .gap_1()
                    .child(
                        Label::new(error.clone())
                            .size(LabelSize::XSmall)
                            .color(Color::Error)
                            .line_clamp(2),
                    )
                    .child(
                        Button::new(format!("retry-databases-{connection_id}"), "重试")
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(move |panel, event, window, cx| {
                                panel.retry_databases(connection_id.clone(), event, window, cx);
                            })),
                    )
                    .into_any_element()
            }
            Some(DatabaseListState::Ready { databases, .. }) if databases.is_empty() => h_flex()
                .pl(px(17.0))
                .pt_1()
                .child(Label::new("未发现数据库").size(LabelSize::XSmall))
                .into_any_element(),
            Some(DatabaseListState::Ready { databases, .. }) => v_flex()
                .pl(px(17.0))
                .pt_1()
                .gap_0p5()
                .children(databases.iter().map(|database| {
                    h_flex()
                        .min_w_0()
                        .gap_1p5()
                        .child(div().size(px(4.0)).rounded_full().bg(rgb(0xa1a1aa)))
                        .child(
                            Label::new(database.clone())
                                .size(LabelSize::XSmall)
                                .truncate(),
                        )
                }))
                .into_any_element(),
        }
    }

    fn render_profiles(
        &self,
        snapshot: &ConnectionWorkspaceSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if snapshot.profiles.is_empty() {
            return v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .gap_1()
                .p_4()
                .child(Label::new("暂无连接").size(LabelSize::Small))
                .child(Label::new("创建一个连接配置开始工作").size(LabelSize::XSmall))
                .child(
                    Button::new("create-first-connection-profile", "新建连接")
                        .size(ButtonSize::Compact)
                        .on_click(cx.listener(Self::create_profile)),
                )
                .into_any_element();
        }

        let mut rows = Vec::new();
        for group in grouped_profiles(snapshot) {
            rows.push(
                h_flex()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .justify_between()
                    .child(
                        Label::new(group.name.unwrap_or("未分组"))
                            .size(LabelSize::XSmall)
                            .weight(FontWeight::SEMIBOLD),
                    )
                    .child(Label::new(group.profiles.len().to_string()).size(LabelSize::XSmall))
                    .into_any_element(),
            );
            rows.extend(
                group
                    .profiles
                    .into_iter()
                    .map(|profile| self.render_profile(profile, cx)),
            );
        }

        v_flex()
            .id("connection-profiles-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pb_2()
            .children(rows)
            .into_any_element()
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

    fn render_selected_actions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected_profile()?;
        let connection_id = selected.profile.id.as_str();
        let operation = self.state.operation(connection_id);
        let busy = operation.is_some() || self.actions_blocked();
        let mcp_in_use = selected
            .mcp_usage
            .as_ref()
            .is_some_and(|usage| usage.mcp_in_use);
        let connected = selected.session.is_connected();
        let action_label = match operation {
            Some(ProfileOperationKind::Connecting) => "连接中",
            Some(ProfileOperationKind::Disconnecting) => "断开中",
            Some(ProfileOperationKind::Deleting) => "删除中",
            None if connected => "断开",
            None => "连接",
        };

        Some(
            v_flex()
                .flex_none()
                .gap_2()
                .p_2()
                .border_t_1()
                .border_color(cx.theme().colors().border)
                .child(
                    h_flex()
                        .min_w_0()
                        .gap_2()
                        .child(
                            Label::new(selected.profile.name.clone())
                                .size(LabelSize::XSmall)
                                .weight(FontWeight::MEDIUM)
                                .truncate()
                                .flex_1(),
                        )
                        .child(
                            Label::new(if connected {
                                "Database Session"
                            } else {
                                "Profile"
                            })
                            .size(LabelSize::XSmall),
                        ),
                )
                .when(mcp_in_use, |element| {
                    element.child(
                        Label::new("MCP 正在使用；资料暂时只读")
                            .size(LabelSize::XSmall)
                            .color(Color::Warning),
                    )
                })
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("selected-connection-session-action", action_label)
                                .size(ButtonSize::Compact)
                                .style(if connected {
                                    ButtonStyle::Outlined
                                } else {
                                    ButtonStyle::Filled
                                })
                                .loading(matches!(
                                    operation,
                                    Some(
                                        ProfileOperationKind::Connecting
                                            | ProfileOperationKind::Disconnecting
                                    )
                                ))
                                .disabled(busy)
                                .when_else(
                                    connected,
                                    |button| {
                                        button.on_click(cx.listener(Self::disconnect_selected))
                                    },
                                    |button| button.on_click(cx.listener(Self::connect_selected)),
                                ),
                        )
                        .child(
                            Button::new("edit-selected-connection-profile", "编辑")
                                .size(ButtonSize::Compact)
                                .disabled(busy || mcp_in_use)
                                .on_click(cx.listener(Self::edit_selected)),
                        )
                        .child(
                            Button::new("delete-selected-connection-profile", "删除")
                                .size(ButtonSize::Compact)
                                .color(Color::Error)
                                .loading(operation == Some(ProfileOperationKind::Deleting))
                                .disabled(busy || mcp_in_use)
                                .on_click(cx.listener(Self::confirm_delete_selected)),
                        ),
                )
                .into_any_element(),
        )
    }
}

impl Render for ConnectionProfilesPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().colors().border;
        let panel_background = cx.theme().colors().panel_background;
        let content = match (self.state.snapshot(), self.state.error()) {
            (None, Some(error)) => self.render_initial_error(error, cx),
            (None, None) => v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .child(Label::new("正在加载连接…").size(LabelSize::Small))
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
            .w(SIDEBAR_WIDTH)
            .h_full()
            .flex_none()
            .min_h_0()
            .border_r_1()
            .border_color(border)
            .bg(panel_background)
            .child(
                h_flex()
                    .h(px(40.0))
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        Label::new("连接")
                            .size(LabelSize::Small)
                            .weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("create-connection-profile", "新建")
                                    .size(ButtonSize::Compact)
                                    .disabled(self.actions_blocked())
                                    .on_click(cx.listener(Self::create_profile)),
                            )
                            .child(
                                Button::new("refresh-connection-profiles", "刷新")
                                    .size(ButtonSize::Compact)
                                    .loading(self.state.is_refreshing())
                                    .disabled(self.state.is_refreshing())
                                    .on_click(cx.listener(Self::refresh)),
                            ),
                    ),
            )
            .child(content)
            .when_some(self.notice.as_ref(), |element, notice| {
                element.child(self.render_notice(notice, cx))
            })
            .when_some(self.render_selected_actions(cx), |element, actions| {
                element.child(actions)
            })
    }
}
