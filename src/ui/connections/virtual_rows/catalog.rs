use super::*;
use crate::application::connection_workspace::{CatalogEntry, CatalogSection, ObjectListState};
use std::collections::BTreeMap;

impl ConnectionProfilesPanel {
    pub(super) fn append_catalog_rows(&self, target: &QueryTarget, rows: &mut Vec<SidebarRow>) {
        let Some(ObjectListState::Ready { catalog, .. }) = self.state.objects(target) else {
            return;
        };
        if target.db_type == crate::db::DbType::Redis {
            let saved = target.clone();
            rows.push(SidebarRow::new(
                format!("redis-search-{target:?}"),
                1,
                move |panel, cx| {
                    panel
                        .render_redis_search(&saved, cx)
                        .unwrap_or_else(|| div().into_any_element())
                },
            ));
        }
        let primary = self
            .redis_search_result
            .as_ref()
            .filter(|(search_target, _)| search_target == target)
            .map(|(_, result)| CatalogSection::from_result(result.clone()));
        let tables = primary.as_ref().unwrap_or_else(|| catalog.tables());
        match tables {
            CatalogSection::Ready(tables) if target.db_type.capabilities().sql => {
                let mut groups = BTreeMap::<Option<String>, Vec<&TableInfo>>::new();
                for entry in catalog.entries() {
                    match entry {
                        CatalogEntry::Schemas(CatalogSection::Ready(schemas)) => {
                            for schema in schemas {
                                groups.entry(Some(schema.clone())).or_default();
                            }
                        }
                        CatalogEntry::Schemas(CatalogSection::Failed(error)) => {
                            Self::append_message(
                                rows,
                                format!("schema-error-{target:?}"),
                                1,
                                Some(error.clone()),
                            );
                        }
                        _ => {}
                    }
                }
                for table in tables {
                    groups
                        .entry(table.reference.schema().map(str::to_owned))
                        .or_default()
                        .push(table);
                }
                if groups.is_empty() {
                    self.append_heading(
                        rows,
                        target,
                        if target.db_type.capabilities().schemas {
                            "Schemas"
                        } else {
                            "Tables"
                        },
                        0,
                        if target.db_type.capabilities().schemas {
                            DatabaseObjectKind::Schema
                        } else {
                            DatabaseObjectKind::Table
                        },
                    );
                }
                for (schema, tables) in groups {
                    let mut depth = 1;
                    if let Some(schema) = schema {
                        let key = (
                            target.connection_id.clone(),
                            target.session_generation,
                            target.database.clone(),
                            schema.clone(),
                        );
                        let expanded = !self.collapsed_schemas.contains(&key);
                        let saved = target.clone();
                        rows.push(
                            SidebarRow::new(format!("schema-{key:?}"), depth, move |_, cx| {
                                let click = key.clone();
                                let keyboard = key.clone();
                                let menu_target = saved.clone();
                                let menu_schema = schema.clone();
                                tree_row(
                                    format!("schema-{key:?}"),
                                    schema.clone(),
                                    "schema",
                                    Some(expanded),
                                    cx,
                                )
                                .on_click(cx.listener(
                                    move |panel, event: &gpui_kit::ClickEvent, _, cx| {
                                        if event.click_count() > 1 {
                                            return;
                                        }
                                        panel.toggle_schema_group(click.clone(), cx)
                                    },
                                ))
                                .on_action(cx.listener(move |panel, _: &menu::Confirm, _, cx| {
                                    panel.toggle_schema_group(keyboard.clone(), cx)
                                }))
                                .on_mouse_down(
                                    gpui_kit::MouseButton::Right,
                                    cx.listener(
                                        move |panel,
                                              event: &gpui_kit::MouseDownEvent,
                                              window,
                                              cx| {
                                            cx.stop_propagation();
                                            panel.open_schema_menu(
                                                menu_target.clone(),
                                                menu_schema.clone(),
                                                event.position,
                                                window,
                                                cx,
                                            );
                                        },
                                    ),
                                )
                                .into_any_element()
                            })
                            .highlight(false),
                        );
                        if !expanded {
                            continue;
                        }
                        depth += 1;
                    }
                    for table in tables {
                        let key = CatalogTableKey::new(target, &table.reference);
                        let saved = target.clone();
                        let table = table.clone();
                        rows.push(
                            SidebarRow::new(format!("table-{key:?}"), depth, move |panel, cx| {
                                panel.render_sql_table(&saved, &table, cx)
                            })
                            .highlight(self.selected_catalog_table.as_ref() == Some(&key)),
                        );
                        if self.expanded_tables.contains(&key) {
                            self.append_detail_rows(&key, target.db_type, depth + 1, rows);
                        }
                    }
                }
            }
            CatalogSection::Unsupported | CatalogSection::Loading => {}
            CatalogSection::Ready(tables) => {
                self.append_heading(
                    rows,
                    target,
                    if target.db_type == crate::db::DbType::Redis {
                        "Keys"
                    } else {
                        "Collections"
                    },
                    tables.len(),
                    DatabaseObjectKind::Table,
                );
                if tables.is_empty() {
                    let redis = target.db_type == crate::db::DbType::Redis;
                    rows.push(SidebarRow::new(
                        format!("primary-empty-{target:?}"),
                        1,
                        move |panel, cx| {
                            let language = panel.settings.read(cx).language();
                            super::super::catalog_view::catalog_empty_row(if redis {
                                text(language, "未发现键", "No keys found")
                            } else {
                                text(language, "未发现集合", "No collections found")
                            })
                        },
                    ));
                }
                for (index, table) in tables.iter().enumerate() {
                    let saved = target.clone();
                    let table = table.clone();
                    rows.push(
                        SidebarRow::new(
                            format!("object-{target:?}-{:?}", table.reference),
                            1,
                            move |panel, cx| {
                                panel.render_primary_catalog_row(&saved, &table, index, cx)
                            },
                        )
                        .highlight(false),
                    );
                }
            }
            CatalogSection::Failed(error) => {
                self.append_heading(
                    rows,
                    target,
                    match target.db_type {
                        crate::db::DbType::Redis => "Keys",
                        crate::db::DbType::MongoDB => "Collections",
                        _ => "Tables",
                    },
                    0,
                    DatabaseObjectKind::Table,
                );
                Self::append_message(
                    rows,
                    format!("primary-state-{target:?}"),
                    1,
                    Some(error.clone()),
                );
            }
        }
        for entry in catalog.entries() {
            self.append_secondary_rows(target, entry, rows);
        }
    }

