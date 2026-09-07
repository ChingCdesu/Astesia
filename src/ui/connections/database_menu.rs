use super::*;
use crate::application::{
    object_kind_can_create, object_kind_can_drop, object_kind_can_rename, DatabaseObjectKind,
    DropObjectTarget, ObjectMutation,
};
use crate::ui::components::{ContextMenu, ContextMenuEntry};
use gpui_kit::Focusable;

#[derive(Clone, Copy)]
enum DatabaseAction {
    Er,
    Performance,
    Backup,
    Restore,
    Paste,
    Create(DatabaseObjectKind),
    Rename,
    Drop,
    Refresh,
}

impl ConnectionProfilesPanel {
    pub(super) fn open_database_menu(
        &mut self,
        target: QueryTarget,
        control: Option<QueryTarget>,
        position: gpui_kit::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.profile_menu_state = None;
        use DatabaseAction::*;
        let language = self.settings.read(cx).language();
        let engine = target.db_type;
        let capabilities = engine.capabilities();
        let busy = self.actions_blocked();
        let can_paste = self
            .copied_table
            .as_ref()
            .is_some_and(|copy| copy.source.db_type == engine);
        let owner = cx.entity().downgrade();
        let mut actions = vec![Er, Performance, Backup, Restore, Paste, Refresh];
        actions.extend(
            [
                DatabaseObjectKind::Database,
                DatabaseObjectKind::Schema,
                DatabaseObjectKind::Table,
                DatabaseObjectKind::View,
                DatabaseObjectKind::Function,
                DatabaseObjectKind::Procedure,
                DatabaseObjectKind::Trigger,
                DatabaseObjectKind::User,
            ]
            .into_iter()
            .map(Create),
        );
        actions.extend([Rename, Drop]);
        let menu = ContextMenu::build(window, cx, |mut menu, _, _| {
            for action in actions {
                let supported = match action {
                    Er => capabilities.foreign_keys,
                    Performance => capabilities.performance == crate::db::PerformanceMode::Native,
                    Backup => capabilities.backup,
                    Restore => capabilities.restore,
                    Paste => can_paste && capabilities.table_copy != crate::db::TableCopyMode::None,
                    Create(kind) => object_kind_can_create(engine, kind),
                    Rename => {
                        control.is_some()
                            && object_kind_can_rename(engine, DatabaseObjectKind::Database)
                    }
                    Drop => {
                        control.is_some()
                            && object_kind_can_drop(engine, DatabaseObjectKind::Database)
                    }
                    Refresh => true,
                };
                if !supported {
                    continue;
                }
                let label = match action {
                    Er => text(language, "实体关系图", "Entity Relationship Diagram"),
                    Performance => text(language, "性能监控", "Performance Monitor"),
                    Backup => text(language, "备份数据库", "Back Up Database"),
                    Restore => text(language, "恢复数据库…", "Restore Database…"),
                    Paste => text(language, "粘贴表…", "Paste Table…"),
                    Refresh => text(language, "刷新对象", "Refresh Objects"),
                    Rename => text(language, "重命名数据库…", "Rename Database…"),
                    Drop => text(language, "删除数据库…", "Drop Database…"),
                    Create(kind) => match kind {
                        DatabaseObjectKind::Database => {
                            text(language, "新建数据库…", "Create Database…")
                        }
                        DatabaseObjectKind::Schema => {
                            text(language, "新建 Schema…", "Create Schema…")
                        }
                        DatabaseObjectKind::Table => text(language, "新建表…", "Create Table…"),
                        DatabaseObjectKind::View => text(language, "新建视图…", "Create View…"),
                        DatabaseObjectKind::Function => {
                            text(language, "新建函数…", "Create Function…")
                        }
                        DatabaseObjectKind::Procedure => {
                            text(language, "新建存储过程…", "Create Procedure…")
                        }
                        DatabaseObjectKind::Trigger => {
                            text(language, "新建触发器…", "Create Trigger…")
                        }
                        DatabaseObjectKind::User => text(language, "新建用户…", "Create User…"),
                    },
                };
                let owner = owner.clone();
                let target = target.clone();
                let control = control.clone();
                if matches!(action, Drop) {
                    menu = menu.separator();
                }
                menu = menu.item(
                    ContextMenuEntry::new(label)
                        .disabled(busy && !matches!(action, Refresh))
                        .handler(move |window, cx| {
                            owner
                                .update(cx, |panel, cx| {
                                    if !panel.state.query_target_is_live(&target) {
                                        return;
                                    }
                                    match action {
                                        Er => panel.request_er_diagram(target.clone(), cx),
                                        Performance => {
                                            panel.request_performance(target.clone(), cx)
                                        }
                                        Backup => panel.request_backup(target.clone(), None, cx),
                                        Restore => panel.request_restore(target.clone(), cx),
                                        Paste => {
                                            if let Some(copy) = panel.copied_table.clone() {
                                                panel.request_dragged_table_copy(
                                                    &copy,
                                                    target.clone(),
                                                    cx,
                                                );
                                            }
                                        }
                                        Refresh => panel.refresh_target_objects(target.clone(), cx),
                                        Create(kind) => panel.request_object_mutation(
                                            ObjectMutationFormMode::Create {
                                                target: target.clone(),
                                                kind,
                                                schema: None,
                                            },
                                            cx,
                                        ),
                                        Rename => panel.request_rename_object(
                                            control.clone().expect("control database"),
                                            DatabaseObjectKind::Database,
                                            target.database.clone(),
                                            cx,
                                        ),
                                        Drop => panel.confirm_drop_object(
                                            control.clone().expect("control database"),
                                            ObjectMutation::Drop(DropObjectTarget::Database(
                                                target.database.clone(),
                                            )),
                                            window,
                                            cx,
                                        ),
                                    }
                                })
                                .ok();
                        }),
                );
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
        self.context_menu = Some((menu, position, subscription));
        cx.notify();
    }
}
