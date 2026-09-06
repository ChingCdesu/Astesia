use crate::ui::components::prelude::*;
use gpui_kit::{ClickEvent, Context, Render, Window};

use crate::application::QueryTarget;
use crate::db::{TableInfo, TableRef};

use super::{ConnectionProfilesEvent, ConnectionProfilesPanel, NoticeTone, PanelNotice};

#[derive(Clone, Debug)]
pub(super) struct DraggedTableCopy {
    pub(super) source: QueryTarget,
    pub(super) table: TableRef,
}

pub(super) struct DraggedTableCopyPreview {
    pub(super) label: String,
}

impl Render for DraggedTableCopyPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().colors().border)
            .bg(cx.theme().colors().panel_background)
            .child(Label::new(self.label.clone()).size(LabelSize::XSmall))
    }
}

impl ConnectionProfilesPanel {
    pub(super) fn search_redis_keys(
        &mut self,
        target: QueryTarget,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.redis_search_busy || !self.state.query_target_is_live(&target) {
            return;
        }
        self.redis_search_generation = self.redis_search_generation.saturating_add(1);
        let generation = self.redis_search_generation;
        self.redis_search_busy = true;
        self.notify_sidebar(cx);
        let contains = self.redis_search.read(cx).text(cx);
        let application = self.application.clone();
        let target_for_task = target.clone();
        let search = crate::ui::runtime::spawn(cx, async move {
            application
                .redis()
                .scan_keys(&target_for_task, &contains)
                .await
        });
        cx.spawn(async move |panel, cx| {
            let result = match search.await {
                Ok(result) => result.map(|keys| {
                    keys.into_iter()
                        .map(|key| TableInfo {
                            reference: TableRef::unqualified(key),
                            row_count: None,
                            comment: Some("key".to_string()),
                        })
                        .collect()
                }),
                Err(error) => Err(format!("Redis search task ended unexpectedly: {error}")),
            };
            panel
                .update(cx, |panel, cx| {
                    if panel.redis_search_generation != generation
                        || !panel.state.query_target_is_live(&target)
                    {
                        return;
                    }
                    panel.redis_search_busy = false;
                    panel.redis_search_result = Some((target, result));
                    panel.notify_sidebar(cx);
                })
                .ok();
        })
        .detach();
    }

    pub(super) fn request_backup(
        &mut self,
        target: QueryTarget,
        tables: Option<Vec<TableRef>>,
        cx: &mut Context<Self>,
    ) {
        if target.db_type.capabilities().backup && self.state.query_target_is_live(&target) {
            cx.emit(ConnectionProfilesEvent::BackupRequested { target, tables });
        }
    }

    pub(super) fn request_restore(&mut self, target: QueryTarget, cx: &mut Context<Self>) {
        if target.db_type.capabilities().restore && self.state.query_target_is_live(&target) {
            cx.emit(ConnectionProfilesEvent::RestoreRequested { target });
        }
    }

    pub(super) fn request_performance(&mut self, target: QueryTarget, cx: &mut Context<Self>) {
        if target.db_type.capabilities().performance == crate::db::PerformanceMode::Native
            && self.state.query_target_is_live(&target)
        {
            cx.emit(ConnectionProfilesEvent::PerformanceRequested { target });
        }
    }

    pub(super) fn request_er_diagram(&mut self, target: QueryTarget, cx: &mut Context<Self>) {
        if target.db_type.capabilities().foreign_keys && self.state.query_target_is_live(&target) {
            cx.emit(ConnectionProfilesEvent::ErDiagramRequested { target });
        }
    }

    pub(super) fn copy_table(
        &mut self,
        source: QueryTarget,
        table: TableRef,
        cx: &mut Context<Self>,
    ) {
        if source.db_type.capabilities().table_copy == crate::db::TableCopyMode::None
            || !self.state.query_target_is_live(&source)
        {
            return;
        }
        let identity = format!("{} / {} / {table}", source.connection_name, source.database);
        self.copied_table = Some(DraggedTableCopy { source, table });
        self.set_notice(
            NoticeTone::Info,
            format!("Copied {identity}; choose a target database"),
        );
        self.notify_sidebar(cx);
    }

    pub(super) fn request_dragged_table_copy(
        &mut self,
        copy: &DraggedTableCopy,
        target: QueryTarget,
        cx: &mut Context<Self>,
    ) {
        if copy.source.db_type == target.db_type
            && target.db_type.capabilities().table_copy != crate::db::TableCopyMode::None
            && self.state.query_target_is_live(&copy.source)
            && self.state.query_target_is_live(&target)
        {
            cx.emit(ConnectionProfilesEvent::CopyTableRequested {
                source: copy.source.clone(),
                target,
                table: copy.table.clone(),
            });
        }
    }

    pub(in crate::ui) fn refresh_target_objects(
        &mut self,
        target: QueryTarget,
        cx: &mut Context<Self>,
    ) {
        if !self.state.query_target_is_live(&target) {
            return;
        }
        let application = self.application.clone();
        let refresh_target = target.clone();
        let refresh = crate::ui::runtime::spawn(cx, async move {
            application
                .catalog()
                .refresh_schema(
                    &refresh_target.connection_id,
                    Some(&refresh_target.database),
                )
                .await
        });
        cx.spawn(async move |panel, cx| {
            let result = refresh.await.unwrap_or_else(|error| Err(error.to_string()));
            panel
                .update(cx, |panel, cx| {
                    if !panel.state.query_target_is_live(&target) {
                        return;
                    }
                    if let Err(error) = result {
                        panel.notice = Some(PanelNotice {
                            tone: NoticeTone::Error,
                            message: error,
                        });
                        panel.notify_sidebar(cx);
                        return;
                    }
                    panel
                        .application
                        .query_completions()
                        .invalidate_connection(&target.connection_id);
                    panel.refresh_table_details(&target, cx);
                    panel.state.clear_object_state(&target);
                    panel.load_objects(target, cx);
                })
                .ok();
        })
        .detach();
    }
}
