use std::path::PathBuf;

use gpui_kit::{AppContext as _, Context, PathPromptOptions, PromptButton, PromptLevel, Window};

use crate::application::{BackupContent, BackupOptions, DropTableMode, QueryTarget};
use crate::db::TableRef;

use super::{AstesiaWorkspace, WorkspaceItem, WorkspaceItemKey};
use crate::ui::{
    copy_table_form::{CopyTableForm, TransferTaskStarted},
    document_item::DocumentItem,
    er_diagram_item::ErDiagramItem,
    localization::text,
    mcp_service_item::McpServiceItem,
    performance_item::PerformanceItem,
    redis_item::{RedisItem, RedisKeyDeleted},
    shell::NotificationTone,
    task_center_item::TaskCenterItem,
};

impl AstesiaWorkspace {
    pub(super) fn open_copy_table_form(
        &mut self,
        source: QueryTarget,
        target: QueryTarget,
        table: TableRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let application = self.application.clone();
        let language = self.settings.read(cx).language();
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle_modal(window, cx, move |window, cx| {
                CopyTableForm::new(application, source, target, table, language, window, cx)
            });
        });
        let Some(form) = self.modal_layer.read(cx).active_modal::<CopyTableForm>() else {
            return;
        };
        self.copy_table_form_subscription = Some(cx.subscribe_in(
            &form,
            window,
            |workspace, _, event: &TransferTaskStarted, window, cx| {
                workspace.finish_transfer_start(Ok(event.task_id.clone()), window, cx);
            },
        ));
    }

    pub(super) fn open_document_collection(
        &mut self,
        target: QueryTarget,
        collection: TableRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = WorkspaceItemKey::Document(target.clone(), collection.clone());
        self.open_or_activate(key, window, cx, move |workspace, window, cx| {
            let item = cx.new(|cx| {
                DocumentItem::new(
                    workspace.application.clone(),
                    target,
                    collection,
                    workspace.settings.clone(),
                    window,
                    cx,
                )
            });
            (WorkspaceItem::new(item), Vec::new())
        });
    }

    pub(super) fn open_redis_key(
        &mut self,
        target: QueryTarget,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let item_key = WorkspaceItemKey::Redis(target.clone(), key.clone());
        self.open_or_activate(item_key, window, cx, move |workspace, window, cx| {
            let item = cx.new(|cx| {
                RedisItem::new(
                    workspace.application.clone(),
                    target,
                    key,
                    workspace.settings.clone(),
                    window,
                    cx,
                )
            });
            let deletion_subscription =
                cx.subscribe(&item, |workspace, _, event: &RedisKeyDeleted, cx| {
                    workspace.connection_profiles.update(cx, |panel, cx| {
                        panel.refresh_target_objects(event.target.clone(), cx);
                    });
                });
            (WorkspaceItem::new(item), vec![deletion_subscription])
        });
    }

    pub(super) fn open_task_center(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_or_activate(
            WorkspaceItemKey::TaskCenter,
            window,
            cx,
            |workspace, _, cx| {
                let item = cx.new(|cx| {
                    TaskCenterItem::new(
                        workspace.application.clone(),
                        workspace.settings.clone(),
                        cx,
                    )
                });
                let observation = cx.observe(&item, |_, _, cx| cx.notify());
                (WorkspaceItem::new(item), vec![observation])
            },
        );
    }

    pub(super) fn open_performance(
        &mut self,
        target: QueryTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = WorkspaceItemKey::Performance(target.clone());
        self.open_or_activate(key, window, cx, move |workspace, _, cx| {
            let item = cx.new(|cx| {
                PerformanceItem::new(
                    workspace.application.clone(),
                    target,
                    workspace.settings.clone(),
                    cx,
                )
            });
            (WorkspaceItem::new(item), Vec::new())
        });
    }

    pub(super) fn open_er_diagram(
        &mut self,
        target: QueryTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = WorkspaceItemKey::ErDiagram(target.clone());
        self.open_or_activate(key, window, cx, move |workspace, _, cx| {
            let item = cx.new(|cx| {
                ErDiagramItem::new(
                    workspace.application.clone(),
                    target,
                    workspace.settings.clone(),
                    cx,
                )
            });
            (WorkspaceItem::new(item), Vec::new())
        });
    }

    pub(super) fn open_mcp_service(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_or_activate(
            WorkspaceItemKey::McpService,
            window,
            cx,
            |workspace, _, cx| {
                let item = cx.new(|cx| {
                    McpServiceItem::new(
                        workspace.application.clone(),
                        workspace.settings.clone(),
                        cx,
                    )
                });
                (WorkspaceItem::new(item), Vec::new())
            },
        );
    }

    pub(super) fn choose_backup_content(
        &mut self,
        target: QueryTarget,
        tables: Option<Vec<TableRef>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_prompt() {
            return;
        }
        let language = self.settings.read(cx).language();
        let answer = window.prompt(
            PromptLevel::Info,
            text(language, "选择备份内容", "Choose backup content"),
            Some(text(
                language,
                "结构备份会包含可恢复的 DROP IF EXISTS 语句。",
                "Structure backups include restorable DROP IF EXISTS statements.",
            )),
            &[
                PromptButton::ok(text(language, "结构和数据", "Structure + Data")),
                PromptButton::new(text(language, "仅结构", "Structure Only")),
                PromptButton::new(text(language, "仅数据", "Data Only")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |workspace, cx| {
            let choice = answer.await.ok();
            workspace
                .update_in(cx, |workspace, window, cx| {
                    let content = match choice {
                        Some(0) => BackupContent::StructureAndData,
                        Some(1) => BackupContent::Structure,
                        Some(2) => BackupContent::Data,
                        _ => return,
                    };
                    if content == BackupContent::Data {
                        workspace.choose_backup_path(
                            target,
                            tables,
                            content,
                            DropTableMode::None,
                            window,
                            cx,
                        );
                    } else {
                        workspace.choose_backup_drop(target, tables, content, window, cx);
                    }
                })
                .ok();
        })
        .detach();
    }

    fn choose_backup_drop(
        &mut self,
        target: QueryTarget,
        tables: Option<Vec<TableRef>>,
        content: BackupContent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let language = self.settings.read(cx).language();
        let answer = window.prompt(
            PromptLevel::Info,
            text(
                language,
                "选择恢复前的删除方式",
                "Choose pre-restore drop behavior",
            ),
            Some(text(
                language,
                "删除语句会写入备份文件，并在恢复时执行。",
                "Drop statements are written into the backup and run during restore.",
            )),
            &[
                PromptButton::ok("DROP TABLE IF EXISTS"),
                PromptButton::new("DROP TABLE"),
                PromptButton::new(text(language, "不删除", "No Drop")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |workspace, cx| {
            let drop_tables = match answer.await.ok() {
                Some(0) => Some(DropTableMode::DropIfExists),
                Some(1) => Some(DropTableMode::Drop),
                Some(2) => Some(DropTableMode::None),
                _ => None,
            };
            if let Some(drop_tables) = drop_tables {
                workspace
                    .update_in(cx, |workspace, window, cx| {
                        workspace.choose_backup_path(
                            target,
                            tables,
                            content,
                            drop_tables,
                            window,
                            cx,
                        );
                    })
                    .ok();
            }
        })
        .detach();
    }

    fn choose_backup_path(
        &mut self,
        target: QueryTarget,
        tables: Option<Vec<TableRef>>,
        content: BackupContent,
        drop_tables: DropTableMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default_name = tables
            .as_ref()
            .and_then(|tables| (tables.len() == 1).then(|| tables[0].name().to_string()))
            .unwrap_or_else(|| target.database.clone());
        let prompt = cx.prompt_for_new_path(
            &PathBuf::default(),
            Some(&format!("{default_name}-backup.sql")),
        );
        cx.spawn_in(window, async move |workspace, cx| {
            let path = match prompt.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) => return,
                Ok(Err(error)) => {
                    workspace
                        .update_in(cx, |workspace, _, cx| {
                            workspace.notify_transfer_error(error.to_string(), cx);
                        })
                        .ok();
                    return;
                }
                Err(error) => {
                    workspace
                        .update_in(cx, |workspace, _, cx| {
                            workspace.notify_transfer_error(error.to_string(), cx);
                        })
                        .ok();
                    return;
                }
            };
            let options = BackupOptions {
                tables,
                content,
                drop_tables,
                output_path: path.to_string_lossy().into_owned(),
            };
            let transfer = workspace
                .read_with(cx, |workspace, _| workspace.application.transfers().clone())
                .ok();
            let Some(transfer) = transfer else {
                return;
            };
            let Ok(start) = cx.update(|_, cx| {
                crate::ui::runtime::spawn(cx, async move {
                    transfer.start_backup(target, options).await
                })
            }) else {
                return;
            };
            let result = match start.await {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
            workspace
                .update_in(cx, |workspace, window, cx| {
                    workspace.finish_transfer_start(result, window, cx);
                })
                .ok();
        })
        .detach();
    }

    pub(super) fn choose_restore_file(
        &mut self,
        target: QueryTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let language = self.settings.read(cx).language();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(text(language, "选择 SQL 备份文件", "Choose SQL backup file").into()),
        });
        cx.spawn_in(window, async move |workspace, cx| {
            let path = match prompt.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    workspace
                        .update_in(cx, |workspace, _, cx| {
                            workspace.notify_transfer_error(error.to_string(), cx);
                        })
                        .ok();
                    return;
                }
                Err(error) => {
                    workspace
                        .update_in(cx, |workspace, _, cx| {
                            workspace.notify_transfer_error(error.to_string(), cx);
                        })
                        .ok();
                    return;
                }
            };
            let Some(path) = path else {
                return;
            };
            let transfer = workspace
                .read_with(cx, |workspace, _| workspace.application.transfers().clone())
                .ok();
            let Some(transfer) = transfer else {
                return;
            };
            let Ok(start) = cx.update(|_, cx| {
                crate::ui::runtime::spawn(cx, async move {
                    transfer
                        .start_restore(target, path.to_string_lossy().into_owned())
                        .await
                })
            }) else {
                return;
            };
            let result = match start.await {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
            workspace
                .update_in(cx, |workspace, window, cx| {
                    workspace.finish_transfer_start(result, window, cx);
                })
                .ok();
        })
        .detach();
    }

    fn finish_transfer_start(
        &mut self,
        result: Result<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(task_id) => {
                let language = self.settings.read(cx).language();
                self.notifications.update(cx, |center, cx| {
                    center.push(
                        NotificationTone::Info,
                        format!(
                            "{}: {task_id}",
                            text(language, "后台任务已启动", "Background task started")
                        ),
                        cx,
                    );
                });
                self.open_task_center(window, cx);
            }
            Err(error) => self.notify_transfer_error(error, cx),
        }
    }

    fn notify_transfer_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.notifications.update(cx, |center, cx| {
            center.push(NotificationTone::Error, error, cx);
        });
    }
}
