mod catalog_view;
mod engine_workflows;
mod object_actions;
mod presentation;
mod view;

use std::sync::Arc;

use gpui::{App, ClickEvent, Entity, EventEmitter, PromptButton, PromptLevel, Subscription, Task};
use ui_input::InputField;
use zed_ui::prelude::*;

use crate::application::connection_workspace::{
    ConnectionWorkspaceError, ConnectionWorkspaceState, OperationApply, ProfileOperationKind,
    ProfileOperationRequest, SnapshotApply,
};
use crate::application::{
    Application, ConnectionOutcome, ConnectionProfileSnapshot, ConnectionWorkspaceSnapshot,
    ProfileOperationCommand, ProfileOperationCompletion, ProfileOperationOutcome, QueryTarget,
};
use crate::connection_repository::SharedConnectionProfile;
use crate::db::{TableInfo, TableRef};
use crate::platform::UiEvent;

use self::engine_workflows::DraggedTableCopy;
use self::presentation::repository_error_message;
use super::engine_presentation::engine_label;
use super::localization::text;
use super::object_definition_item::ObjectDefinition;
use super::object_mutation_form::ObjectMutationFormMode;
use super::shell::ShellSettings;

pub(super) const SIDEBAR_WIDTH: Pixels = px(272.0);

pub(super) fn bind_connection_profiles_keys(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("enter", menu::Confirm, Some("ConnectionProfileRow")),
        gpui::KeyBinding::new("space", menu::Confirm, Some("ConnectionProfileRow")),
        gpui::KeyBinding::new("enter", menu::Confirm, Some("QueryTargetRow")),
        gpui::KeyBinding::new("space", menu::Confirm, Some("QueryTargetRow")),
        gpui::KeyBinding::new("enter", menu::Confirm, Some("SchemaObjectRow")),
        gpui::KeyBinding::new("space", menu::Confirm, Some("SchemaObjectRow")),
    ]);
}

pub(super) struct ConnectionProfilesPanel {
    application: Arc<Application>,
    state: ConnectionWorkspaceState,
    selected_profile_id: Option<String>,
    selected_query_target: Option<QueryTarget>,
    notice: Option<PanelNotice>,
    object_operation_in_progress: bool,
    redis_search: Entity<InputField>,
    redis_search_result: Option<(QueryTarget, Result<Vec<TableInfo>, String>)>,
    redis_search_generation: u64,
    redis_search_busy: bool,
    copied_table: Option<DraggedTableCopy>,
    settings: Entity<ShellSettings>,
    _redis_search_observation: Subscription,
    _settings_observation: Subscription,
    _application_events: Task<()>,
}

