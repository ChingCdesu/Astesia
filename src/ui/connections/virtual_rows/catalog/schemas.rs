use super::*;
use crate::application::connection_workspace::DatabaseCatalogSnapshot;
use crate::db::FunctionInfo;
use crate::ui::connections::catalog_tree::{tree_folder_row, CatalogFolderKey};

#[derive(Default)]
struct SchemaObjects<'a> {
    tables: Vec<&'a TableInfo>,
    functions: Vec<&'a FunctionInfo>,
}

fn schema_objects(
    catalog: &DatabaseCatalogSnapshot,
) -> BTreeMap<Option<String>, SchemaObjects<'_>> {
    let mut groups = BTreeMap::<Option<String>, SchemaObjects<'_>>::new();
    for entry in catalog.entries() {
        match entry {
            CatalogEntry::Schemas(CatalogSection::Ready(schemas)) => {
                for schema in schemas {
                    groups.entry(Some(schema.clone())).or_default();
                }
            }
            CatalogEntry::Tables(CatalogSection::Ready(tables)) => {
                for table in tables {
                    groups
                        .entry(table.reference.schema().map(str::to_owned))
                        .or_default()
                        .tables
                        .push(table);
                }
            }
            CatalogEntry::Functions(CatalogSection::Ready(functions)) => {
                for function in functions {
                    groups
                        .entry(function.schema.clone())
                        .or_default()
                        .functions
                        .push(function);
                }
            }
            _ => {}
        }
    }
    groups
}

