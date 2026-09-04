use editor::Editor;
use gpui::{DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Subscription};
use workspace::ModalView;
use zed_ui::prelude::*;

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
    selected_index: usize,
    _search_observation: Subscription,
}

impl CommandPalette {
    pub(super) fn new(language: UiLanguage, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            cx.notify();
        });
        window.focus(&search.read(cx).focus_handle(cx), cx);
        Self {
            search,
            language,
            selected_index: 0,
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
    }

    fn filtered_entries(&self, cx: &App) -> Vec<CommandEntry> {
        filter_entries(self.entries(), &self.search.read(cx).text(cx))
    }

    fn select_next(&mut self, _: &menu::SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.filtered_entries(cx).len();
        if count > 0 {
            self.selected_index = (self.selected_index + 1) % count;
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
        _: &gpui::ClickEvent,
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
            .role(gpui::Role::ComboBox)
            .aria_label(text(self.language, "命令面板", "Command palette"))
            .aria_value(selected_value)
            .track_focus(&focus_handle)
            .key_context("CommandPalette")
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .w(px(560.0))
            .max_h(px(520.0))
            .rounded_lg()
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .h(px(44.0))
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(self.search.clone()),
            )
            .child(
                v_flex()
                    .id("command-palette-results")
                    .role(gpui::Role::ListBox)
                    .aria_label(text(self.language, "命令结果", "Command results"))
                    .max_h(px(460.0))
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
                        h_flex()
                            .id(format!("command-palette-entry-{index}"))
                            .role(gpui::Role::ListBoxOption)
                            .aria_label(entry.accessibility_label())
                            .aria_selected(selected)
                            .when(selected, |element| element.aria_active_descendant())
                            .tab_index(0)
                            .key_context("CommandPaletteEntry")
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .when(selected, |element| {
                                element.bg(colors.ghost_element_selected)
                            })
                            .hover(|element| element.bg(colors.ghost_element_hover))
                            .on_action(cx.listener(move |_, _: &menu::Confirm, _, cx| {
                                cx.emit(CommandSelected(command));
                            }))
                            .on_click(cx.listener(move |palette, event, window, cx| {
                                palette.choose(command, event, window, cx);
                            }))
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .flex_1()
                                    .child(Label::new(entry.title).size(LabelSize::Small))
                                    .child(
                                        Label::new(entry.category)
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    ),
                            )
                            .children(entry.shortcut.map(|shortcut| {
                                Label::new(shortcut)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                            }))
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
