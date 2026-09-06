use crate::ui::components::prelude::*;
use gpui_kit::{PromptButton, PromptLevel, Window};

use super::{ConnectionProfilesEvent, ConnectionProfilesPanel, NoticeTone, PanelNotice};
use crate::application::{DatabaseObjectKind, ObjectMutation, QueryTarget};
use crate::ui::localization::text;
use crate::ui::object_mutation_form::{kind_label, ObjectMutationFormMode};

impl ConnectionProfilesPanel {
    pub(in crate::ui) fn object_mutated(
        &mut self,
        target: QueryTarget,
        database_list_changed: bool,
        cx: &mut Context<Self>,
    ) {
        if database_list_changed {
            self.state.clear_database_state(&target.connection_id);
            self.load_databases(target.connection_id, cx);
        } else {
            self.state.clear_object_state(&target);
            self.load_objects(target, cx);
        }
    }

    pub(super) fn request_object_mutation(
        &mut self,
        mode: ObjectMutationFormMode,
        cx: &mut Context<Self>,
    ) {
        if !self.object_operation_in_progress && self.state.query_target_is_live(mode.target()) {
            cx.emit(ConnectionProfilesEvent::ObjectMutationRequested(mode));
        }
    }

    pub(super) fn request_rename_object(
        &mut self,
        target: QueryTarget,
        kind: DatabaseObjectKind,
        name: String,
        cx: &mut Context<Self>,
    ) {
        self.request_object_mutation(
            ObjectMutationFormMode::Rename {
                target,
                kind,
                original_name: name,
            },
            cx,
        );
    }

    pub(super) fn confirm_drop_object(
        &mut self,
        target: QueryTarget,
        mutation: ObjectMutation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.object_operation_in_progress || window.has_active_prompt() {
            return;
        }
        let language = self.settings.read(cx).language();
        let kind = mutation.kind();
        let identity = mutation.display_identity();
        let message = format!(
            "{} {} “{identity}”?",
            text(language, "删除", "Drop"),
            kind_label(kind, language)
        );
        let detail = if target.db_type == crate::db::DbType::PostgreSQL
            && kind == DatabaseObjectKind::Schema
        {
            text(
                language,
                "这会级联删除该 Schema 中的全部对象，且无法撤销。",
                "This also drops every object in the schema and cannot be undone.",
            )
        } else {
            text(
                language,
                "该操作会立即执行且无法撤销。",
                "This runs immediately and cannot be undone.",
            )
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some(detail),
            &[
                PromptButton::ok(text(language, "删除", "Drop")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |panel, cx| {
            if answer.await.ok() != Some(0) {
                return;
            }
            panel
                .update_in(cx, |panel, _, cx| {
                    panel.drop_object(target, mutation, cx);
                })
                .ok();
        })
        .detach();
    }

    pub(super) fn drop_object(
        &mut self,
        target: QueryTarget,
        mutation: ObjectMutation,
        cx: &mut Context<Self>,
    ) {
        if self.object_operation_in_progress || !self.state.query_target_is_live(&target) {
            return;
        }
        self.object_operation_in_progress = true;
        self.notice = None;
        cx.notify();
        let application = self.application.clone();
        let operation_target = target.clone();
        let kind = mutation.kind();
        let identity = mutation.display_identity();
        let operation = crate::ui::runtime::spawn(cx, async move {
            application
                .objects()
                .execute(&operation_target, &mutation)
                .await
        });
        cx.spawn(async move |panel, cx| {
            let result = match operation.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            panel
                .update(cx, |panel, cx| {
                    panel.object_operation_in_progress = false;
                    match result {
                        Ok(()) => {
                            panel.notice = Some(PanelNotice {
                                tone: NoticeTone::Info,
                                message: format!(
                                    "{}: {identity}",
                                    text(
                                        panel.settings.read(cx).language(),
                                        "数据库对象已删除",
                                        "Database object dropped"
                                    )
                                ),
                            });
                            panel.object_mutated(target, kind == DatabaseObjectKind::Database, cx);
                        }
                        Err(error) => {
                            panel.notice = Some(PanelNotice {
                                tone: NoticeTone::Error,
                                message: error,
                            });
                            cx.notify();
                        }
                    }
                })
                .ok();
        })
        .detach();
    }
}