#[derive(Clone, Debug)]
pub(super) enum ConnectionProfilesEvent {
    CreateRequested,
    EditRequested(Arc<SharedConnectionProfile>),
    QueryTargetSelected(QueryTarget),
    TableStructureRequested {
        target: QueryTarget,
        table: TableRef,
    },
    TableDataRequested {
        target: QueryTarget,
        table: TableRef,
    },
    DocumentCollectionRequested {
        target: QueryTarget,
        collection: TableRef,
    },
    RedisKeyRequested {
        target: QueryTarget,
        key: String,
    },
    BackupRequested {
        target: QueryTarget,
        tables: Option<Vec<TableRef>>,
    },
    RestoreRequested {
        target: QueryTarget,
    },
    PerformanceRequested {
        target: QueryTarget,
    },
    ErDiagramRequested {
        target: QueryTarget,
    },
    CopyTableRequested {
        source: QueryTarget,
        target: QueryTarget,
        table: TableRef,
    },
    ObjectDefinitionRequested(ObjectDefinition),
    ObjectMutationRequested(ObjectMutationFormMode),
    QueryTargetInvalidated(QueryTarget),
    QuerySessionInvalidated {
        connection_id: String,
        session_generation: u64,
    },
    QuerySessionsChanged(Arc<ConnectionWorkspaceSnapshot>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ConnectionProfilesStatus {
    pub(super) summary: String,
    pub(super) session: ConnectionSessionStatus,
    pub(super) activity: ConnectionActivityStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConnectionSessionStatus {
    Loading,
    NoSelection,
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
    Deleting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConnectionActivityStatus {
    Loading,
    Refreshing,
    LoadingDatabases,
    LoadingObjects,
    Working,
    NeedsRefresh,
    Ready,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NoticeTone {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PanelNotice {
    tone: NoticeTone,
    message: String,
}

impl EventEmitter<ConnectionProfilesEvent> for ConnectionProfilesPanel {}

impl ConnectionProfilesPanel {
    pub(super) fn new(
        application: Arc<Application>,
        settings: Entity<ShellSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let language = settings.read(cx).language();
        let redis_search = cx.new(|cx| {
            InputField::new(
                window,
                cx,
                text(language, "搜索 Redis 键", "Search Redis keys"),
            )
            .label(text(language, "Redis 键搜索", "Redis key search"))
        });
        let redis_search_observation = cx.observe(&redis_search, |_, _, cx| cx.notify());
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let mut application_events = application.subscribe_events();
        let application_event_task = cx.spawn(async move |panel, cx| loop {
            let refresh = match application_events.recv().await {
                Ok(UiEvent::McpConnectionsChanged(_)) => true,
                Ok(UiEvent::TaskProgress { .. } | UiEvent::TaskCompleted { .. }) => false,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if refresh {
                panel
                    .update(cx, |panel, cx| panel.refresh_profiles(cx))
                    .ok();
            }
        });
        let mut panel = Self {
            application,
            state: ConnectionWorkspaceState::default(),
            selected_profile_id: None,
            selected_query_target: None,
            notice: None,
            object_operation_in_progress: false,
            redis_search,
            redis_search_result: None,
            redis_search_generation: 0,
            redis_search_busy: false,
            copied_table: None,
            settings,
            _redis_search_observation: redis_search_observation,
            _settings_observation: settings_observation,
            _application_events: application_event_task,
        };
        panel.refresh_profiles(cx);
        panel
    }

    pub(super) fn profile_saved(&mut self, profile_id: String, cx: &mut Context<Self>) {
        self.selected_profile_id = Some(profile_id);
        self.refresh_profiles(cx);
    }

    pub(super) fn status(&self, cx: &App) -> ConnectionProfilesStatus {
        derive_status(
            &self.state,
            self.selected_profile_id.as_deref(),
            self.selected_query_target.as_ref(),
            self.object_operation_in_progress,
            self.settings.read(cx).language(),
        )
    }

    pub(super) fn query_target(&self) -> Option<&QueryTarget> {
        self.selected_query_target.as_ref()
    }

    fn selected_profile(&self) -> Option<&ConnectionProfileSnapshot> {
        let selected = self.selected_profile_id.as_deref()?;
        self.state
            .snapshot()?
            .profiles
            .iter()
            .find(|profile| profile.profile.id == selected)
    }

    fn actions_blocked(&self) -> bool {
        self.state.is_refreshing()
            || self.state.error().is_some()
            || self.object_operation_in_progress
    }

    fn reconcile_selection(&mut self) {
        reconcile_selected_profile(&mut self.selected_profile_id, self.state.snapshot());
    }

    fn reconcile_query_target(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.selected_query_target.as_ref() else {
            return;
        };
        if !self.state.query_target_is_live(target) {
            self.clear_query_target(cx);
        }
    }

    fn select_database(&mut self, target: QueryTarget, cx: &mut Context<Self>) {
        if self.selected_query_target.as_ref() != Some(&target) {
            self.selected_query_target = Some(target.clone());
            cx.emit(ConnectionProfilesEvent::QueryTargetSelected(target.clone()));
        }
        self.load_objects(target, cx);
        cx.notify();
    }

    fn request_table_structure(
        &mut self,
        target: QueryTarget,
        table: TableRef,
        cx: &mut Context<Self>,
    ) {
        if target.db_type.capabilities().sql && self.state.query_target_is_live(&target) {
            cx.emit(ConnectionProfilesEvent::TableStructureRequested { target, table });
        }
    }

    fn request_primary_data(
        &mut self,
        target: QueryTarget,
        object: TableRef,
        cx: &mut Context<Self>,
    ) {
        if !self.state.query_target_is_live(&target) {
            return;
        }
        match target.db_type {
            crate::db::DbType::MongoDB => {
                cx.emit(ConnectionProfilesEvent::DocumentCollectionRequested {
                    target,
                    collection: object,
                });
            }
            crate::db::DbType::Redis => {
                cx.emit(ConnectionProfilesEvent::RedisKeyRequested {
                    target,
                    key: object.name().to_string(),
                });
            }
            _ if target.db_type.capabilities().sql => {
                cx.emit(ConnectionProfilesEvent::TableDataRequested {
                    target,
                    table: object,
                });
            }
            _ => {}
        }
    }

    fn request_object_definition(&mut self, object: ObjectDefinition, cx: &mut Context<Self>) {
        if object.target.db_type.capabilities().sql
            && self.state.query_target_is_live(&object.target)
        {
            cx.emit(ConnectionProfilesEvent::ObjectDefinitionRequested(object));
        }
    }

    fn invalidate_query_session(&mut self, connection_id: &str, cx: &mut Context<Self>) {
        let selected_session_generation = self
            .selected_query_target
            .as_ref()
            .filter(|target| target.connection_id == connection_id)
            .map(|target| target.session_generation);
        let current_session_generation = self
            .state
            .snapshot()
            .and_then(|snapshot| {
                snapshot
                    .profiles
                    .iter()
                    .find(|profile| profile.profile.id == connection_id)
            })
            .and_then(|profile| profile.session.generation);
        if let Some(session_generation) = current_session_generation.or(selected_session_generation)
        {
            cx.emit(ConnectionProfilesEvent::QuerySessionInvalidated {
                connection_id: connection_id.to_string(),
                session_generation,
            });
        }
        if self
            .selected_query_target
            .as_ref()
            .is_some_and(|target| target.connection_id == connection_id)
        {
            self.selected_query_target = None;
            cx.notify();
        }
    }

    fn clear_query_target(&mut self, cx: &mut Context<Self>) {
        if let Some(target) = self.selected_query_target.take() {
            cx.emit(ConnectionProfilesEvent::QueryTargetInvalidated(target));
            cx.notify();
        }
    }

    fn emit_query_sessions(&self, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.state.snapshot() {
            cx.emit(ConnectionProfilesEvent::QuerySessionsChanged(Arc::new(
                snapshot.clone(),
            )));
        }
    }

    fn set_notice(&mut self, tone: NoticeTone, message: impl Into<String>) {
        self.notice = Some(PanelNotice {
            tone,
            message: message.into(),
        });
    }

    fn refresh(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_profiles(cx);
    }

    pub(super) fn refresh_profiles(&mut self, cx: &mut Context<Self>) {
        let request = self.state.begin_refresh();
        cx.notify();

        let application = self.application.clone();
        let load =
            gpui_tokio::Tokio::spawn(
                cx,
                async move { application.connections().snapshot().await },
            );
        cx.spawn(async move |panel, cx| {
            let result = match load.await {
                Ok(snapshot) => snapshot.map_err(ConnectionWorkspaceError::from),
                Err(error) => Err(ConnectionWorkspaceError::load_profiles(error)),
            };
            panel
                .update(cx, |panel, cx| {
                    let applied = panel.state.finish_refresh(request, result);
                    if applied == SnapshotApply::Applied {
                        panel.reconcile_selection();
                        panel.reconcile_query_target(cx);
                        panel.emit_query_sessions(cx);
                    }
                    if applied != SnapshotApply::Superseded {
                        cx.notify();
                    }
                })
                .ok();
        })
        .detach();
    }

    fn select_profile(
        &mut self,
        profile_id: String,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_profile_id(profile_id, cx);
    }

    fn select_profile_id(&mut self, profile_id: String, cx: &mut Context<Self>) {
        let changed = self.selected_profile_id.as_deref() != Some(&profile_id);
        self.selected_profile_id = Some(profile_id.clone());
        if changed {
            cx.notify();
        }
        self.load_databases(profile_id, cx);
    }

    fn create_profile(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.actions_blocked() {
            return;
        }
        cx.emit(ConnectionProfilesEvent::CreateRequested);
    }

    fn edit_selected(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.actions_blocked() {
            return;
        }
        let Some(selected) = self.selected_profile() else {
            return;
        };
        if selected
            .mcp_usage
            .as_ref()
            .is_some_and(|usage| usage.mcp_in_use)
            || self.state.operation(&selected.profile.id).is_some()
        {
            return;
        }
        cx.emit(ConnectionProfilesEvent::EditRequested(Arc::new(
            selected.profile.clone(),
        )));
    }

    fn connect_selected(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(connection_id) = self
            .selected_profile()
            .filter(|profile| !profile.session.is_connected())
            .map(|profile| profile.profile.id.clone())
        else {
            return;
        };
        self.connect(connection_id, cx);
    }

    fn connect(&mut self, connection_id: String, cx: &mut Context<Self>) {
        if self.actions_blocked() {
            return;
        }
        let Some(request) = self
            .state
            .begin_operation(&connection_id, ProfileOperationKind::Connecting)
        else {
            return;
        };
        self.notice = None;
        cx.notify();
        self.run_profile_operation(
            request,
            ProfileOperationCommand::Connect { connection_id },
            cx,
        );
    }

    fn disconnect_selected(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.actions_blocked() {
            return;
        }
        let Some(connection_id) = self
            .selected_profile()
            .filter(|profile| profile.session.is_connected())
            .map(|profile| profile.profile.id.clone())
        else {
            return;
        };
        let Some(request) = self
            .state
            .begin_operation(&connection_id, ProfileOperationKind::Disconnecting)
        else {
            return;
        };
        self.state.clear_database_state(&connection_id);
        self.invalidate_query_session(&connection_id, cx);
        self.notice = None;
        cx.notify();
        self.run_profile_operation(
            request,
            ProfileOperationCommand::Disconnect { connection_id },
            cx,
        );
    }

    fn run_profile_operation(
        &mut self,
        request: ProfileOperationRequest,
        command: ProfileOperationCommand,
        cx: &mut Context<Self>,
    ) {
        let connection_id = command.connection_id().to_string();
        let operation_name = command.operation_name();
        let language = self.settings.read(cx).language();
        let unexpected_message = match &command {
            ProfileOperationCommand::Connect { .. } => text(
                language,
                "连接后台任务意外结束；请刷新后确认当前连接状态。",
                "The connect task ended unexpectedly. Refresh to confirm the current state.",
            ),
            ProfileOperationCommand::Disconnect { .. } => text(
                language,
                "断开后台任务意外结束；请刷新后确认当前连接状态。",
                "The disconnect task ended unexpectedly. Refresh to confirm the current state.",
            ),
            ProfileOperationCommand::Delete { .. } => text(
                language,
                "删除后台任务意外结束；请刷新后确认连接配置是否仍存在。",
                "The delete task ended unexpectedly. Refresh to confirm whether the profile still exists.",
            ),
        };
        let application = self.application.clone();
        let operation = gpui_tokio::Tokio::spawn(cx, async move {
            application
                .connections()
                .perform_profile_operation(command)
                .await
        });

        cx.spawn(async move |panel, cx| {
            let result = operation.await;
            panel
                .update(cx, |panel, cx| match result {
                    Ok(completion) => {
                        panel.apply_profile_operation(&request, completion, &connection_id, cx)
                    }
                    Err(error) => {
                        if panel.state.finish_operation(
                            &request,
                            Err(ConnectionWorkspaceError::operation(operation_name, error)),
                        ) != OperationApply::Discarded
                        {
                            panel.set_notice(NoticeTone::Error, unexpected_message);
                            cx.notify();
                        }
                    }
                })
                .ok();
        })
        .detach();
    }

    fn apply_profile_operation(
        &mut self,
        request: &ProfileOperationRequest,
        completion: ProfileOperationCompletion,
        connection_id: &str,
        cx: &mut Context<Self>,
    ) {
        let apply = self.state.finish_operation(
            request,
            completion.snapshot.map_err(ConnectionWorkspaceError::from),
        );
        let language = self.settings.read(cx).language();
        match apply {
            OperationApply::Discarded => return,
            OperationApply::Snapshot(SnapshotApply::Applied) => {
                self.reconcile_selection();
                self.reconcile_query_target(cx);
                self.emit_query_sessions(cx);
                if self.apply_operation_outcome(completion.outcome, language) {
                    self.load_databases(connection_id.to_string(), cx);
                }
            }
            OperationApply::Snapshot(SnapshotApply::Failed) => {
                self.set_notice(
                    NoticeTone::Error,
                    operation_snapshot_failure_message(&completion.outcome, language),
                );
            }
            OperationApply::Snapshot(SnapshotApply::Superseded) => {
                self.apply_superseded_outcome(completion.outcome, language);
                self.refresh_profiles(cx);
            }
        }
        cx.notify();
    }

    fn apply_operation_outcome(
        &mut self,
        outcome: ProfileOperationOutcome,
        language: crate::platform::UiLanguage,
    ) -> bool {
        match outcome {
            ProfileOperationOutcome::Connected(Ok(ConnectionOutcome::Succeeded)) => {
                self.set_notice(
                    NoticeTone::Info,
                    text(language, "连接成功", "Connected successfully"),
                );
                true
            }
            ProfileOperationOutcome::Connected(Ok(ConnectionOutcome::Rejected(message)))
            | ProfileOperationOutcome::Connected(Err(message)) => {
                self.set_notice(NoticeTone::Error, message);
                false
            }
            ProfileOperationOutcome::Disconnected(report) => {
                let tone = if report.is_complete() {
                    NoticeTone::Info
                } else {
                    NoticeTone::Warning
                };
                self.set_notice(tone, report.message());
                false
            }
            ProfileOperationOutcome::Deleted(Ok(deleted)) => {
                let message = if deleted.credential_cleanup_pending {
                    text(
                        language,
                        "连接配置已删除；系统凭据将在稍后完成清理",
                        "Connection profile deleted; system credentials will be cleaned up later",
                    )
                } else {
                    text(language, "连接配置已删除", "Connection profile deleted")
                };
                self.set_notice(NoticeTone::Info, message);
                false
            }
            ProfileOperationOutcome::Deleted(Err(error)) => {
                self.set_notice(NoticeTone::Error, repository_error_message(&error));
                false
            }
        }
    }

    fn apply_superseded_outcome(
        &mut self,
        outcome: ProfileOperationOutcome,
        language: crate::platform::UiLanguage,
    ) {
        match outcome {
            ProfileOperationOutcome::Connected(_) => self.set_notice(
                NoticeTone::Warning,
                text(
                    language,
                    "连接结果已返回，正在重新同步最新状态…",
                    "The connect result arrived; resyncing the latest state…",
                ),
            ),
            ProfileOperationOutcome::Disconnected(_) => self.set_notice(
                NoticeTone::Warning,
                text(
                    language,
                    "断开结果已返回，正在重新同步最新状态…",
                    "The disconnect result arrived; resyncing the latest state…",
                ),
            ),
            ProfileOperationOutcome::Deleted(Ok(_)) => self.set_notice(
                NoticeTone::Warning,
                text(
                    language,
                    "连接配置已删除，正在重新同步最新列表…",
                    "The connection profile was deleted; resyncing the latest list…",
                ),
            ),
            ProfileOperationOutcome::Deleted(Err(error)) => {
                self.set_notice(NoticeTone::Error, repository_error_message(&error))
            }
        }
    }

    fn load_databases(&mut self, connection_id: String, cx: &mut Context<Self>) {
        let Some(request) = self.state.begin_database_load(&connection_id) else {
            return;
        };
        cx.notify();

        let application = self.application.clone();
        let language = self.settings.read(cx).language();
        let connection_id_for_task = connection_id.clone();
        let load = gpui_tokio::Tokio::spawn(cx, async move {
            application
                .connections()
                .load_databases(&connection_id_for_task)
                .await
        });
        cx.spawn(async move |panel, cx| {
            let result = match load.await {
                Ok(result) => result,
                Err(error) => Err(format!(
                    "{}: {error}",
                    text(
                        language,
                        "数据库列表后台任务意外结束",
                        "The database-list task ended unexpectedly",
                    )
                )),
            };
            panel
                .update(cx, |panel, cx| {
                    if panel.state.finish_database_load(&request, result) {
                        panel.reconcile_query_target(cx);
                        cx.notify();
                    }
                })
                .ok();
        })
        .detach();
    }

    fn retry_databases(
        &mut self,
        connection_id: String,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.clear_database_state(&connection_id);
        self.load_databases(connection_id, cx);
    }

    fn load_objects(&mut self, target: QueryTarget, cx: &mut Context<Self>) {
        let Some(requests) = self.state.begin_object_load(&target) else {
            return;
        };
        cx.notify();

        let language = self.settings.read(cx).language();
        for request in requests {
            let application = self.application.clone();
            let kind = request.kind();
            let connection_id = request.connection_id().to_string();
            let database = request.database().to_string();
            let load = gpui_tokio::Tokio::spawn(cx, async move {
                application
                    .catalog()
                    .catalog_section(&connection_id, &database, kind)
                    .await
            });
            cx.spawn(async move |panel, cx| {
                let result = match load.await {
                    Ok(result) => result,
                    Err(error) => crate::application::CatalogLoadResult::failed(
                        kind,
                        format!(
                            "{}: {error}",
                            text(
                                language,
                                "数据库对象后台任务意外结束",
                                "The database-object task ended unexpectedly",
                            )
                        ),
                    ),
                };
                panel
                    .update(cx, |panel, cx| {
                        if panel.state.finish_object_load(&request, result) {
                            cx.notify();
                        }
                    })
                    .ok();
            })
            .detach();
        }
    }

    fn retry_objects(
        &mut self,
        target: QueryTarget,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.clear_object_state(&target);
        self.load_objects(target, cx);
    }

    fn confirm_delete_selected(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.actions_blocked() {
            return;
        }
        let Some(selected) = self.selected_profile() else {
            return;
        };
        if selected
            .mcp_usage
            .as_ref()
            .is_some_and(|usage| usage.mcp_in_use)
            || self.state.operation(&selected.profile.id).is_some()
        {
            return;
        }
        let connection_id = selected.profile.id.clone();
        let expected_revision = selected.profile.revision;
        let language = self.settings.read(cx).language();
        let message = format!(
            "{} “{}”?",
            text(language, "删除连接配置", "Delete connection profile"),
            selected.profile.name
        );
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some(text(
                language,
                "保存的凭据也会被删除。此操作无法撤销。",
                "Stored credentials will also be deleted. This cannot be undone.",
            )),
            &[
                PromptButton::ok(text(language, "删除", "Delete")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn(async move |panel, cx| {
            if answer.await.ok() != Some(0) {
                return;
            }
            panel
                .update(cx, |panel, cx| {
                    panel.delete_profile(connection_id, expected_revision, cx);
                })
                .ok();
        })
        .detach();
    }

    fn delete_profile(
        &mut self,
        connection_id: String,
        expected_revision: i64,
        cx: &mut Context<Self>,
    ) {
        if self.actions_blocked() {
            return;
        }
        let Some(request) = self
            .state
            .begin_operation(&connection_id, ProfileOperationKind::Deleting)
        else {
            return;
        };
        self.invalidate_query_session(&connection_id, cx);
        self.notice = None;
        cx.notify();
        self.run_profile_operation(
            request,
            ProfileOperationCommand::Delete {
                connection_id,
                expected_revision,
            },
            cx,
        );
    }
}

fn operation_snapshot_failure_message(
    outcome: &ProfileOperationOutcome,
    language: crate::platform::UiLanguage,
) -> &'static str {
    match outcome {
        ProfileOperationOutcome::Connected(_) => text(
            language,
            "连接结果已返回，但无法刷新连接状态；请刷新后再继续。",
            "The connect result arrived, but the connection state could not be refreshed. Refresh before continuing.",
        ),
        ProfileOperationOutcome::Disconnected(_) => text(
            language,
            "断开结果已返回，但无法刷新连接状态；请刷新后再继续。",
            "The disconnect result arrived, but the connection state could not be refreshed. Refresh before continuing.",
        ),
        ProfileOperationOutcome::Deleted(Ok(_)) => text(
            language,
            "连接配置已删除，但无法刷新列表；当前列表已锁定，请刷新后再继续。",
            "The profile was deleted, but the list could not be refreshed. Refresh before continuing.",
        ),
        ProfileOperationOutcome::Deleted(Err(_)) => text(
            language,
            "删除失败，且无法刷新连接列表；请刷新后再继续。",
            "Deletion failed and the connection list could not be refreshed. Refresh before continuing.",
        ),
    }
}

fn derive_status(
    state: &ConnectionWorkspaceState,
    selected_profile_id: Option<&str>,
    selected_target: Option<&QueryTarget>,
    object_operation_in_progress: bool,
    language: crate::platform::UiLanguage,
) -> ConnectionProfilesStatus {
    let selected = selected_profile_id.and_then(|selected| {
        state
            .snapshot()?
            .profiles
            .iter()
            .find(|profile| profile.profile.id == selected)
    });
    let operation = selected.and_then(|profile| state.operation(&profile.profile.id));
    let summary = selected
        .map(|profile| {
            format!(
                "{} · {}",
                profile.profile.name,
                engine_label(profile.profile.db_type),
            )
        })
        .or_else(|| {
            state.snapshot().map(|snapshot| {
                format!(
                    "{} {}",
                    snapshot.profiles.len(),
                    super::localization::text(language, "个连接配置", "connection profiles")
                )
            })
        })
        .unwrap_or_else(|| {
            super::localization::text(language, "未加载连接配置", "Connection profiles not loaded")
                .to_string()
        });
    let session = match (selected, operation) {
        (_, Some(ProfileOperationKind::Connecting)) => ConnectionSessionStatus::Connecting,
        (_, Some(ProfileOperationKind::Disconnecting)) => ConnectionSessionStatus::Disconnecting,
        (_, Some(ProfileOperationKind::Deleting)) => ConnectionSessionStatus::Deleting,
        (Some(profile), None) if profile.session.is_connected() => {
            ConnectionSessionStatus::Connected
        }
        (Some(_), None) => ConnectionSessionStatus::Disconnected,
        (None, _) if state.snapshot().is_some() => ConnectionSessionStatus::NoSelection,
        (None, _) => ConnectionSessionStatus::Loading,
    };
    let activity = if state.error().is_some() {
        ConnectionActivityStatus::NeedsRefresh
    } else if state.is_refreshing() {
        ConnectionActivityStatus::Refreshing
    } else if operation.is_some() || object_operation_in_progress {
        ConnectionActivityStatus::Working
    } else if selected.is_some_and(|profile| {
        matches!(
            state.databases(&profile.profile.id),
            Some(crate::application::connection_workspace::DatabaseListState::Loading { .. })
        )
    }) {
        ConnectionActivityStatus::LoadingDatabases
    } else if selected_target.is_some_and(|target| {
        state
            .objects(target)
            .is_some_and(crate::application::connection_workspace::ObjectListState::is_loading)
    }) {
        ConnectionActivityStatus::LoadingObjects
    } else if state.snapshot().is_some() {
        ConnectionActivityStatus::Ready
    } else {
        ConnectionActivityStatus::Loading
    };

    ConnectionProfilesStatus {
        summary,
        session,
        activity,
    }
}

fn reconcile_selected_profile(
    selected_profile_id: &mut Option<String>,
    snapshot: Option<&crate::application::ConnectionWorkspaceSnapshot>,
) {
    let Some(selected) = selected_profile_id.as_ref() else {
        return;
    };
    let selection_exists = snapshot.is_some_and(|snapshot| {
        snapshot
            .profiles
            .iter()
            .any(|profile| &profile.profile.id == selected)
    });
    if !selection_exists {
        *selected_profile_id = None;
    }
}

#[cfg(test)]
mod tests;
