use super::*;
use gpui_kit::component::tab::{Tab, TabBar};

impl AstesiaWorkspace {
    pub(super) fn tab_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let language = self.settings.read(cx).language();
        let active_index = self
            .tabs
            .tabs()
            .iter()
            .position(|id| Some(*id) == self.tabs.active());
        TabBar::new("workspace-tabs")
            .track_scroll(&self.tab_scroll_handle)
            .selected_index(active_index.unwrap_or(0))
            .children(self.tabs.tabs().iter().enumerate().map(|(index, id)| {
                let id = *id;
                let tab = self
                    .workspace_tabs
                    .iter()
                    .find(|tab| tab.id == id)
                    .expect("workspace tab model and views must agree");
                let fallback = format!("{} {}", text(language, "查询", "Query"), index + 1);
                let label = tab.item.label(&fallback, cx);
                let dirty = tab.item.has_unsaved_changes(cx);
                let accessible_label = if dirty {
                    format!(
                        "{label}, {}",
                        text(language, "有未保存的更改", "has unsaved changes")
                    )
                } else {
                    label.clone()
                };
                Tab::new()
                    .label(label.clone())
                    .aria_label(accessible_label)
                    .on_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.activate_tab(id, window, cx);
                    }))
                    .when(dirty, |tab| {
                        tab.prefix(Indicator::dot().color(Color::Warning))
                    })
                    .suffix(
                        IconButton::new(format!("close-workspace-tab-{index}"), IconName::Close)
                            .icon_size(IconSize::XSmall)
                            .aria_label(text(language, "关闭标签页", "Close Tab"))
                            .tooltip(Tooltip::text(text(language, "关闭标签页", "Close Tab")))
                            .on_click(cx.listener(move |workspace, _, window, cx| {
                                cx.stop_propagation();
                                workspace.close_tab(id, window, cx);
                            })),
                    )
            }))
            .suffix(
                IconButton::new("new-query-tab", IconName::Plus)
                    .icon_size(IconSize::Small)
                    .aria_label(text(language, "新建查询", "New Query"))
                    .tooltip(move |_, cx| {
                        Tooltip::for_action(
                            text(language, "新建查询", "New Query"),
                            &NewQueryTab,
                            cx,
                        )
                    })
                    .on_click(
                        cx.listener(|workspace, _, window, cx| workspace.new_query_tab(window, cx)),
                    ),
            )
    }
}