impl ConnectionProfilesPanel {
    pub(super) fn append_schema_catalog_rows(
        &self,
        target: &QueryTarget,
        catalog: &DatabaseCatalogSnapshot,
        rows: &mut Vec<SidebarRow>,
    ) {
        let groups = schema_objects(catalog);
        for entry in catalog.entries() {
            if let CatalogEntry::Schemas(CatalogSection::Failed(error)) = entry {
                Self::append_message(
                    rows,
                    format!("schema-error-{target:?}"),
                    1,
                    Some(error.clone()),
                );
            }
        }
        if groups.is_empty() {
            self.append_heading(rows, target, "Schemas", 0, DatabaseObjectKind::Schema);
            for entry in catalog.entries() {
                let status = match entry {
                    CatalogEntry::Tables(CatalogSection::Failed(e))
                    | CatalogEntry::Functions(CatalogSection::Failed(e)) => Some(Some(e.clone())),
                    CatalogEntry::Schemas(CatalogSection::Loading)
                    | CatalogEntry::Tables(CatalogSection::Loading)
                    | CatalogEntry::Functions(CatalogSection::Loading) => Some(None),
                    _ => None,
                };
                if let Some(status) = status {
                    Self::append_message(
                        rows,
                        format!("schema-state-{target:?}-{:?}", entry.kind()),
                        2,
                        status,
                    );
                }
            }
        }
        let functions = catalog
            .entries()
            .find_map(|entry| {
                if let CatalogEntry::Functions(section) = entry {
                    Some(section)
                } else {
                    None
                }
            })
            .filter(|s| !matches!(s, CatalogSection::Unsupported));
        for (schema, objects) in groups {
            let depth = if let Some(schema_name) = &schema {
                let key = (
                    target.connection_id.clone(),
                    target.session_generation,
                    target.database.clone(),
                    schema_name.clone(),
                );
                let expanded = !self.collapsed_schemas.contains(&key);
                let saved = target.clone();
                let schema_name = schema_name.clone();
                rows.push(
                    SidebarRow::new(format!("schema-{key:?}"), 1, move |_, cx| {
                        let click = key.clone();
                        let keyboard = key.clone();
                        let target = saved.clone();
                        let schema = schema_name.clone();
                        tree_row(
                            format!("schema-{key:?}"),
                            schema_name.clone(),
                            "schema",
                            Some(expanded),
                            cx,
                        )
                        .on_click(
                            cx.listener(move |panel, event: &gpui_kit::ClickEvent, _, cx| {
                                if event.click_count() == 1 {
                                    panel.toggle_schema_group(click.clone(), cx);
                                }
                            }),
                        )
                        .on_action(cx.listener(move |panel, _: &menu::Confirm, _, cx| {
                            panel.toggle_schema_group(keyboard.clone(), cx)
                        }))
                        .on_mouse_down(
                            gpui_kit::MouseButton::Right,
                            cx.listener(
                                move |panel, event: &gpui_kit::MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    panel.open_schema_menu(
                                        target.clone(),
                                        schema.clone(),
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
                2
            } else {
                1
            };
            if self.append_schema_folder(
                rows,
                target,
                schema.clone(),
                depth,
                DatabaseObjectKind::Table,
                objects.tables.len(),
            ) {
                for table in objects.tables {
                    let key = CatalogTableKey::new(target, &table.reference);
                    let saved = target.clone();
                    let table = table.clone();
                    rows.push(
                        SidebarRow::new(format!("table-{key:?}"), depth + 1, move |panel, cx| {
                            panel.render_sql_table(&saved, &table, cx)
                        })
                        .highlight(self.selected_catalog_table.as_ref() == Some(&key)),
                    );
                    if self.expanded_tables.contains(&key) {
                        self.append_detail_rows(&key, target.db_type, depth + 2, rows);
                    }
                }
                append_section_status(
                    rows,
                    target,
                    schema.as_deref(),
                    depth + 1,
                    "Tables",
                    catalog.tables(),
                );
            }
            if let Some(section) = functions {
                if !self.append_schema_folder(
                    rows,
                    target,
                    schema.clone(),
                    depth,
                    DatabaseObjectKind::Function,
                    objects.functions.len(),
                ) {
                    continue;
                }
                for function in objects.functions {
                    let saved = target.clone();
                    let function = function.clone();
                    rows.push(SidebarRow::new(
                        format!("function-{target:?}-{schema:?}-{}", function.name),
                        depth + 1,
                        move |panel, cx| panel.render_schema_function(&saved, &function, cx),
                    ));
                }
                append_section_status(
                    rows,
                    target,
                    schema.as_deref(),
                    depth + 1,
                    "Functions",
                    section,
                );
            }
        }
    }

    fn append_schema_folder(
        &self,
        rows: &mut Vec<SidebarRow>,
        target: &QueryTarget,
        schema: Option<String>,
        depth: usize,
        kind: DatabaseObjectKind,
        count: usize,
    ) -> bool {
        let key = CatalogFolderKey::new(target, schema.clone(), kind);
        let expanded = self
            .catalog_folder_expansion
            .get(&key)
            .copied()
            .unwrap_or(kind == DatabaseObjectKind::Table);
        let saved = target.clone();
        rows.push(
            SidebarRow::new(format!("folder-{key:?}"), depth, move |panel, cx| {
                let language = panel.settings.read(cx).language();
                let label = if kind == DatabaseObjectKind::Function {
                    text(language, "函数", "Functions")
                } else {
                    text(language, "表", "Tables")
                };
                let click = key.clone();
                let keyboard = key.clone();
                let menu_key = key.clone();
                let target = saved.clone();
                let schema = schema.clone();
                tree_folder_row(format!("folder-{key:?}"), label, count, expanded, cx)
                    .on_click(
                        cx.listener(move |panel, event: &gpui_kit::ClickEvent, _, cx| {
                            if event.click_count() == 1 {
                                panel.toggle_catalog_folder(click.clone(), expanded, cx);
                            }
                        }),
                    )
                    .on_action(cx.listener(move |panel, _: &menu::Confirm, _, cx| {
                        panel.toggle_catalog_folder(keyboard.clone(), expanded, cx)
                    }))
                    .on_mouse_down(
                        gpui_kit::MouseButton::Right,
                        cx.listener(move |panel, event: &gpui_kit::MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            panel.selected_sidebar_row = Some(format!("folder-{menu_key:?}"));
                            panel.open_catalog_folder_menu(
                                target.clone(),
                                schema.clone(),
                                kind,
                                event.position,
                                window,
                                cx,
                            );
                        }),
                    )
                    .into_any_element()
            })
            .highlight(false),
        );
        expanded
    }

    fn toggle_catalog_folder(
        &mut self,
        key: CatalogFolderKey,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        self.selected_sidebar_row = Some(format!("folder-{key:?}"));
        self.catalog_folder_expansion.insert(key, !expanded);
        self.notify_sidebar(cx);
    }
}

fn append_section_status<T>(
    rows: &mut Vec<SidebarRow>,
    target: &QueryTarget,
    schema: Option<&str>,
    depth: usize,
    name: &str,
    section: &CatalogSection<T>,
) {
    let status = match section {
        CatalogSection::Loading => None,
        CatalogSection::Failed(error) => Some(error.clone()),
        _ => return,
    };
    ConnectionProfilesPanel::append_message(
        rows,
        format!("schema-section-{target:?}-{schema:?}-{name}"),
        depth,
        status,
    );
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
