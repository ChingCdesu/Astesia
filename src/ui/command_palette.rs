use crate::ui::components::prelude::*;
use crate::ui::modal::ModalView;
use crate::ui::text_editor::Editor;
use gpui_kit::{DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Subscription};

use crate::platform::{ThemePreference, UiLanguage};

use super::localization::{text, theme_label};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkspaceCommand {
    NewQuery,
    CloseActiveTab,
    NextTab,
    PreviousTab,
    ToggleSidebar,
    RefreshConnections,
    ConnectProfile,
    DisconnectProfile,
    EditProfile,
    DeleteProfile,
    RestartApplication,
    SetTheme(ThemePreference),
    SetLanguage(UiLanguage),
}

#[derive(Clone, Debug)]
pub(super) struct CommandSelected(pub(super) WorkspaceCommand);

impl EventEmitter<CommandSelected> for CommandPalette {}
impl EventEmitter<DismissEvent> for CommandPalette {}

#[derive(Clone)]
struct CommandEntry {
    command: WorkspaceCommand,
    title: &'static str,
    category: &'static str,
    shortcut: Option<&'static str>,
    keywords: &'static str,
}

impl CommandEntry {
    fn accessibility_label(&self) -> String {
        format!("{} · {}", self.title, self.category)
    }
}

pub(super) struct CommandPalette {
    search: Entity<Editor>,
    language: UiLanguage,
    profile_actions: super::connections::ProfileActions,
    selected_index: usize,
    scroll_handle: gpui_kit::ScrollHandle,
    _search_observation: Subscription,
}

