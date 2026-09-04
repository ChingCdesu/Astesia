mod catalog_loading;
mod catalog_primary;
mod catalog_view;
mod engine_workflows;
mod object_actions;
mod presentation;
mod profile_operations;
mod selection;
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
#[cfg(test)]
use self::selection::{derive_status, reconcile_selected_profile};
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
}
#[cfg(test)]
mod tests;
