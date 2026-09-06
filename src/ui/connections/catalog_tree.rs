use super::*;
use crate::application::{TableStructureLoadError, TableStructureSnapshot};
mod menus;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct CatalogTableKey {
    connection: String,
    generation: u64,
    database: String,
    table: TableRef,
}

impl CatalogTableKey {
    pub(super) fn new(target: &QueryTarget, table: &TableRef) -> Self {
        Self {
            connection: target.connection_id.clone(),
            generation: target.session_generation,
            database: target.database.clone(),
            table: table.clone(),
        }
    }
    pub(super) fn belongs_to_connection(&self, connection: &str) -> bool {
        self.connection == connection
    }
    pub(super) fn matches(&self, target: &QueryTarget) -> bool {
        self.connection == target.connection_id
            && self.generation == target.session_generation
            && self.database == target.database
    }
}

pub(super) enum CatalogDetail {
    Loading(u64),
    Ready(TableStructureSnapshot),
    Failed(String),
}

impl ConnectionProfilesPanel {
    pub(super) fn toggle_schema_group(
        &mut self,
        key: (String, u64, String, String),
        cx: &mut Context<Self>,
    ) {
        self.selected_sidebar_row = Some(format!("schema-{key:?}"));
        if !self.collapsed_schemas.remove(&key) {
            self.collapsed_schemas.insert(key);
        }
        self.notify_sidebar(cx);
    }
    pub(super) fn toggle_detail_section(
        &mut self,
        key: (CatalogTableKey, u8),
        cx: &mut Context<Self>,
    ) {
        self.selected_sidebar_row = Some(format!("detail-section-{key:?}"));
        if !self.collapsed_details.remove(&key) {
            self.collapsed_details.insert(key);
        }
        self.notify_sidebar(cx);
    }
    pub(super) fn prune_catalog_details(&mut self) {
        let live = self
            .state
            .snapshot()
            .map(|snapshot| {
                snapshot
                    .profiles
                    .iter()
                    .filter_map(|profile| {
                        profile
                            .session
                            .generation
                            .map(|generation| (profile.profile.id.clone(), generation))
                    })
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        self.expanded_databases
            .retain(|(connection, generation, _)| {
                live.contains(&(connection.clone(), *generation))
            });
        if self
            .selected_catalog_table
            .as_ref()
            .is_some_and(|key| !live.contains(&(key.connection.clone(), key.generation)))
        {
            self.selected_catalog_table = None;
        }
        self.table_details
            .retain(|key, _| live.contains(&(key.connection.clone(), key.generation)));
        self.expanded_tables
            .retain(|key| live.contains(&(key.connection.clone(), key.generation)));
        self.collapsed_details
            .retain(|(key, _)| live.contains(&(key.connection.clone(), key.generation)));
        self.collapsed_schemas
            .retain(|(connection, generation, _, _)| {
                live.contains(&(connection.clone(), *generation))
            });
    }

    pub(super) fn render_sql_table(
        &self,
        target: &QueryTarget,
        table: &crate::db::TableInfo,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = CatalogTableKey::new(target, &table.reference);
        let expanded = self.expanded_tables.contains(&key);
        let loading = matches!(
            self.table_details.get(&key),
            Some(CatalogDetail::Loading(_))
        );
        let expansion_target = target.clone();
        let expansion_table = table.reference.clone();
        let browse_target = target.clone();
        let browse_table = table.reference.clone();
        let menu_target = target.clone();
        let menu_table = table.reference.clone();
        let keyboard_target = target.clone();
        let keyboard_table = table.reference.clone();
        let language = self.settings.read(cx).language();
        let dragged_table = super::engine_workflows::DraggedTableCopy {
            source: target.clone(),
            table: table.reference.clone(),
        };
        let drag_label = table.reference.to_string();
        div()
            .child(
                h_flex()
                    .min_w_0()
                    .pl(px(12.0))
                    .gap_0p5()
                    .child(
                        IconButton::new(
                            format!("expand-table-{key:?}"),
                            if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            },
                        )
                        .icon_size(IconSize::XSmall)
                        .text_color(Color::Muted.color(cx))
                        .size(ButtonSize::Compact)
                        .w(px(16.0))
                        .min_w(px(16.0))
                        .px_0()
                        .flex_shrink_0()
                        .aria_label(format!(
                            "{} {}",
                            if expanded {
                                text(language, "折叠", "Collapse")
                            } else {
                                text(language, "展开", "Expand")
                            },
                            table.reference
                        ))
                        .aria_expanded(expanded)
                        .on_click(cx.listener(
                            move |panel, event: &gpui_kit::ClickEvent, _, cx| {
                                if event.click_count() > 1 {
                                    return;
                                }
                                panel.toggle_table_details(
                                    expansion_target.clone(),
                                    expansion_table.clone(),
                                    cx,
                                )
                            },
                        )),
                    )
                    .child(
                        tree_row_with_disclosure(
                            format!("browse-sql-table-{key:?}"),
                            table.reference.name().to_string(),
                            "table",
                            CatalogDisclosure::External,
                            None,
                            loading.then(|| text(language, "正在加载…", "Loading…")),
                            cx,
                        )
                        .flex_1()
                        .min_w_0()
                        .when(
                            target.db_type.capabilities().table_copy
                                != crate::db::TableCopyMode::None,
                            |row| {
                                row.on_drag(dragged_table, move |_, _, _, cx| {
                                    cx.new(|_| super::engine_workflows::DraggedTableCopyPreview {
                                        label: drag_label.clone(),
                                    })
                                })
                            },
                        )
                        .on_action(cx.listener(move |panel, _: &menu::Confirm, _, cx| {
                            panel.request_primary_data(
                                keyboard_target.clone(),
                                keyboard_table.clone(),
                                cx,
                            )
                        }))
                        .on_click(
                            cx.listener(move |panel, event: &gpui_kit::ClickEvent, _, cx| {
                                if event.click_count() > 1 {
                                    return;
                                }
                                panel.request_primary_data(
                                    browse_target.clone(),
                                    browse_table.clone(),
                                    cx,
                                )
                            }),
                        )
                        .on_mouse_down(
                            gpui_kit::MouseButton::Right,
                            cx.listener(
                                move |panel, event: &gpui_kit::MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    panel.open_table_menu(
                                        menu_target.clone(),
                                        menu_table.clone(),
                                        event.position,
                                        window,
                                        cx,
                                    );
                                },
                            ),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn toggle_table_details(
        &mut self,
        target: QueryTarget,
        table: TableRef,
        cx: &mut Context<Self>,
    ) {
        let key = CatalogTableKey::new(&target, &table);
        self.selected_sidebar_row = Some(format!("table-{key:?}"));
        if !self.expanded_tables.remove(&key) {
            self.expanded_tables.insert(key.clone());
            if !matches!(
                self.table_details.get(&key),
                Some(CatalogDetail::Ready(_) | CatalogDetail::Loading(_))
            ) {
                self.load_table_details(target, table, cx);
            }
        }
        self.notify_sidebar(cx);
    }

    fn load_table_details(&mut self, target: QueryTarget, table: TableRef, cx: &mut Context<Self>) {
        if !self.state.query_target_is_live(&target) {
            return;
        }
        self.detail_generation = self
            .detail_generation
            .checked_add(1)
            .expect("catalog detail generation exhausted");
        let generation = self.detail_generation;
        let key = CatalogTableKey::new(&target, &table);
        self.table_details
            .insert(key.clone(), CatalogDetail::Loading(generation));
        let service = self.application.catalog().clone();
        let connection = target.connection_id.clone();
        let database = target.database.clone();
        let load = crate::ui::runtime::spawn(cx, async move {
            service
                .table_structure(&connection, &database, &table)
                .await
        });
        cx.spawn(async move |panel, cx| {
            let result = load.await.unwrap_or_else(|error| Err(TableStructureLoadError::BackgroundTask(error.to_string())));
            panel.update(cx, |panel, cx| {
                if !panel.state.query_target_is_live(&target) || !matches!(panel.table_details.get(&key), Some(CatalogDetail::Loading(current)) if *current == generation) { return; }
                panel.table_details.insert(key, match result { Ok(detail) => CatalogDetail::Ready(detail), Err(error) => CatalogDetail::Failed(error.message().to_string()) });
                panel.notify_sidebar(cx);
            }).ok();
        }).detach();
    }

    pub(super) fn refresh_table_details(&mut self, target: &QueryTarget, cx: &mut Context<Self>) {
        let tables = self
            .expanded_tables
            .iter()
            .filter(|key| key.matches(target))
            .map(|key| key.table.clone())
            .collect::<Vec<_>>();
        self.table_details.retain(|key, _| !key.matches(target));
        for table in tables {
            self.load_table_details(target.clone(), table, cx);
        }
    }
}

pub(super) fn catalog_icon(kind: &str) -> Icon {
    let path = match kind {
        "database" => "icons/astesia/catalog-database.svg",
        "schema" => "icons/astesia/catalog-schema.svg",
        "column" => "icons/astesia/catalog-column.svg",
        "constraint" => "icons/astesia/catalog-constraint.svg",
        "index" => "icons/astesia/catalog-index.svg",
        _ => "icons/astesia/catalog-table.svg",
    };
    Icon::from_path(path)
}

enum CatalogDisclosure {
    Inline(Option<bool>),
    External,
}

pub(super) fn tree_row(
    id: String,
    label: String,
    kind: &str,
    expanded: Option<bool>,
    cx: &App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    tree_row_loading(id, label, kind, expanded, None, cx)
}

pub(super) fn tree_row_loading(
    id: String,
    label: String,
    kind: &str,
    expanded: Option<bool>,
    loading_label: Option<&'static str>,
    cx: &App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    tree_row_with_disclosure(
        id,
        label,
        kind,
        CatalogDisclosure::Inline(expanded),
        None,
        loading_label,
        cx,
    )
}

pub(super) fn column_row(
    id: String,
    name: String,
    detail: String,
    cx: &App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let label = format!("{name} {detail}");
    let content = h_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .overflow_hidden()
        .child(Label::new(name).flex_shrink_0().max_w_full().truncate())
        .child(
            Label::new(detail)
                .color(Color::Muted)
                .text_size(px(11.0))
                .buffer_font(cx)
                .flex_1()
                .min_w_0()
                .truncate(),
        )
        .into_any_element();
    tree_row_with_disclosure(
        id,
        label,
        "column",
        CatalogDisclosure::Inline(None),
        Some(content),
        None,
        cx,
    )
}

fn tree_row_with_disclosure(
    id: String,
    label: String,
    kind: &str,
    disclosure: CatalogDisclosure,
    label_content: Option<AnyElement>,
    loading_label: Option<&'static str>,
    cx: &App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let expanded = match disclosure {
        CatalogDisclosure::Inline(expanded) => expanded,
        CatalogDisclosure::External => None,
    };
    h_flex()
        .id(id)
        .key_context("SchemaObjectRow")
        .role(gpui_kit::Role::TreeItem)
        .tab_index(0)
        .aria_label(
            loading_label.map_or_else(|| label.clone(), |loading| format!("{label}, {loading}")),
        )
        .tooltip(crate::ui::components::Tooltip::text(label.clone()))
        .when_some(expanded, |row, expanded| row.aria_expanded(expanded))
        .min_w_0()
        .cursor_pointer()
        .w_full()
        .focus_visible(|row| {
            row.shadow(vec![gpui_kit::BoxShadow {
                inset: true,
                color: cx.theme().colors().border_focused,
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(0.0),
                spread_radius: px(1.0),
            }])
        })
        .child(catalog_row_content(
            loading_label.map_or(label, str::to_string),
            kind,
            disclosure,
            label_content,
            loading_label.is_some(),
        ))
}

fn catalog_row_content(
    label: String,
    kind: &str,
    disclosure: CatalogDisclosure,
    label_content: Option<AnyElement>,
    loading: bool,
) -> gpui_kit::Div {
    let inline = matches!(disclosure, CatalogDisclosure::Inline(_));
    h_flex()
        .w_full()
        .min_w_0()
        .text_size(px(12.0))
        .py_0p5()
        .pr_3()
        .pl(px(if inline { 12.0 } else { 0.0 }))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .when(inline, |row| {
                    row.child(
                        h_flex()
                            .w(px(16.0))
                            .flex_shrink_0()
                            .justify_center()
                            .when_some(
                                match disclosure {
                                    CatalogDisclosure::Inline(expanded) => expanded,
                                    CatalogDisclosure::External => None,
                                },
                                |slot, expanded| {
                                    slot.child(
                                        Icon::new(if expanded {
                                            IconName::ChevronDown
                                        } else {
                                            IconName::ChevronRight
                                        })
                                        .size(IconSize::XSmall)
                                        .color(Color::Muted),
                                    )
                                },
                            ),
                    )
                })
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1p5()
                        .child(
                            catalog_icon(kind)
                                .size(IconSize::Small)
                                .flex_shrink_0()
                                .when(loading, |icon| icon.color(Color::Muted)),
                        )
                        .child(h_flex().h_6().flex_1().min_w_0().child(
                            label_content.unwrap_or_else(|| {
                                Label::new(label)
                                    .truncate()
                                    .when(loading, |label| label.color(Color::Muted))
                                    .into_any_element()
                            }),
                        )),
                ),
        )
}
