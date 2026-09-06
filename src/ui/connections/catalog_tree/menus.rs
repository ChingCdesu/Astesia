use super::*;
use crate::application::{
    object_kind_can_create, object_kind_can_drop, object_kind_can_rename, DatabaseObjectKind,
    DropObjectTarget, ObjectMutation,
};
use crate::ui::components::{ContextMenu, ContextMenuEntry};
use gpui_kit::Focusable;

#[derive(Clone, Copy)]
enum CatalogAction {
    Structure,
    Copy,
    Backup,
    CreateTable,
    CreateSchema,
    RenameTable,
    DropTable,
    RenameSchema,
    DropSchema,
}

impl ConnectionProfilesPanel {
    pub(super) fn open_table_menu(
        &mut self,
        target: QueryTarget,
        table: TableRef,
        position: gpui_kit::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_catalog_menu(target, Some(table), None, position, window, cx);
    }

    pub(in super::super) fn open_schema_menu(
        &mut self,
        target: QueryTarget,
        schema: String,
        position: gpui_kit::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_catalog_menu(target, None, Some(schema), position, window, cx);
    }

    fn open_catalog_menu(
        &mut self,
        target: QueryTarget,
        table: Option<TableRef>,
        schema: Option<String>,
        position: gpui_kit::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use CatalogAction::*;
        let language = self.settings.read(cx).language();
        let owner = cx.entity().downgrade();
        let busy = self.actions_blocked();
        let engine = target.db_type;
        let actions = if table.is_some() {
            vec![
                Structure,
                Copy,
                Backup,
                CreateTable,
                CreateSchema,
                RenameTable,
                DropTable,
            ]
        } else {
            vec![CreateTable, CreateSchema, RenameSchema, DropSchema]
        };
        let menu = ContextMenu::build(window, cx, |mut menu, _, _| {
            for action in actions {
                let supported = match action {
                    Structure => true,
                    Copy => engine.capabilities().table_copy != crate::db::TableCopyMode::None,
                    Backup => engine.capabilities().backup,
                    CreateTable => object_kind_can_create(engine, DatabaseObjectKind::Table),
                    CreateSchema => object_kind_can_create(engine, DatabaseObjectKind::Schema),
                    RenameTable => object_kind_can_rename(engine, DatabaseObjectKind::Table),
                    RenameSchema => object_kind_can_rename(engine, DatabaseObjectKind::Schema),
                    DropTable => object_kind_can_drop(engine, DatabaseObjectKind::Table),
                    DropSchema => object_kind_can_drop(engine, DatabaseObjectKind::Schema),
                };
                if !supported {
                    continue;
                }
                let owner = owner.clone();
                let target = target.clone();
                let table = table.clone();
                let schema = schema.clone();
                let label = match action {
                    Structure => text(language, "查看表结构", "View Table Structure"),
                    Copy => text(language, "复制表", "Copy Table"),
                    Backup => text(language, "备份此表", "Back Up Table"),
                    CreateTable => text(language, "新建表…", "Create Table…"),
                    CreateSchema => text(language, "新建 Schema…", "Create Schema…"),
                    RenameTable => text(language, "重命名表…", "Rename Table…"),
                    DropTable => text(language, "删除表…", "Drop Table…"),
                    RenameSchema => text(language, "重命名 Schema…", "Rename Schema…"),
                    DropSchema => text(language, "删除 Schema…", "Drop Schema…"),
                };
                if matches!(action, DropTable | DropSchema) {
                    menu = menu.separator();
                }
                menu = menu.item(ContextMenuEntry::new(label).disabled(busy).handler(
                    move |window, cx| {
                        owner
                            .update(cx, |panel, cx| {
                                if !panel.state.query_target_is_live(&target) {
                                    return;
                                }
                                match action {
                                    Structure => panel.request_table_structure(
                                        target.clone(),
                                        table.clone().expect("table action"),
                                        cx,
                                    ),
                                    Copy => panel.copy_table(
                                        target.clone(),
                                        table.clone().expect("table action"),
                                        cx,
                                    ),
                                    Backup => panel.request_backup(
                                        target.clone(),
                                        Some(vec![table.clone().expect("table action")]),
                                        cx,
                                    ),
                                    CreateTable | CreateSchema => panel.request_object_mutation(
                                        ObjectMutationFormMode::Create {
                                            target: target.clone(),
                                            kind: if matches!(action, CreateTable) {
                                                DatabaseObjectKind::Table
                                            } else {
                                                DatabaseObjectKind::Schema
                                            },
                                            schema: schema.clone().or_else(|| {
                                                table.as_ref().and_then(|table| {
                                                    table.schema().map(str::to_owned)
                                                })
                                            }),
                                        },
                                        cx,
                                    ),
                                    RenameTable => panel.request_rename_object(
                                        target.clone(),
                                        DatabaseObjectKind::Table,
                                        table.as_ref().expect("table action").to_string(),
                                        cx,
                                    ),
                                    RenameSchema => panel.request_rename_object(
                                        target.clone(),
                                        DatabaseObjectKind::Schema,
                                        schema.clone().expect("schema action"),
                                        cx,
                                    ),
                                    DropTable => panel.confirm_drop_object(
                                        target.clone(),
                                        ObjectMutation::Drop(DropObjectTarget::Table(
                                            table.as_ref().expect("table action").to_string(),
                                        )),
                                        window,
                                        cx,
                                    ),
                                    DropSchema => panel.confirm_drop_object(
                                        target.clone(),
                                        ObjectMutation::Drop(DropObjectTarget::Schema(
                                            schema.clone().expect("schema action"),
                                        )),
                                        window,
                                        cx,
                                    ),
                                }
                            })
                            .ok();
                    },
                ));
            }
            menu
        });
        let previous = window.focused(cx);
        window.focus(&menu.focus_handle(cx), cx);
        let subscription = cx.subscribe_in(
            &menu,
            window,
            move |panel, menu, _: &gpui_kit::DismissEvent, window, cx| {
                if menu.focus_handle(cx).contains_focused(window, cx) {
                    if let Some(previous) = previous.as_ref() {
                        window.focus(previous, cx);
                    }
                }
                panel.context_menu = None;
                cx.notify();
            },
        );
        self.profile_menu_state = None;
        self.context_menu = Some((menu, position, subscription));
        cx.notify();
    }
}
