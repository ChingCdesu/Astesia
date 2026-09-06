use crate::ui::components::{prelude::*, Tooltip};
use gpui_kit::{rgb, FontWeight};

use super::ConnectionProfilesPanel;
use crate::application::connection_workspace::{CatalogEntry, CatalogSection};
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
    pub(super) fn render_redis_search(
        &self,
        target: &QueryTarget,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if target.db_type != crate::db::DbType::Redis {
            return None;
        }
        let language = self.settings.read(cx).language();
        let search_target = target.clone();
        Some(
            h_flex()
                .gap_1()
                .pr_1()
                .child(div().flex_1().child(self.redis_search.clone()))
                .child(
                    Button::new(
                        format!("search-redis-keys-{}", target.database),
                        text(language, "扫描", "Scan"),
                    )
                    .size(ButtonSize::Compact)
                    .loading(self.redis_search_busy)
                    .disabled(self.redis_search_busy)
                    .on_click(cx.listener(move |panel, event, window, cx| {
                        panel.search_redis_keys(search_target.clone(), event, window, cx);
                    })),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_secondary_entry(
        &self,
        target: &QueryTarget,
        entry: &CatalogEntry,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let language = self.settings.read(cx).language();
        let mut rows = Vec::new();
        let empty = match entry {
            CatalogEntry::Views(CatalogSection::Ready(items)) => items.is_empty(),
            CatalogEntry::Functions(CatalogSection::Ready(items)) => items.is_empty(),
            CatalogEntry::Procedures(CatalogSection::Ready(items)) => items.is_empty(),
            CatalogEntry::Triggers(CatalogSection::Ready(items)) => items.is_empty(),
            CatalogEntry::Users(CatalogSection::Ready(items)) => items.is_empty(),
            _ => false,
        };
        if empty {
            return rows;
        }
        match entry {
            CatalogEntry::Schemas(_) => {}
            CatalogEntry::Tables(_) => {}
            CatalogEntry::Views(section) => {
                rows.extend(self.render_definition_section(
                    target,
                    section,
                    CatalogSectionSpec::new(
                        text(language, "视图", "Views"),
                        DatabaseObjectKind::View,
                    ),
                    |item| item.name.clone(),
                    ObjectDefinition::view,
                    cx,
                ));
            }
            CatalogEntry::Functions(section) => {
                rows.extend(self.render_definition_section(
                    target,
                    section,
                    CatalogSectionSpec::new(
                        text(language, "函数", "Functions"),
                        DatabaseObjectKind::Function,
                    ),
                    |item| item.name.clone(),
                    ObjectDefinition::function,
                    cx,
                ));
            }
            CatalogEntry::Procedures(section) => {
                rows.extend(self.render_definition_section(
                    target,
                    section,
                    CatalogSectionSpec::new(
                        text(language, "存储过程", "Procedures"),
                        DatabaseObjectKind::Procedure,
                    ),
                    |item| item.name.clone(),
                    ObjectDefinition::procedure,
                    cx,
                ));
            }
            CatalogEntry::Triggers(section) => {
                rows.extend(self.render_mutable_catalog_text_section(
                    target,
                    section,
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
                ));
            }
            CatalogEntry::Users(section) => {
                rows.extend(self.render_mutable_catalog_text_section(
                    target,
                    section,
                    CatalogSectionSpec::new(
                        text(language, "用户", "Users"),
                        DatabaseObjectKind::User,
                    ),
                    |item| match &item.host {
                        Some(host) => format!("{}@{host}", item.name),
                        None => item.name.clone(),
                    },
                    |item| DropObjectTarget::User {
                        name: item.name.clone(),
                        host: item.host.clone(),
                    },
                    cx,
                ));
            }
        }
        rows
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
                                    .role(gpui_kit::Role::Button)
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

    pub(super) fn catalog_section_heading_with_create(
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

pub(super) fn catalog_empty_row(label: impl Into<SharedString>) -> AnyElement {
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

pub(super) fn catalog_error_row(error: &str) -> AnyElement {
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
