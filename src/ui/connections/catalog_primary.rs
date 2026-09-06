use crate::ui::components::{prelude::*, Tooltip};
use gpui_kit::rgb;

use super::engine_workflows::{DraggedTableCopy, DraggedTableCopyPreview};
use super::ConnectionProfilesPanel;
use crate::application::{
    object_kind_can_drop, object_kind_can_rename, DatabaseObjectKind, DropObjectTarget,
    ObjectMutation, QueryTarget,
};
use crate::ui::localization::text;

impl ConnectionProfilesPanel {
    pub(super) fn render_primary_catalog_row(
        &self,
        target: &QueryTarget,
        object: &crate::db::TableInfo,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.settings.read(cx).language();
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
        let copy_target = target.clone();
        let copy_table = object.reference.clone();
        let dragged_table = DraggedTableCopy {
            source: target.clone(),
            table: object.reference.clone(),
        };
        let drag_label = name.clone();
        let backup_target = target.clone();
        let backup_table = object.reference.clone();
        let structure_label = text(language, "查看表结构", "View table structure");
        let data_name = name.clone();
        let supports_sql = target.db_type.capabilities().sql;
        let supports_browse = supports_sql
            || matches!(
                target.db_type,
                crate::db::DbType::MongoDB | crate::db::DbType::Redis
            );
        h_flex()
            .id(format!(
                "schema-object-{}-{}-{index}",
                target.connection_id, target.database
            ))
            .min_w_0()
            .gap_1p5()
            .when(supports_browse, |element| {
                element.child(
                    h_flex()
                        .id(format!("browse-table-data-{index}"))
                        .role(gpui_kit::Role::Button)
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
                        .hover(|element| element.bg(cx.theme().colors().ghost_element_hover))
                        .on_action(cx.listener(move |panel, _: &menu::Confirm, _, cx| {
                            panel.request_primary_data(
                                target_for_action.clone(),
                                table_for_action.clone(),
                                cx,
                            );
                        }))
                        .on_click(cx.listener(move |panel, _, _, cx| {
                            panel.request_primary_data(
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
            .when(!supports_browse, |element| {
                element
                    .px_1()
                    .py_0p5()
                    .child(div().size(px(3.0)).rounded_full().bg(rgb(0x71717a)))
                    .child(Label::new(name).size(LabelSize::XSmall).truncate().flex_1())
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
                    IconButton::new(format!("view-table-structure-{index}"), IconName::ListTree)
                        .icon_size(IconSize::XSmall)
                        .tooltip(Tooltip::text(structure_label))
                        .on_click(cx.listener(move |panel, _, _, cx| {
                            panel.request_table_structure(
                                structure_target.clone(),
                                structure_table.clone(),
                                cx,
                            );
                        })),
                )
            })
            .when(
                target.db_type.capabilities().table_copy != crate::db::TableCopyMode::None,
                |element| {
                    element.child(
                        IconButton::new(format!("copy-table-{index}"), IconName::Copy)
                            .icon_size(IconSize::XSmall)
                            .tooltip(Tooltip::text(text(language, "复制表", "Copy table")))
                            .on_click(cx.listener(move |panel, _, _, cx| {
                                panel.copy_table(copy_target.clone(), copy_table.clone(), cx);
                            })),
                    )
                },
            )
            .when(target.db_type.capabilities().backup, |element| {
                element.child(
                    IconButton::new(format!("backup-table-{index}"), IconName::Download)
                        .icon_size(IconSize::XSmall)
                        .tooltip(Tooltip::text(text(
                            language,
                            "备份此表",
                            "Back up this table",
                        )))
                        .on_click(cx.listener(move |panel, _, _, cx| {
                            panel.request_backup(
                                backup_target.clone(),
                                Some(vec![backup_table.clone()]),
                                cx,
                            );
                        })),
                )
            })
            .when(
                object_kind_can_rename(target.db_type, DatabaseObjectKind::Table),
                |element| {
                    element.child(
                        IconButton::new(format!("rename-table-{index}"), IconName::Pencil)
                            .icon_size(IconSize::XSmall)
                            .disabled(self.object_operation_in_progress)
                            .tooltip(Tooltip::text(text(language, "重命名表", "Rename table")))
                            .on_click(cx.listener(move |panel, _, _, cx| {
                                panel.request_rename_object(
                                    rename_target.clone(),
                                    DatabaseObjectKind::Table,
                                    rename_name.clone(),
                                    cx,
                                );
                            })),
                    )
                },
            )
            .when(
                object_kind_can_drop(target.db_type, DatabaseObjectKind::Table),
                |element| {
                    element.child(
                        IconButton::new(format!("drop-table-{index}"), IconName::Trash)
                            .icon_size(IconSize::XSmall)
                            .disabled(self.object_operation_in_progress)
                            .tooltip(Tooltip::text(text(language, "删除表", "Drop table")))
                            .on_click(cx.listener(move |panel, _, window, cx| {
                                panel.confirm_drop_object(
                                    drop_target.clone(),
                                    ObjectMutation::Drop(DropObjectTarget::Table(
                                        drop_name.clone(),
                                    )),
                                    window,
                                    cx,
                                );
                            })),
                    )
                },
            )
            .when(
                target.db_type.capabilities().table_copy != crate::db::TableCopyMode::None,
                |element| {
                    element.on_drag(dragged_table, move |_, _, _, cx| {
                        cx.new(|_| DraggedTableCopyPreview {
                            label: drag_label.clone(),
                        })
                    })
                },
            )
            .into_any_element()
    }
}
