mod presentation;
mod view;

use std::sync::Arc;

use gpui::{App, ClickEvent, EventEmitter, PromptButton, PromptLevel};
use zed_ui::prelude::*;

use crate::application::connection_workspace::{
    ConnectionWorkspaceError, ConnectionWorkspaceState, OperationApply, ProfileOperationKind,
    ProfileOperationRequest, SnapshotApply,
};
use crate::application::{
    Application, ConnectionOutcome, ConnectionProfileSnapshot, ProfileOperationCommand,
    ProfileOperationCompletion, ProfileOperationOutcome,
};
use crate::connection_repository::SharedConnectionProfile;

use self::presentation::repository_error_message;
use super::engine_presentation::engine_label;

pub(super) const SIDEBAR_WIDTH: Pixels = px(272.0);

pub(super) fn bind_connection_profiles_keys(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("enter", menu::Confirm, Some("ConnectionProfileRow")),
        gpui::KeyBinding::new("space", menu::Confirm, Some("ConnectionProfileRow")),
    ]);
}

pub(super) struct ConnectionProfilesPanel {
    application: Arc<Application>,
    state: ConnectionWorkspaceState,
    selected_profile_id: Option<String>,
    notice: Option<PanelNotice>,
}

#[derive(Clone, Debug)]
pub(super) enum ConnectionProfilesEvent {
    CreateRequested,
    EditRequested(Arc<SharedConnectionProfile>),
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

impl ConnectionSessionStatus {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Loading => "连接状态加载中",
            Self::NoSelection => "未选择连接",
            Self::Disconnected => "未连接",
            Self::Connecting => "连接中",
            Self::Connected => "已连接",
            Self::Disconnecting => "断开中",
            Self::Deleting => "删除中",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConnectionActivityStatus {
    Loading,
    Refreshing,
    LoadingDatabases,
    Working,
    NeedsRefresh,
    Ready,
}

impl ConnectionActivityStatus {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Loading => "正在加载",
            Self::Refreshing => "正在刷新",
            Self::LoadingDatabases => "正在加载数据库",
            Self::Working => "正在处理",
            Self::NeedsRefresh => "需要刷新",
            Self::Ready => "就绪",
        }
    }
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
    pub(super) fn new(application: Arc<Application>, cx: &mut Context<Self>) -> Self {
        let mut panel = Self {
            application,
            state: ConnectionWorkspaceState::default(),
            selected_profile_id: None,
            notice: None,
        };
        panel.refresh_profiles(cx);
        panel
    }

    pub(super) fn profile_saved(&mut self, profile_id: String, cx: &mut Context<Self>) {
        self.selected_profile_id = Some(profile_id);
        self.refresh_profiles(cx);
    }

    pub(super) fn status(&self) -> ConnectionProfilesStatus {
        derive_status(&self.state, self.selected_profile_id.as_deref())
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
        self.state.is_refreshing() || self.state.error().is_some()
    }

