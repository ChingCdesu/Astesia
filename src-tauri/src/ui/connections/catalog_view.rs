use gpui::{rgb, FontWeight};
use zed_ui::{prelude::*, Tooltip};

use super::ConnectionProfilesPanel;
use crate::application::connection_workspace::{
    CatalogSection, DatabaseCatalogSnapshot, ObjectListState,
};
use crate::application::{
    object_kind_can_create, object_kind_can_drop, object_kind_can_rename, DatabaseObjectKind,
    DropObjectTarget, ObjectMutation, QueryTarget,
};
use crate::ui::localization::text;
use crate::ui::object_definition_item::ObjectDefinition;
use crate::ui::object_mutation_form::ObjectMutationFormMode;

#[derive(Clone, Copy)]
struct CatalogSectionSpec {
    label: &'static str,
    kind: DatabaseObjectKind,
}

impl CatalogSectionSpec {
    const fn new(label: &'static str, kind: DatabaseObjectKind) -> Self {
        Self { label, kind }
    }
}

impl ConnectionProfilesPanel {
    pub(super) fn render_object_list(
        &self,
        target: &QueryTarget,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.settings.read(cx).language();
        match self.state.objects(target) {
            None => h_flex()
                .pl_4()
                .py_1()
                .child(
                    Label::new(text(language, "正在加载对象…", "Loading objects…"))
                        .size(LabelSize::XSmall),
                )
                .into_any_element(),
            Some(ObjectListState::Ready { catalog, .. }) => {
                self.render_catalog(target, catalog, cx)
            }
        }
    }

