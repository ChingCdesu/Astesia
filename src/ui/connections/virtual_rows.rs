use super::catalog_tree::{tree_row, CatalogTableKey};
use super::*;
use crate::application::connection_workspace::DatabaseListState;
use crate::application::DatabaseObjectKind;
use std::rc::Rc;

type RowRenderer =
    dyn Fn(&ConnectionProfilesPanel, &mut Context<ConnectionProfilesPanel>) -> AnyElement;

pub(super) struct SidebarRow {
    key: String,
    depth: usize,
    highlight: Option<bool>,
    render: Box<RowRenderer>,
}

impl SidebarRow {
    fn new(
        key: String,
        depth: usize,
        render: impl Fn(&ConnectionProfilesPanel, &mut Context<ConnectionProfilesPanel>) -> AnyElement
            + 'static,
    ) -> Self {
        Self {
            key,
            depth,
            highlight: None,
            render: Box::new(render),
        }
    }

    fn highlight(mut self, selected: bool) -> Self {
        self.highlight = Some(selected);
        self
    }
}

impl ConnectionProfilesPanel {
    pub(super) fn render_virtual_profiles(
        &self,
        snapshot: &ConnectionWorkspaceSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cached = self.sidebar_rows_cache.borrow().clone();
        let rows = if let Some(rows) = cached {
            rows
        } else {
            let rows = Rc::new(self.sidebar_rows(snapshot));
            let keys = rows.iter().map(|row| row.key.clone()).collect::<Vec<_>>();
            let old_top = self.sidebar_list.logical_scroll_top();
            let old_keys = self.sidebar_row_keys.borrow();
            let positions = keys
                .iter()
                .enumerate()
                .map(|(index, key)| (key, index))
                .collect::<std::collections::HashMap<_, _>>();
            let anchor = old_keys.get(..=old_top.item_ix).and_then(|prefix| {
                prefix
                    .iter()
                    .rev()
                    .find_map(|key| positions.get(key).copied())
            });
            let same_row = old_keys
                .get(old_top.item_ix)
                .is_some_and(|key| positions.contains_key(key));
            let prefix = old_keys
                .iter()
                .zip(&keys)
                .take_while(|(old, new)| old == new)
                .count();
            let suffix = old_keys[prefix..]
                .iter()
                .rev()
                .zip(keys[prefix..].iter().rev())
                .take_while(|(old, new)| old == new)
                .count();
            let old_end = old_keys.len() - suffix;
            let new_end = keys.len() - suffix;
            if prefix != old_end || prefix != new_end {
                self.sidebar_list.splice(prefix..old_end, new_end - prefix);
                self.sidebar_list.scroll_to(gpui_kit::ListOffset {
                    item_ix: anchor.unwrap_or(0),
                    offset_in_item: if same_row {
                        old_top.offset_in_item
                    } else {
                        px(0.0)
                    },
                });
            }
            drop(old_keys);
            *self.sidebar_row_keys.borrow_mut() = keys;
            *self.sidebar_rows_cache.borrow_mut() = Some(rows.clone());
            rows
        };
        let panel = cx.entity().downgrade();
        // Errors and Redis search share the tree scroll position but have variable heights.
        gpui_kit::list(self.sidebar_list.clone(), move |index, _, cx| {
            panel
                .update(cx, |panel, cx| {
                    #[cfg(test)]
                    panel.sidebar_rendered_rows.borrow_mut().push(index);
                    let row = &rows[index];
                    let clicked_key = row.key.clone();
                    let selected = panel
                        .selected_sidebar_row
                        .as_ref()
                        .map(|key| key == &row.key);
                    let outlined = row
                        .highlight
                        .is_some_and(|fallback| selected.unwrap_or(fallback));
                    div()
                        .id(row.key.clone())
                        .relative()
                        .w_full()
                        .when_some(row.highlight, |element, contextual_selection| {
                            let selected = selected.unwrap_or(contextual_selection);
                            let colors = cx.theme().colors();
                            if selected {
                                element.bg(colors.ghost_element_selected)
                            } else {
                                element.hover(|element| element.bg(colors.ghost_element_hover))
                            }
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |panel, _, _, cx| {
                                panel.selected_sidebar_row = Some(clicked_key.clone());
                                cx.notify();
                            }),
                        )
                        .pl(px(row.depth as f32 * 20.0))
                        .child((row.render)(panel, cx))
                        .when(outlined, |element| {
                            element.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .border_1()
                                    .border_color(cx.theme().colors().border_focused),
                            )
                        })
                        .into_any_element()
                })
                .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full()
        .into_any_element()
    }

    fn sidebar_rows(&self, snapshot: &ConnectionWorkspaceSnapshot) -> Vec<SidebarRow> {
        #[cfg(test)]
        self.sidebar_model_builds
            .set(self.sidebar_model_builds.get() + 1);
        let mut rows = Vec::new();
        for group in super::presentation::grouped_profiles(snapshot) {
            let key = group.name.map(str::to_owned);
            let collapsed = self.collapsed_groups.contains(&key);
            let count = group.profiles.len();
            rows.push(SidebarRow::new(
                format!("group-{key:?}"),
                0,
                move |panel, cx| {
                    let language = panel.settings.read(cx).language();
                    let name = key
                        .clone()
                        .unwrap_or_else(|| text(language, "未分组", "Ungrouped").to_string());
                    let action_key = key.clone();
                    crate::ui::components::ListItem::new("profile-group")
                        .w_full()
                        .inset(false)
                        .text_size(px(12.0))
                        .spacing(crate::ui::components::ListItemSpacing::Dense)
                        .start_slot(
                            Icon::new(if collapsed {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            })
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                        )
                        .child(
                            h_flex().h_6().child(
                                Label::new(name.clone())
                                    .text_size(px(10.0))
                                    .color(Color::Muted),
                            ),
                        )
                        .end_slot(
                            Label::new(count.to_string())
                                .text_size(px(10.0))
                                .color(Color::Muted),
                        )
                        .aria_role(gpui_kit::Role::Button)
                        .aria_label(format!(
                            "{name}, {}",
                            if collapsed {
                                text(language, "已折叠", "collapsed")
                            } else {
                                text(language, "已展开", "expanded")
                            }
                        ))
                        .on_click(
                            cx.listener(move |panel, event: &gpui_kit::ClickEvent, _, cx| {
                                if event.click_count() > 1 {
                                    return;
                                }
                                if !panel.collapsed_groups.remove(&action_key) {
                                    panel.collapsed_groups.insert(action_key.clone());
                                }
                                panel.notify_sidebar(cx);
                            }),
                        )
                        .into_any_element()
                },
            ));
            if collapsed {
                continue;
            }
            for profile in group.profiles {
                let saved = profile.clone();
                rows.push(
                    SidebarRow::new(
                        format!("profile-{}", profile.profile.id),
                        0,
                        move |panel, cx| panel.render_profile(&saved, cx),
                    )
                    .highlight(
                        self.selected_profile_id.as_deref() == Some(profile.profile.id.as_str())
                            && self.selected_query_target.is_none(),
                    ),
                );
                if !profile.session.is_connected() {
                    continue;
                }
                let Some(DatabaseListState::Ready { databases, .. }) =
                    self.state.databases(&profile.profile.id)
                else {
                    if matches!(
                        self.state.databases(&profile.profile.id),
                        None | Some(DatabaseListState::Loading { .. })
                    ) {
                        continue;
                    }
                    let saved = profile.clone();
                    rows.push(SidebarRow::new(
                        format!(
                            "databases-{}-{:?}",
                            profile.profile.id,
                            self.state.databases(&profile.profile.id)
                        ),
                        1,
                        move |panel, cx| panel.render_database_list(&saved, cx),
                    ));
                    continue;
                };
                if databases.is_empty() {
                    let saved = profile.clone();
                    rows.push(SidebarRow::new(
                        format!(
                            "databases-{}-{:?}",
                            profile.profile.id,
                            self.state.databases(&profile.profile.id)
                        ),
                        1,
                        move |panel, cx| panel.render_database_list(&saved, cx),
                    ));
                }
                for database in databases {
                    let target = QueryTarget {
                        connection_id: profile.profile.id.clone(),
                        connection_name: profile.profile.name.clone(),
                        database: database.clone(),
                        db_type: profile.profile.db_type,
                        session_generation: profile
                            .session
                            .generation
                            .expect("connected generation"),
                    };
                    let expanded = self.expanded_databases.contains(&(
                        target.connection_id.clone(),
                        target.session_generation,
                        database.clone(),
                    ));
                    let control = databases
                        .iter()
                        .find(|candidate| *candidate != database)
                        .map(|database| {
                            let mut control = target.clone();
                            control.database = database.clone();
                            control
                        });
                    let saved = target.clone();
                    rows.push(
                        SidebarRow::new(format!("database-{target:?}"), 0, move |panel, cx| {
                            panel.render_database_row(&saved, control.clone(), expanded, cx)
                        })
                        .highlight(
                            self.selected_query_target.as_ref() == Some(&target)
                                && self.selected_catalog_table.is_none(),
                        ),
                    );
                    if expanded {
                        self.append_catalog_rows(&target, &mut rows);
                    }
                }
            }
        }
        rows
    }

    fn render_database_row(
        &self,
        target: &QueryTarget,
        control: Option<QueryTarget>,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let clicked = target.clone();
        let keyboard = target.clone();
        let menu_target = target.clone();
        let drop_target = target.clone();
        super::catalog_tree::tree_row_loading(
            format!("database-{target:?}"),
            target.database.clone(),
            "database",
            Some(expanded),
            self.state
                .objects(target)
                .is_some_and(|state| state.is_loading())
                .then(|| text(self.settings.read(cx).language(), "正在加载…", "Loading…")),
            cx,
        )
        .key_context("QueryTargetRow")
        .aria_selected(self.selected_query_target.as_ref() == Some(target))
        .on_click(
            cx.listener(move |panel, event: &gpui_kit::ClickEvent, _, cx| {
                if event.click_count() <= 1 {
                    panel.toggle_database(clicked.clone(), cx);
                }
            }),
        )
        .on_action(cx.listener(move |panel, _: &menu::Confirm, _, cx| {
            panel.toggle_database(keyboard.clone(), cx)
        }))
        .on_drop(cx.listener(move |panel, copy: &DraggedTableCopy, _, cx| {
            panel.request_dragged_table_copy(copy, drop_target.clone(), cx)
        }))
        .on_mouse_down(
            gpui_kit::MouseButton::Right,
            cx.listener(move |panel, event: &gpui_kit::MouseDownEvent, window, cx| {
                cx.stop_propagation();
                panel.open_database_menu(
                    menu_target.clone(),
                    control.clone(),
                    event.position,
                    window,
                    cx,
                );
            }),
        )
        .into_any_element()
    }
}

mod catalog;
mod details;