impl CommandPalette {
    pub(super) fn new(
        language: UiLanguage,
        profile_actions: super::connections::ProfileActions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text(
                text(language, "输入命令或搜索…", "Type a command or search…"),
                window,
                cx,
            );
            editor
        });
        let search_observation = cx.observe(&search, |palette, _, cx| {
            palette.selected_index = 0;
            palette.scroll_handle.scroll_to_item(0);
            cx.notify();
        });
        window.focus(&search.read(cx).focus_handle(cx), cx);
        Self {
            search,
            language,
            profile_actions,
            selected_index: 0,
            scroll_handle: gpui_kit::ScrollHandle::new(),
            _search_observation: search_observation,
        }
    }

    fn entries(&self) -> Vec<CommandEntry> {
        let language = self.language;
        vec![
            CommandEntry {
                command: WorkspaceCommand::NewQuery,
                title: text(language, "新建查询", "New Query"),
                category: text(language, "工作区", "Workspace"),
                shortcut: Some("⌘N"),
                keywords: "new query 新建 查询",
            },
            CommandEntry {
                command: WorkspaceCommand::ToggleSidebar,
                title: text(language, "显示或隐藏侧栏", "Toggle Sidebar"),
                category: text(language, "工作区", "Workspace"),
                shortcut: Some("⌘B"),
                keywords: "toggle sidebar 侧栏",
            },
            CommandEntry {
                command: WorkspaceCommand::CloseActiveTab,
                title: text(language, "关闭当前标签页", "Close Active Tab"),
                category: text(language, "标签页", "Tabs"),
                shortcut: Some("⌘W"),
                keywords: "close tab 关闭 标签页",
            },
            CommandEntry {
                command: WorkspaceCommand::NextTab,
                title: text(language, "下一个标签页", "Next Tab"),
                category: text(language, "标签页", "Tabs"),
                shortcut: Some("⌃Tab"),
                keywords: "next tab 下一个 标签页",
            },
            CommandEntry {
                command: WorkspaceCommand::PreviousTab,
                title: text(language, "上一个标签页", "Previous Tab"),
                category: text(language, "标签页", "Tabs"),
                shortcut: Some("⌃⇧Tab"),
                keywords: "previous tab 上一个 标签页",
            },
            CommandEntry {
                command: WorkspaceCommand::RefreshConnections,
                title: text(language, "刷新连接", "Refresh Connections"),
                category: text(language, "连接", "Connections"),
                shortcut: Some("⌘R"),
                keywords: "refresh connections 刷新 连接",
            },
            CommandEntry {
                command: WorkspaceCommand::ConnectProfile,
                title: text(language, "连接选中配置", "Connect Selected Profile"),
                category: text(language, "连接", "Connections"),
                shortcut: None,
                keywords: "Connect Selected Profile 连接选中配置",
            },
            CommandEntry {
                command: WorkspaceCommand::DisconnectProfile,
                title: text(language, "断开选中连接", "Disconnect Selected Profile"),
                category: text(language, "连接", "Connections"),
                shortcut: None,
                keywords: "Disconnect Selected Profile 断开选中连接",
            },
            CommandEntry {
                command: WorkspaceCommand::EditProfile,
                title: text(language, "编辑选中连接…", "Edit Selected Profile…"),
                category: text(language, "连接", "Connections"),
                shortcut: None,
                keywords: "Edit Selected Profile… 编辑选中连接…",
            },
            CommandEntry {
                command: WorkspaceCommand::DeleteProfile,
                title: text(language, "删除选中连接…", "Delete Selected Profile…"),
                category: text(language, "连接", "Connections"),
                shortcut: None,
                keywords: "Delete Selected Profile… 删除选中连接…",
            },
            CommandEntry {
                command: WorkspaceCommand::RestartApplication,
                title: text(language, "重启 Astesia", "Restart Astesia"),
                category: text(language, "应用", "Application"),
                shortcut: None,
                keywords: "restart relaunch application 重启 应用",
            },
            CommandEntry {
                command: WorkspaceCommand::SetTheme(ThemePreference::System),
                title: theme_label(language, ThemePreference::System),
                category: text(language, "主题", "Theme"),
                shortcut: None,
                keywords: "theme system 主题 系统",
            },
            CommandEntry {
                command: WorkspaceCommand::SetTheme(ThemePreference::Light),
                title: theme_label(language, ThemePreference::Light),
                category: text(language, "主题", "Theme"),
                shortcut: None,
                keywords: "theme light 主题 浅色",
            },
            CommandEntry {
                command: WorkspaceCommand::SetTheme(ThemePreference::Dark),
                title: theme_label(language, ThemePreference::Dark),
                category: text(language, "主题", "Theme"),
                shortcut: None,
                keywords: "theme dark 主题 深色",
            },
            CommandEntry {
                command: WorkspaceCommand::SetLanguage(UiLanguage::Chinese),
                title: "中文",
                category: text(language, "语言", "Language"),
                shortcut: None,
                keywords: "language chinese 中文 语言",
            },
            CommandEntry {
                command: WorkspaceCommand::SetLanguage(UiLanguage::English),
                title: "English",
                category: text(language, "语言", "Language"),
                shortcut: None,
                keywords: "language english 英文 语言",
            },
        ]
        .into_iter()
        .filter(|entry| match entry.command {
            WorkspaceCommand::ConnectProfile => self.profile_actions.connect,
            WorkspaceCommand::DisconnectProfile => self.profile_actions.disconnect,
            WorkspaceCommand::EditProfile => self.profile_actions.edit,
            WorkspaceCommand::DeleteProfile => self.profile_actions.delete,
            _ => true,
        })
        .collect()
    }

    fn filtered_entries(&self, cx: &App) -> Vec<CommandEntry> {
        filter_entries(self.entries(), &self.search.read(cx).text(cx))
    }

    fn select_next(&mut self, _: &menu::SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.filtered_entries(cx).len();
        if count > 0 {
            self.selected_index = (self.selected_index + 1) % count;
            self.scroll_handle.scroll_to_item(self.selected_index);
            cx.notify();
        }
    }

    fn select_previous(
        &mut self,
        _: &menu::SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.filtered_entries(cx).len();
        if count > 0 {
            self.selected_index = (self.selected_index + count - 1) % count;
            self.scroll_handle.scroll_to_item(self.selected_index);
            cx.notify();
        }
    }

    fn confirm(&mut self, _: &menu::Confirm, _: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.filtered_entries(cx).get(self.selected_index).cloned() else {
            return;
        };
        cx.emit(CommandSelected(entry.command));
    }

    fn cancel(&mut self, _: &menu::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn choose(
        &mut self,
        command: WorkspaceCommand,
        _: &gpui_kit::ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(CommandSelected(command));
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let entries = self.filtered_entries(cx);
        let focus_handle = self.focus_handle(cx);
        let no_matches = text(self.language, "没有匹配的命令", "No matching commands");
        let selected_value = entries
            .get(self.selected_index)
            .map(CommandEntry::accessibility_label)
            .unwrap_or_else(|| no_matches.to_string());
        v_flex()
            .id("command-palette")
            .role(gpui_kit::Role::ComboBox)
            .aria_label(text(self.language, "命令面板", "Command palette"))
            .aria_value(selected_value)
            .track_focus(&focus_handle)
            .key_context("CommandPalette")
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .w(px(560.0))
            .h(px(420.0))
            .rounded_lg()
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .shadow(crate::ui::components::ElevationIndex::ModalSurface.shadow(cx))
            .overflow_hidden()
            .child(
                h_flex()
                    .h(DynamicSpacing::Base48.rems(cx))
                    .flex_none()
                    .px(DynamicSpacing::Base20.rems(cx))
                    .gap(DynamicSpacing::Base08.rems(cx))
                    .border_b_1()
                    .border_color(colors.border)
                    .child(div().flex_1().min_w_0().child(self.search.clone()))
                    .child(
                        Label::new("Esc")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(
                v_flex()
                    .id("command-palette-results")
                    .role(gpui_kit::Role::ListBox)
                    .aria_label(text(self.language, "命令结果", "Command results"))
                    .flex_1()
                    .min_h_0()
                    .p(DynamicSpacing::Base08.rems(cx))
                    .gap(DynamicSpacing::Base02.rems(cx))
                    .track_scroll(&self.scroll_handle)
                    .overflow_y_scroll()
                    .when(entries.is_empty(), |element| {
                        element.child(
                            h_flex()
                                .justify_center()
                                .p_4()
                                .child(Label::new(no_matches).size(LabelSize::Small)),
                        )
                    })
                    .children(entries.into_iter().enumerate().map(|(index, entry)| {
                        let command = entry.command;
                        let selected = index == self.selected_index;
                        div()
                            .id(format!("command-palette-entry-{index}"))
                            .tab_index(0)
                            .key_context("CommandPaletteEntry")
                            .on_action(cx.listener(move |_, _: &menu::Confirm, _, cx| {
                                cx.emit(CommandSelected(command));
                            }))
                            .child(
                                crate::ui::components::ListItem::new(format!(
                                    "command-row-{index}"
                                ))
                                .inset(true)
                                .aria_role(gpui_kit::Role::ListBoxOption)
                                .aria_label(entry.accessibility_label())
                                .toggle_state(selected)
                                .when(selected, |row| row.aria_active_descendant())
                                .on_click(cx.listener(move |palette, event, window, cx| {
                                    palette.choose(command, event, window, cx);
                                }))
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .flex_1()
                                        .py(DynamicSpacing::Base08.rems(cx))
                                        .child(Label::new(entry.title).size(LabelSize::Small))
                                        .child(
                                            Label::new(entry.category)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        ),
                                )
                                .when_some(
                                    entry.shortcut,
                                    |row, shortcut| {
                                        row.end_slot(
                                            Label::new(shortcut)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        )
                                    },
                                ),
                            )
                    })),
            )
    }
}

impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }
}

impl ModalView for CommandPalette {
    fn fade_out_background(&self) -> bool {
        true
    }
}

fn filter_entries(entries: Vec<CommandEntry>, query: &str) -> Vec<CommandEntry> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return entries;
    }
    entries
        .into_iter()
        .filter(|entry| {
            entry.title.to_lowercase().contains(&query)
                || entry.category.to_lowercase().contains(&query)
                || entry.keywords.to_lowercase().contains(&query)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_filter_matches_titles_categories_and_cross_language_keywords() {
        let entries = vec![CommandEntry {
            command: WorkspaceCommand::NewQuery,
            title: "New Query",
            category: "Workspace",
            shortcut: Some("⌘N"),
            keywords: "new query 新建 查询",
        }];

        assert_eq!(filter_entries(entries.clone(), "workspace").len(), 1);
        assert_eq!(filter_entries(entries.clone(), "新建").len(), 1);
        assert!(filter_entries(entries, "theme").is_empty());
    }
}