    fn reconcile_selection(&mut self) {
        reconcile_selected_profile(&mut self.selected_profile_id, self.state.snapshot());
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
        let unexpected_message = match &command {
            ProfileOperationCommand::Connect { .. } => {
                "连接后台任务意外结束；请刷新后确认当前连接状态。"
            }
            ProfileOperationCommand::Disconnect { .. } => {
                "断开后台任务意外结束；请刷新后确认当前连接状态。"
            }
            ProfileOperationCommand::Delete { .. } => {
                "删除后台任务意外结束；请刷新后确认连接配置是否仍存在。"
            }
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
        match apply {
            OperationApply::Discarded => return,
            OperationApply::Snapshot(SnapshotApply::Applied) => {
                self.reconcile_selection();
                if self.apply_operation_outcome(completion.outcome) {
                    self.load_databases(connection_id.to_string(), cx);
                }
            }
            OperationApply::Snapshot(SnapshotApply::Failed) => {
                self.set_notice(
                    NoticeTone::Error,
                    operation_snapshot_failure_message(&completion.outcome),
                );
            }
            OperationApply::Snapshot(SnapshotApply::Superseded) => {
                self.apply_superseded_outcome(completion.outcome);
                self.refresh_profiles(cx);
            }
        }
        cx.notify();
    }

    fn apply_operation_outcome(&mut self, outcome: ProfileOperationOutcome) -> bool {
        match outcome {
            ProfileOperationOutcome::Connected(Ok(ConnectionOutcome::Succeeded)) => {
                self.set_notice(NoticeTone::Info, "连接成功");
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
                    "连接配置已删除；系统凭据将在稍后完成清理"
                } else {
                    "连接配置已删除"
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

    fn apply_superseded_outcome(&mut self, outcome: ProfileOperationOutcome) {
        match outcome {
            ProfileOperationOutcome::Connected(_) => {
                self.set_notice(NoticeTone::Warning, "连接结果已返回，正在重新同步最新状态…")
            }
            ProfileOperationOutcome::Disconnected(_) => {
                self.set_notice(NoticeTone::Warning, "断开结果已返回，正在重新同步最新状态…")
            }
            ProfileOperationOutcome::Deleted(Ok(_)) => {
                self.set_notice(NoticeTone::Warning, "连接配置已删除，正在重新同步最新列表…")
            }
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
                Err(error) => Err(format!("数据库列表后台任务意外结束：{error}")),
            };
            panel
                .update(cx, |panel, cx| {
                    if panel.state.finish_database_load(&request, result) {
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
        let message = format!("删除连接配置“{}”？", selected.profile.name);
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some("保存的凭据也会被删除。此操作无法撤销。"),
            &[PromptButton::ok("删除"), PromptButton::cancel("取消")],
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

fn operation_snapshot_failure_message(outcome: &ProfileOperationOutcome) -> &'static str {
    match outcome {
        ProfileOperationOutcome::Connected(_) => {
            "连接结果已返回，但无法刷新连接状态；请刷新后再继续。"
        }
        ProfileOperationOutcome::Disconnected(_) => {
            "断开结果已返回，但无法刷新连接状态；请刷新后再继续。"
        }
        ProfileOperationOutcome::Deleted(Ok(_)) => {
            "连接配置已删除，但无法刷新列表；当前列表已锁定，请刷新后再继续。"
        }
        ProfileOperationOutcome::Deleted(Err(_)) => {
            "删除失败，且无法刷新连接列表；请刷新后再继续。"
        }
    }
}

fn derive_status(
    state: &ConnectionWorkspaceState,
    selected_profile_id: Option<&str>,
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
            state
                .snapshot()
                .map(|snapshot| format!("{} 个连接配置", snapshot.profiles.len()))
        })
        .unwrap_or_else(|| "未加载连接配置".to_string());
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
    } else if operation.is_some() {
        ConnectionActivityStatus::Working
    } else if selected.is_some_and(|profile| {
        matches!(
            state.databases(&profile.profile.id),
            Some(crate::application::connection_workspace::DatabaseListState::Loading { .. })
        )
    }) {
        ConnectionActivityStatus::LoadingDatabases
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
    if selected_profile_id.as_ref().is_some_and(|selected| {
        snapshot.is_none_or(|snapshot| {
            !snapshot
                .profiles
                .iter()
                .any(|profile| &profile.profile.id == selected)
        })
    }) {
        *selected_profile_id = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{
        ConnectionProfileSnapshot, ConnectionWorkspaceSnapshot, DatabaseSessionSnapshot,
    };
    use crate::db::DbType;

    fn profile(id: &str) -> SharedConnectionProfile {
        SharedConnectionProfile {
            id: id.to_string(),
            name: id.to_string(),
            db_type: DbType::PostgreSQL,
            host: "127.0.0.1".to_string(),
            port: 5432,
            username: "tester".to_string(),
            database: None,
            color: None,
            group_name: None,
            tags: Vec::new(),
            has_credential: false,
            revision: 1,
            mcp_enabled: false,
        }
    }

    fn snapshot(profile: SharedConnectionProfile) -> ConnectionWorkspaceSnapshot {
        ConnectionWorkspaceSnapshot {
            repository_revision: 1,
            mcp_revision: 0,
            profiles: vec![ConnectionProfileSnapshot {
                profile,
                session: DatabaseSessionSnapshot {
                    generation: Some(7),
                },
                mcp_usage: None,
            }],
        }
    }

    #[test]
    fn replacing_profiles_clears_a_missing_selection() {
        let mut selected = Some("primary".to_string());
        let empty = ConnectionWorkspaceSnapshot {
            repository_revision: 2,
            mcp_revision: 0,
            profiles: Vec::new(),
        };

        reconcile_selected_profile(&mut selected, Some(&empty));

        assert!(selected.is_none());
    }

    #[test]
    fn structured_status_tracks_selected_session_and_operation() {
        let mut state = ConnectionWorkspaceState::default();
        let refresh = state.begin_refresh();
        state.finish_refresh(refresh, Ok(snapshot(profile("primary"))));

        let status = derive_status(&state, Some("primary"));
        assert_eq!(status.session, ConnectionSessionStatus::Connected);
        assert_eq!(status.activity, ConnectionActivityStatus::Ready);

        state.begin_operation("primary", ProfileOperationKind::Disconnecting);
        let status = derive_status(&state, Some("primary"));
        assert_eq!(status.session, ConnectionSessionStatus::Disconnecting);
        assert_eq!(status.activity, ConnectionActivityStatus::Working);
    }
}