    fn render_catalog(
        &self,
        target: &QueryTarget,
        catalog: &DatabaseCatalogSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.settings.read(cx).language();
        let (primary_label, primary_empty_label) = match target.db_type {
            crate::db::DbType::MongoDB => (
                text(language, "集合", "Collections"),
                text(language, "未发现集合", "No collections found"),
            ),
            crate::db::DbType::Redis => (
                text(language, "键", "Keys"),
                text(language, "未发现键", "No keys found"),
            ),
            _ => (
                text(language, "表", "Tables"),
                text(language, "未发现表", "No tables found"),
            ),
        };
        let primary_rows = match &catalog.tables {
            CatalogSection::Unsupported => Vec::new(),
            CatalogSection::Loading => vec![
                self.catalog_section_heading_with_create(
                    target,
                    primary_label,
                    0,
                    DatabaseObjectKind::Table,
                    None,
                    cx,
                ),
                catalog_empty_row(text(language, "正在加载…", "Loading…")),
            ],
            CatalogSection::Failed(error) => vec![
                self.catalog_section_heading_with_create(
                    target,
                    primary_label,
                    0,
                    DatabaseObjectKind::Table,
                    None,
                    cx,
                ),
                catalog_error_row(error),
            ],
            CatalogSection::Ready(tables) => {
                let mut rows = vec![self.catalog_section_heading_with_create(
                    target,
                    primary_label,
                    tables.len(),
                    DatabaseObjectKind::Table,
                    None,
                    cx,
                )];
                if tables.is_empty() {
                    rows.push(catalog_empty_row(primary_empty_label));
                    rows
                } else {
                    rows.extend(tables.iter().enumerate().map(|(index, object)| {
                        let name = object.reference.to_string();
                        let target_for_action = target.clone();
                        let target_for_click = target.clone();
                        let table_for_action = object.reference.clone();
                        let table_for_click = object.reference.clone();
                        let structure_target = target.clone();
                        let structure_table = object.reference.clone();
                        let rename_target = target.clone();
                        let rename_name = name.clone();
                        let drop_target = target.clone();
                        let drop_name = name.clone();
                        let structure_label = text(language, "查看表结构", "View table structure");
                        let data_name = name.clone();
                        let supports_sql = target.db_type.capabilities().sql;
                        h_flex()
                            .id(format!(
                                "schema-object-{}-{}-{index}",
                                target.connection_id, target.database
                            ))
                            .min_w_0()
                            .gap_1p5()
                            .when(supports_sql, |element| {
                                element.child(
                                    h_flex()
                                        .id(format!("browse-table-data-{index}"))
                                        .role(gpui::Role::Button)
                                        .tab_index(0)
                                        .key_context("SchemaObjectRow")
                                        .aria_label(format!(
                                            "{} {name}",
                                            text(language, "浏览表数据", "Browse table data")
                                        ))
                                        .min_w_0()
                                        .flex_1()
                                        .gap_1p5()
                                        .px_1()
                                        .py_0p5()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(cx.theme().colors().border.opacity(0.0))
                                        .cursor_pointer()
                                        .focus_visible(|element| {
                                            element.border_color(cx.theme().colors().border_focused)
                                        })
                                        .hover(|element| {
                                            element.bg(cx.theme().colors().ghost_element_hover)
                                        })
                                        .on_action(cx.listener(
                                            move |panel, _: &menu::Confirm, _, cx| {
                                                panel.request_table_data(
                                                    target_for_action.clone(),
                                                    table_for_action.clone(),
                                                    cx,
                                                );
                                            },
                                        ))
                                        .on_click(cx.listener(move |panel, _, _, cx| {
                                            panel.request_table_data(
                                                target_for_click.clone(),
                                                table_for_click.clone(),
                                                cx,
                                            );
                                        }))
                                        .child(div().size(px(3.0)).rounded_full().bg(rgb(0x71717a)))
                                        .child(
                                            Label::new(data_name)
                                                .size(LabelSize::XSmall)
                                                .truncate()
                                                .flex_1(),
                                        )
                                        .when_some(object.row_count, |element, count| {
                                            element.child(
                                                Label::new(count.to_string())
                                                    .size(LabelSize::XSmall)
                                                    .color(Color::Muted),
                                            )
                                        }),
                                )
                            })
                            .when(!supports_sql, |element| {
                                element
                                    .px_1()
                                    .py_0p5()
                                    .child(div().size(px(3.0)).rounded_full().bg(rgb(0x71717a)))
                                    .child(
                                        Label::new(name)
                                            .size(LabelSize::XSmall)
                                            .truncate()
                                            .flex_1(),
                                    )
                                    .when_some(object.row_count, |element, count| {
                                        element.child(
                                            Label::new(count.to_string())
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        )
                                    })
                            })
                            .when(supports_sql, |element| {
                                element.child(
                                    IconButton::new(
                                        format!("view-table-structure-{index}"),
                                        IconName::ListTree,
                                    )
                                    .icon_size(IconSize::XSmall)
                                    .tooltip(Tooltip::text(structure_label))
                                    .on_click(cx.listener(
                                        move |panel, _, _, cx| {
                                            panel.request_table_structure(
                                                structure_target.clone(),
                                                structure_table.clone(),
                                                cx,
                                            );
                                        },
                                    )),
                                )
                            })
                            .when(
                                object_kind_can_rename(target.db_type, DatabaseObjectKind::Table),
                                |element| {
                                    element.child(
                                        IconButton::new(
                                            format!("rename-table-{index}"),
                                            IconName::Pencil,
                                        )
                                        .icon_size(IconSize::XSmall)
                                        .disabled(self.object_operation_in_progress)
                                        .tooltip(Tooltip::text(text(
                                            language,
                                            "重命名表",
                                            "Rename table",
                                        )))
                                        .on_click(
                                            cx.listener(move |panel, _, _, cx| {
                                                panel.request_rename_object(
                                                    rename_target.clone(),
                                                    DatabaseObjectKind::Table,
                                                    rename_name.clone(),
                                                    cx,
                                                );
                                            }),
                                        ),
                                    )
                                },
                            )
                            .when(
                                object_kind_can_drop(target.db_type, DatabaseObjectKind::Table),
                                |element| {
                                    element.child(
                                        IconButton::new(
                                            format!("drop-table-{index}"),
                                            IconName::Trash,
                                        )
                                        .icon_size(IconSize::XSmall)
                                        .disabled(self.object_operation_in_progress)
                                        .tooltip(Tooltip::text(text(
                                            language,
                                            "删除表",
                                            "Drop table",
                                        )))
                                        .on_click(
                                            cx.listener(move |panel, _, window, cx| {
                                                panel.confirm_drop_object(
                                                    drop_target.clone(),
                                                    ObjectMutation::Drop(DropObjectTarget::Table(
                                                        drop_name.clone(),
                                                    )),
                                                    window,
                                                    cx,
                                                );
                                            }),
                                        ),
                                    )
                                },
                            )
                            .into_any_element()
                    }));
                    rows
                }
            }
        };
        let refresh_target = target.clone();
        let refresh_label = text(language, "刷新对象", "Refresh objects");

        v_flex()
            .pl_4()
            .py_1()
            .gap_0p5()
            .child(
                h_flex()
                    .min_w_0()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Label::new(text(language, "数据库对象", "Database objects"))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .when(self.object_operation_in_progress, |element| {
                                element.child(
                                    Label::new(text(language, "正在更新…", "Updating…"))
                                        .size(LabelSize::XSmall),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_0p5()
                            .when(
                                target.db_type.capabilities().database_management,
                                |element| {
                                    let create_target = target.clone();
                                    element.child(
                                        IconButton::new(
                                            format!("create-database-{}", target.connection_id),
                                            IconName::Plus,
                                        )
                                        .icon_size(IconSize::XSmall)
                                        .disabled(self.object_operation_in_progress)
                                        .tooltip(Tooltip::text(text(
                                            language,
                                            "新建数据库",
                                            "Create database",
                                        )))
                                        .on_click(
                                            cx.listener(move |panel, _, _, cx| {
                                                panel.request_object_mutation(
                                                    ObjectMutationFormMode::Create {
                                                        target: create_target.clone(),
                                                        kind: DatabaseObjectKind::Database,
                                                        schema: None,
                                                    },
                                                    cx,
                                                );
                                            }),
                                        ),
                                    )
                                },
                            )
                            .child(
                                IconButton::new(
                                    format!(
                                        "refresh-objects-{}-{}",
                                        target.connection_id, target.database
                                    ),
                                    IconName::RotateCw,
                                )
                                .icon_size(IconSize::XSmall)
                                .disabled(self.object_operation_in_progress)
                                .tooltip(Tooltip::text(refresh_label))
                                .on_click(cx.listener(
                                    move |panel, event, window, cx| {
                                        panel.retry_objects(
                                            refresh_target.clone(),
                                            event,
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                            ),
                    ),
            )
            .children(primary_rows)
            .children(self.render_mutable_catalog_text_section(
                target,
                &catalog.schemas,
                CatalogSectionSpec::new(
                    text(language, "Schema", "Schemas"),
                    DatabaseObjectKind::Schema,
                ),
                |item| item.clone(),
                |item| DropObjectTarget::Schema(item.clone()),
                cx,
            ))
            .children(self.render_definition_section(
                target,
                &catalog.views,
                CatalogSectionSpec::new(text(language, "视图", "Views"), DatabaseObjectKind::View),
                |item| item.name.clone(),
                ObjectDefinition::view,
                cx,
            ))
            .children(self.render_definition_section(
                target,
                &catalog.functions,
                CatalogSectionSpec::new(
                    text(language, "函数", "Functions"),
                    DatabaseObjectKind::Function,
                ),
                |item| item.name.clone(),
                ObjectDefinition::function,
                cx,
            ))
            .children(self.render_definition_section(
                target,
                &catalog.procedures,
                CatalogSectionSpec::new(
                    text(language, "存储过程", "Procedures"),
                    DatabaseObjectKind::Procedure,
                ),
                |item| item.name.clone(),
                ObjectDefinition::procedure,
                cx,
            ))
            .children(self.render_mutable_catalog_text_section(
                target,
                &catalog.triggers,
                CatalogSectionSpec::new(
                    text(language, "触发器", "Triggers"),
                    DatabaseObjectKind::Trigger,
                ),
                |item| format!("{} · {} {}", item.name, item.timing, item.event),
                |item| DropObjectTarget::Trigger {
                    name: item.name.clone(),
                    table: item.table.clone(),
                },
                cx,
            ))
            .children(self.render_mutable_catalog_text_section(
                target,
                &catalog.users,
                CatalogSectionSpec::new(text(language, "用户", "Users"), DatabaseObjectKind::User),
                |item| match &item.host {
                    Some(host) => format!("{}@{host}", item.name),
                    None => item.name.clone(),
                },
                |item| DropObjectTarget::User {
                    name: item.name.clone(),
                    host: item.host.clone(),
                },
                cx,
            ))
            .into_any_element()
    }

    fn render_definition_section<T>(
        &self,
        target: &QueryTarget,
        section: &CatalogSection<T>,
        spec: CatalogSectionSpec,
        display: impl Fn(&T) -> String,
        object: impl Fn(QueryTarget, &T) -> ObjectDefinition,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let CatalogSectionSpec { label, kind } = spec;
        match section {
            CatalogSection::Unsupported => Vec::new(),
            CatalogSection::Loading => vec![
                self.catalog_section_heading_with_create(target, label, 0, kind, None, cx),
                catalog_empty_row(text(
                    self.settings.read(cx).language(),
                    "正在加载…",
                    "Loading…",
                )),
            ],
            CatalogSection::Failed(error) => vec![
                self.catalog_section_heading_with_create(target, label, 0, kind, None, cx),
                div()
                    .px_1()
                    .py_0p5()
                    .child(
                        Label::new(error.clone())
                            .size(LabelSize::XSmall)
                            .color(Color::Error)
                            .line_clamp(2),
                    )
                    .into_any_element(),
            ],
            CatalogSection::Ready(items) => {
                let mut rows = vec![self.catalog_section_heading_with_create(
                    target,
                    label,
                    items.len(),
                    kind,
                    None,
                    cx,
                )];
                if items.is_empty() {
                    rows.push(catalog_empty_row("—"));
                } else {
                    rows.extend(items.iter().enumerate().map(|(index, item)| {
                        let label = display(item);
                        let action_object = object(target.clone(), item);
                        let click_object = action_object.clone();
                        let drop_target = target.clone();
                        let drop_mutation = ObjectMutation::Drop(action_object.drop_target());
                        h_flex()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .id(format!(
                                        "definition-object-{}-{}-{index}",
                                        target.connection_id, target.database
                                    ))
                                    .role(gpui::Role::Button)
                                    .tab_index(0)
                                    .key_context("SchemaObjectRow")
                                    .aria_label(label.clone())
                                    .min_w_0()
                                    .flex_1()
                                    .gap_1p5()
                                    .px_1()
                                    .py_0p5()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(cx.theme().colors().border.opacity(0.0))
                                    .cursor_pointer()
                                    .focus_visible(|element| {
                                        element.border_color(cx.theme().colors().border_focused)
                                    })
                                    .hover(|element| {
                                        element.bg(cx.theme().colors().ghost_element_hover)
                                    })
                                    .on_action(cx.listener(
                                        move |panel, _: &menu::Confirm, _, cx| {
                                            panel.request_object_definition(
                                                action_object.clone(),
                                                cx,
                                            );
                                        },
                                    ))
                                    .on_click(cx.listener(move |panel, _, _, cx| {
                                        panel.request_object_definition(click_object.clone(), cx);
                                    }))
                                    .child(div().size(px(3.0)).rounded_full().bg(rgb(0x71717a)))
                                    .child(
                                        Label::new(label)
                                            .size(LabelSize::XSmall)
                                            .truncate()
                                            .flex_1(),
                                    ),
                            )
                            .when(object_kind_can_drop(target.db_type, kind), |element| {
                                element.child(
                                    IconButton::new(
                                        format!("drop-definition-{kind:?}-{index}"),
                                        IconName::Trash,
                                    )
                                    .icon_size(IconSize::XSmall)
                                    .disabled(self.object_operation_in_progress)
                                    .tooltip(Tooltip::text(text(
                                        self.settings.read(cx).language(),
                                        "删除对象",
                                        "Drop object",
                                    )))
                                    .on_click(cx.listener(
                                        move |panel, _, window, cx| {
                                            panel.confirm_drop_object(
                                                drop_target.clone(),
                                                drop_mutation.clone(),
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                                )
                            })
                            .into_any_element()
                    }));
                }
                rows
            }
        }
    }

    fn catalog_section_heading_with_create(
        &self,
        target: &QueryTarget,
        label: &str,
        count: usize,
        kind: DatabaseObjectKind,
        schema: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !object_kind_can_create(target.db_type, kind) {
            return catalog_section_heading(label.to_string(), count);
        }
        let create_target = target.clone();
        let create_label = format!(
            "{} {label}",
            text(self.settings.read(cx).language(), "新建", "Create")
        );
        h_flex()
            .pt_1p5()
            .px_1()
            .justify_between()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Label::new(label.to_string())
                            .size(LabelSize::XSmall)
                            .weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new(count.to_string())
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(
                IconButton::new(
                    format!("create-catalog-{kind:?}-{}", target.database),
                    IconName::Plus,
                )
                .icon_size(IconSize::XSmall)
                .disabled(self.object_operation_in_progress)
                .tooltip(Tooltip::text(create_label))
                .on_click(cx.listener(move |panel, _, _, cx| {
                    panel.request_object_mutation(
                        ObjectMutationFormMode::Create {
                            target: create_target.clone(),
                            kind,
                            schema: schema.clone(),
                        },
                        cx,
                    );
                })),
            )
            .into_any_element()
    }

    fn render_mutable_catalog_text_section<T>(
        &self,
        target: &QueryTarget,
        section: &CatalogSection<T>,
        spec: CatalogSectionSpec,
        display: impl Fn(&T) -> String,
        drop_target: impl Fn(&T) -> DropObjectTarget,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let CatalogSectionSpec { label, kind } = spec;
        match section {
            CatalogSection::Unsupported => Vec::new(),
            CatalogSection::Loading => vec![
                self.catalog_section_heading_with_create(target, label, 0, kind, None, cx),
                catalog_empty_row(text(
                    self.settings.read(cx).language(),
                    "正在加载…",
                    "Loading…",
                )),
            ],
            CatalogSection::Failed(error) => vec![
                self.catalog_section_heading_with_create(target, label, 0, kind, None, cx),
                catalog_error_row(error),
            ],
            CatalogSection::Ready(items) => {
                let mut rows = vec![self.catalog_section_heading_with_create(
                    target,
                    label,
                    items.len(),
                    kind,
                    None,
                    cx,
                )];
                if items.is_empty() {
                    rows.push(catalog_empty_row("—"));
                } else {
                    rows.extend(items.iter().enumerate().map(|(index, item)| {
                        let drop_target = drop_target(item);
                        let rename_target = target.clone();
                        let rename_name = drop_target.name().to_string();
                        let target_for_drop = target.clone();
                        let drop_mutation = ObjectMutation::Drop(drop_target);
                        h_flex()
                            .min_w_0()
                            .gap_1p5()
                            .px_1()
                            .py_0p5()
                            .child(div().size(px(3.0)).rounded_full().bg(rgb(0x71717a)))
                            .child(
                                Label::new(display(item))
                                    .size(LabelSize::XSmall)
                                    .truncate()
                                    .flex_1(),
                            )
                            .when(object_kind_can_rename(target.db_type, kind), |element| {
                                element.child(
                                    IconButton::new(
                                        format!("rename-catalog-{kind:?}-{index}"),
                                        IconName::Pencil,
                                    )
                                    .icon_size(IconSize::XSmall)
                                    .disabled(self.object_operation_in_progress)
                                    .tooltip(Tooltip::text(text(
                                        self.settings.read(cx).language(),
                                        "重命名对象",
                                        "Rename object",
                                    )))
                                    .on_click(cx.listener(
                                        move |panel, _, _, cx| {
                                            panel.request_rename_object(
                                                rename_target.clone(),
                                                kind,
                                                rename_name.clone(),
                                                cx,
                                            );
                                        },
                                    )),
                                )
                            })
                            .when(object_kind_can_drop(target.db_type, kind), |element| {
                                element.child(
                                    IconButton::new(
                                        format!("drop-catalog-{kind:?}-{index}"),
                                        IconName::Trash,
                                    )
                                    .icon_size(IconSize::XSmall)
                                    .disabled(self.object_operation_in_progress)
                                    .tooltip(Tooltip::text(text(
                                        self.settings.read(cx).language(),
                                        "删除对象",
                                        "Drop object",
                                    )))
                                    .on_click(cx.listener(
                                        move |panel, _, window, cx| {
                                            panel.confirm_drop_object(
                                                target_for_drop.clone(),
                                                drop_mutation.clone(),
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                                )
                            })
                            .into_any_element()
                    }));
                }
                rows
            }
        }
    }
}

fn catalog_section_heading(label: impl Into<SharedString>, count: usize) -> AnyElement {
    h_flex()
        .pt_1p5()
        .px_1()
        .justify_between()
        .child(
            Label::new(label)
                .size(LabelSize::XSmall)
                .weight(FontWeight::SEMIBOLD),
        )
        .child(
            Label::new(count.to_string())
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .into_any_element()
}

fn catalog_empty_row(label: impl Into<SharedString>) -> AnyElement {
    div()
        .px_1()
        .py_0p5()
        .child(
            Label::new(label)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .into_any_element()
}

fn catalog_error_row(error: &str) -> AnyElement {
    div()
        .px_1()
        .py_0p5()
        .child(
            Label::new(error.to_string())
                .size(LabelSize::XSmall)
                .color(Color::Error)
                .line_clamp(2),
        )
        .into_any_element()
}
