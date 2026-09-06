use super::*;

impl Render for AstesiaWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let status = self.connection_profiles.read(cx).status(cx);
        let language = self.settings.read(cx).language();
        let sidebar_visible = self.settings.read(cx).sidebar_visible();
        let active_item = self.active_item().map(WorkspaceItem::element);
        let active_tab = self.tabs.active();
        let status_indicator = if status.activity == ConnectionActivityStatus::NeedsRefresh {
            Color::Error
        } else {
            match status.session {
                ConnectionSessionStatus::Connected => Color::Success,
                ConnectionSessionStatus::Connecting
                | ConnectionSessionStatus::Disconnecting
                | ConnectionSessionStatus::Deleting => Color::Warning,
                ConnectionSessionStatus::Loading
                | ConnectionSessionStatus::NoSelection
                | ConnectionSessionStatus::Disconnected => Color::Muted,
            }
        };
        let session_label = match status.session {
            ConnectionSessionStatus::Loading => {
                text(language, "连接状态加载中", "Loading connection state")
            }
            ConnectionSessionStatus::NoSelection => {
                text(language, "未选择连接", "No connection selected")
            }
            ConnectionSessionStatus::Disconnected => text(language, "未连接", "Disconnected"),
            ConnectionSessionStatus::Connecting => text(language, "连接中", "Connecting"),
            ConnectionSessionStatus::Connected => text(language, "已连接", "Connected"),
            ConnectionSessionStatus::Disconnecting => text(language, "断开中", "Disconnecting"),
            ConnectionSessionStatus::Deleting => text(language, "删除中", "Deleting"),
        };
        let activity_label = match status.activity {
            ConnectionActivityStatus::Loading => text(language, "正在加载", "Loading"),
            ConnectionActivityStatus::Refreshing => text(language, "正在刷新", "Refreshing"),
            ConnectionActivityStatus::LoadingDatabases => {
                text(language, "正在加载数据库", "Loading databases")
            }
            ConnectionActivityStatus::LoadingObjects => {
                text(language, "正在加载对象", "Loading objects")
            }
            ConnectionActivityStatus::Working => text(language, "正在处理", "Working"),
            ConnectionActivityStatus::NeedsRefresh => {
                text(language, "需要刷新", "Refresh required")
            }
            ConnectionActivityStatus::Ready => text(language, "就绪", "Ready"),
        };

        let content = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .when(active_tab.is_some(), |pane| pane.child(self.tab_bar(cx)))
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .min_w_0()
                    .bg(colors.editor_background)
                    .child(active_item.unwrap_or_else(|| {
                        v_flex()
                            .size_full()
                            .items_center()
                            .justify_center()
                            .gap(DynamicSpacing::Base08.rems(cx))
                            .child(
                                Label::new(text(language, "尚未打开任何内容", "No open tabs"))
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new(text(
                                    language,
                                    "从左侧选择连接，打开查询或数据表",
                                    "Select a connection on the left to open a query or table",
                                ))
                                .size(LabelSize::Small)
                                .color(Color::Placeholder),
                            )
                            .into_any_element()
                    })),
            );
        let content = sidebar_layout(
            self.connection_profiles.clone(),
            content,
            sidebar_visible,
            &self.sidebar_resize,
        );

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("AstesiaWorkspace")
            .on_action(cx.listener(Self::toggle_command_palette))
            .on_action(cx.listener(Self::new_query_action))
            .on_action(cx.listener(Self::close_active_tab_action))
            .on_action(cx.listener(Self::next_tab_action))
            .on_action(cx.listener(Self::previous_tab_action))
            .on_action(cx.listener(Self::toggle_sidebar_action))
            .on_action(cx.listener(Self::refresh_connections_action))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(content),
            )
            .child(
                h_flex()
                    .h(px(32.0))
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .border_t_1()
                    .border_color(colors.border)
                    .bg(colors.status_bar_background)
                    .child(
                        h_flex()
                            .id("workspace-session-status")
                            .min_w_0()
                            .gap(DynamicSpacing::Base08.rems(cx))
                            .aria_label(format!(
                                "{} · {session_label} · {activity_label}",
                                status.summary
                            ))
                            .tooltip(Tooltip::text(format!("{session_label} · {activity_label}")))
                            .when(
                                status.session != ConnectionSessionStatus::NoSelection,
                                |row| row.child(Indicator::dot().color(status_indicator)),
                            )
                            .child(
                                Label::new(status.summary)
                                    .size(LabelSize::XSmall)
                                    .truncate(),
                            )
                            .when(status.activity != ConnectionActivityStatus::Ready, |row| {
                                row.child(Label::new(activity_label).size(LabelSize::XSmall))
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        IconButton::new("open-task-center", IconName::ListTodo)
                            .icon_size(IconSize::Small)
                            .aria_label(text(language, "任务", "Tasks"))
                            .tooltip(Tooltip::text(text(language, "任务", "Tasks")))
                            .on_click(cx.listener(|workspace, _, window, cx| {
                                workspace.open_task_center(window, cx);
                            })),
                    )
                    .child(
                        IconButton::new("open-mcp-service", IconName::Server)
                            .icon_size(IconSize::Small)
                            .aria_label(text(language, "MCP 服务", "MCP Service"))
                            .tooltip(Tooltip::text(text(language, "MCP 服务", "MCP Service")))
                            .on_click(cx.listener(|workspace, _, window, cx| {
                                workspace.open_mcp_service(window, cx);
                            })),
                    )
                    .child(
                        IconButton::new("open-command-palette", IconName::Command)
                            .icon_size(IconSize::Small)
                            .aria_label(text(language, "命令", "Commands"))
                            .tooltip(move |_, cx| {
                                Tooltip::for_action(
                                    text(language, "命令", "Commands"),
                                    &ToggleCommandPalette,
                                    cx,
                                )
                            })
                            .on_click(cx.listener(Self::open_palette_click)),
                    )
                    .child(self.settings_menu(cx))
                    .child(
                        Label::new(format!("Astesia v{}", env!("CARGO_PKG_VERSION")))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(self.modal_layer.clone())
            .child(self.notifications.clone())
    }
}

fn sidebar_layout(
    sidebar: impl IntoElement,
    content: impl IntoElement,
    visible: bool,
    state: &Entity<gpui_kit::component::ResizableState>,
) -> AnyElement {
    if !visible {
        return content.into_any_element();
    }
    gpui_kit::component::h_resizable("workspace-sidebar")
        .with_state(state)
        .child(
            gpui_kit::component::resizable_panel()
                .size(super::super::connections::SIDEBAR_WIDTH)
                .size_range(px(200.0)..px(560.0))
                .flex_none()
                .child(sidebar),
        )
        .child(
            gpui_kit::component::resizable_panel()
                .size_range(px(400.0)..Pixels::MAX)
                .child(content),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SidebarLayoutTest {
        state: Entity<gpui_kit::component::ResizableState>,
        visible: bool,
    }

    impl Render for SidebarLayoutTest {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(960.0)).h(px(600.0)).child(sidebar_layout(
                div().size_full().debug_selector(|| "sidebar".into()),
                div().size_full().debug_selector(|| "content".into()),
                self.visible,
                &self.state,
            ))
        }
    }

    #[gpui_kit::test]
    fn sidebar_drag_clamps_width_and_preserves_it_when_hidden(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| crate::ui::initialize_editor_runtime(ThemePreference::Light, cx));
        let state = cx.new(|_| gpui_kit::component::ResizableState::default());
        let (view, cx) = cx.add_window_view({
            let state = state.clone();
            move |_, _| SidebarLayoutTest {
                state,
                visible: true,
            }
        });
        for _ in 0..2 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        assert_eq!(cx.debug_bounds("sidebar").unwrap().size.width, px(272.0));
        for (requested, expected) in [(360.0, 360.0), (900.0, 560.0), (100.0, 200.0)] {
            let boundary = cx.debug_bounds("content").unwrap().left();
            cx.simulate_mouse_down(
                point(boundary - px(2.0), px(50.0)),
                MouseButton::Left,
                gpui_kit::Modifiers::default(),
            );
            for x in [boundary + px(10.0), px(requested)] {
                cx.simulate_mouse_move(
                    point(x, px(50.0)),
                    Some(MouseButton::Left),
                    gpui_kit::Modifiers::default(),
                );
            }
            cx.simulate_mouse_up(
                point(px(requested), px(50.0)),
                MouseButton::Left,
                gpui_kit::Modifiers::default(),
            );
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(cx.debug_bounds("sidebar").unwrap().size.width, px(expected));
            assert!(cx.debug_bounds("content").unwrap().size.width >= px(400.0));
        }
        for visible in [false, true] {
            view.update(cx, |view, cx| {
                view.visible = visible;
                cx.notify();
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(
                cx.debug_bounds("content").unwrap().size.width,
                px(if visible { 760.0 } else { 960.0 })
            );
        }
    }
}
