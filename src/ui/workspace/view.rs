use super::*;

impl Render for AstesiaWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let status = self.connection_profiles.read(cx).status(cx);
        let language = self.settings.read(cx).language();
        let theme = self.settings.read(cx).theme();
        let sidebar_visible = self.settings.read(cx).sidebar_visible();
        let active_item = self.active_item().element();
        let active_tab = self.tabs.active();
        let status_indicator = if status.activity == ConnectionActivityStatus::NeedsRefresh {
            rgb(0xef4444)
        } else {
            match status.session {
                ConnectionSessionStatus::Connected => rgb(0x22c55e),
                ConnectionSessionStatus::Connecting
                | ConnectionSessionStatus::Disconnecting
                | ConnectionSessionStatus::Deleting => rgb(0xeab308),
                ConnectionSessionStatus::Loading
                | ConnectionSessionStatus::NoSelection
                | ConnectionSessionStatus::Disconnected => rgb(0xa1a1aa),
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

        v_flex()
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
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .overflow_hidden()
                    .when(sidebar_visible, |element| {
                        element.child(self.connection_profiles.clone())
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .min_h_0()
                            .child(
                                h_flex()
                                    .h(px(40.0))
                                    .flex_none()
                                    .items_end()
                                    .px_1()
                                    .border_b_1()
                                    .border_color(colors.border)
                                    .bg(colors.tab_bar_background)
                                    .gap_1()
                                    .child(
                                        h_flex()
                                            .id("workspace-tabs-scroll")
                                            .h_full()
                                            .flex_1()
                                            .min_w_0()
                                            .items_end()
                                            .gap_1()
                                            .overflow_x_scroll()
                                            .children(self.tabs.tabs().iter().enumerate().map(
                                                |(index, id)| {
                                                    let id = *id;
                                                    let is_active = id == active_tab;
                                                    let tab = self
                                                        .workspace_tabs
                                                        .iter()
                                                        .find(|tab| tab.id == id)
                                                        .expect(
                                                            "workspace tab model and views must agree",
                                                        );
                                                    let fallback = format!(
                                                        "{} {}",
                                                        text(language, "查询", "Query"),
                                                        index + 1
                                                    );
                                                    let label = tab.item.label(&fallback, cx);
                                                    let dirty = tab.item.has_unsaved_changes(cx);
                                                    let accessibility_label = if dirty {
                                                        format!(
                                                            "{label}, {}",
                                                            text(
                                                                language,
                                                                "有未保存的更改",
                                                                "has unsaved changes"
                                                            )
                                                        )
                                                    } else {
                                                        label.clone()
                                                    };
                                                    h_flex()
                                                        .id(format!("workspace-tab-{index}"))
                                                        .role(gpui::Role::Button)
                                                        .tab_index(0)
                                                        .key_context("WorkspaceTabRow")
                                                        .aria_label(accessibility_label)
                                                        .aria_toggled(if is_active {
                                                            gpui::Toggled::True
                                                        } else {
                                                            gpui::Toggled::False
                                                        })
                                                        .h(px(36.0))
                                                        .min_w(px(112.0))
                                                        .max_w(px(260.0))
                                                        .px_2()
                                                        .gap_1()
                                                        .items_center()
                                                        .rounded_t_md()
                                                        .border_1()
                                                        .border_b_0()
                                                        .border_color(colors.border)
                                                        .bg(if is_active {
                                                            colors.tab_active_background
                                                        } else {
                                                            colors.tab_inactive_background
                                                        })
                                                        .cursor_pointer()
                                                        .on_action(cx.listener(
                                                            move |workspace,
                                                                  _: &menu::Confirm,
                                                                  window,
                                                                  cx| {
                                                                workspace
                                                                    .activate_tab(id, window, cx);
                                                            },
                                                        ))
                                                        .on_click(cx.listener(
                                                            move |workspace, _, window, cx| {
                                                                workspace
                                                                    .activate_tab(id, window, cx);
                                                            },
                                                        ))
                                                        .when(dirty, |element| {
                                                            element.child(
                                                                Indicator::dot()
                                                                    .color(Color::Warning),
                                                            )
                                                        })
                                                        .child(
                                                            Label::new(label)
                                                                .size(LabelSize::Small)
                                                                .weight(gpui::FontWeight::MEDIUM)
                                                                .truncate()
                                                                .flex_1(),
                                                        )
                                                        .when(
                                                            self.tabs.tabs().len() > 1,
                                                            |element| {
                                                                element.child(
                                                                    IconButton::new(
                                                                        format!(
                                                                            "close-workspace-tab-{index}"
                                                                        ),
                                                                        IconName::Close,
                                                                    )
                                                                    .icon_size(IconSize::XSmall)
                                                                    .on_click(cx.listener(
                                                                        move |workspace,
                                                                              _,
                                                                              window,
                                                                              cx| {
                                                                            workspace.close_tab(
                                                                                id, window, cx,
                                                                            );
                                                                        },
                                                                    )),
                                                                )
                                                            },
                                                        )
                                                },
                                            )),
                                    )
                                    .child(
                                        IconButton::new("new-query-tab", IconName::Plus)
                                            .icon_size(IconSize::Small)
                                            .tooltip(move |_, cx| {
                                                Tooltip::with_meta(
                                                    text(language, "新建查询", "New Query"),
                                                    None,
                                                    "⌘N",
                                                    cx,
                                                )
                                            })
                                            .on_click(cx.listener(|workspace, _, window, cx| {
                                                workspace.new_query_tab(window, cx);
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .w_full()
                                    .min_h_0()
                                    .min_w_0()
                                    .bg(colors.background)
                                    .child(active_item),
                            ),
                    ),
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
                    .child(div().size(px(8.0)).rounded_full().bg(status_indicator))
                    .child(Label::new(status.summary).size(LabelSize::XSmall))
                    .child(div().flex_1())
                    .child(
                        Button::new("open-task-center", text(language, "任务", "Tasks"))
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Transparent)
                            .on_click(cx.listener(|workspace, _, window, cx| {
                                workspace.open_task_center(window, cx);
                            })),
                    )
                    .child(
                        Button::new("open-mcp-service", "MCP")
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Transparent)
                            .on_click(cx.listener(|workspace, _, window, cx| {
                                workspace.open_mcp_service(window, cx);
                            })),
                    )
                    .child(
                        Button::new("open-command-palette", text(language, "命令", "Commands"))
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Transparent)
                            .key_binding(zed_ui::KeyBinding::for_action(&ToggleCommandPalette, cx))
                            .on_click(cx.listener(Self::open_palette_click)),
                    )
                    .child(
                        Label::new(format!("Astesia v{}", env!("CARGO_PKG_VERSION")))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Button::new("cycle-language", self.settings.read(cx).language().code())
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Transparent)
                            .on_click(cx.listener(Self::cycle_language)),
                    )
                    .child(
                        Button::new("cycle-theme", theme_label(language, theme))
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Transparent)
                            .on_click(cx.listener(Self::cycle_theme)),
                    )
                    .child(Label::new(session_label).size(LabelSize::XSmall))
                    .child(Label::new(activity_label).size(LabelSize::XSmall)),
            )
            .child(self.modal_layer.clone())
            .child(self.notifications.clone())
    }
}