    fn append_heading(
        &self,
        rows: &mut Vec<SidebarRow>,
        target: &QueryTarget,
        label: &'static str,
        count: usize,
        kind: DatabaseObjectKind,
    ) {
        let target = target.clone();
        rows.push(SidebarRow::new(
            format!("heading-{target:?}-{kind:?}"),
            1,
            move |panel, cx| {
                let language = panel.settings.read(cx).language();
                let label = match label {
                    "Tables" => text(language, "表", "Tables"),
                    "Schemas" => text(language, "Schema", "Schemas"),
                    "Keys" => text(language, "键", "Keys"),
                    "Collections" => text(language, "集合", "Collections"),
                    "Views" => text(language, "视图", "Views"),
                    "Functions" => text(language, "函数", "Functions"),
                    "Procedures" => text(language, "存储过程", "Procedures"),
                    "Triggers" => text(language, "触发器", "Triggers"),
                    label => label,
                };
                panel.catalog_section_heading_with_create(&target, label, count, kind, None, cx)
            },
        ));
    }

    pub(super) fn append_message(
        rows: &mut Vec<SidebarRow>,
        key: String,
        depth: usize,
        message: Option<String>,
    ) {
        rows.push(SidebarRow::new(
            format!("{key}-{message:?}"),
            depth,
            move |panel, cx| match &message {
                Some(message) => super::catalog_view::catalog_error_row(message),
                None => super::catalog_view::catalog_empty_row(text(
                    panel.settings.read(cx).language(),
                    "正在加载…",
                    "Loading…",
                )),
            },
        ));
    }

    fn append_secondary_rows(
        &self,
        target: &QueryTarget,
        entry: &CatalogEntry,
        rows: &mut Vec<SidebarRow>,
    ) {
        macro_rules! section {
            ($section:expr, $variant:ident, $label:literal, $kind:ident) => {
                match $section {
                    CatalogSection::Unsupported | CatalogSection::Loading => {}
                    CatalogSection::Ready(items) if items.is_empty() => {}
                    section => {
                        let count = match section {
                            CatalogSection::Ready(items) => items.len(),
                            _ => 0,
                        };
                        self.append_heading(rows, target, $label, count, DatabaseObjectKind::$kind);
                        match section {
                            CatalogSection::Ready(items) => {
                                for (index, item) in items.iter().enumerate() {
                                    let entry =
                                        CatalogEntry::$variant(CatalogSection::Ready(vec![
                                            item.clone()
                                        ]));
                                    let saved = target.clone();
                                    rows.push(SidebarRow::new(
                                        format!("secondary-{target:?}-{}-{index}", $label),
                                        1,
                                        move |panel, cx| {
                                            panel
                                                .render_secondary_entry(&saved, &entry, cx)
                                                .pop()
                                                .expect("one catalog item")
                                        },
                                    ));
                                }
                            }
                            CatalogSection::Failed(error) => Self::append_message(
                                rows,
                                format!("secondary-error-{target:?}-{}", $label),
                                1,
                                Some(error.clone()),
                            ),
                            CatalogSection::Unsupported | CatalogSection::Loading => {}
                        }
                    }
                }
            };
        }
        match entry {
            CatalogEntry::Views(s) => section!(s, Views, "Views", View),
            CatalogEntry::Functions(s) => section!(s, Functions, "Functions", Function),
            CatalogEntry::Procedures(s) => section!(s, Procedures, "Procedures", Procedure),
            CatalogEntry::Triggers(s) => section!(s, Triggers, "Triggers", Trigger),
            _ => {}
        }
    }
}
