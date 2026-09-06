mod catalog_loading;
mod catalog_primary;
mod catalog_tree;
mod catalog_view;
mod database_menu;
mod engine_workflows;
mod object_actions;
mod presentation;
mod profile_menu;
mod profile_operations;
mod selection;
mod view;
mod virtual_rows;

use std::{collections::HashSet, sync::Arc};

use crate::ui::components::prelude::*;
use crate::ui::input_field::InputField;
use gpui_kit::{
    App, ClickEvent, Entity, EventEmitter, PromptButton, PromptLevel, Subscription, Task,
};

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
use super::localization::text;
use super::object_definition_item::ObjectDefinition;
use super::object_mutation_form::ObjectMutationFormMode;
use super::shell::ShellSettings;

pub(super) const SIDEBAR_WIDTH: Pixels = px(272.0);

gpui_kit::actions!(astesia, [OpenProfileMenu]);

pub(super) fn bind_connection_profiles_keys(cx: &mut App) {
    cx.bind_keys([
        gpui_kit::KeyBinding::new("shift-f10", OpenProfileMenu, Some("ConnectionProfileRow")),
        gpui_kit::KeyBinding::new("enter", menu::Confirm, Some("ConnectionProfileRow")),
        gpui_kit::KeyBinding::new("space", menu::Confirm, Some("ConnectionProfileRow")),
        gpui_kit::KeyBinding::new("enter", menu::Confirm, Some("QueryTargetRow")),
        gpui_kit::KeyBinding::new("space", menu::Confirm, Some("QueryTargetRow")),
        gpui_kit::KeyBinding::new("enter", menu::Confirm, Some("SchemaObjectRow")),
        gpui_kit::KeyBinding::new("space", menu::Confirm, Some("SchemaObjectRow")),
    ]);
}

pub(super) struct ConnectionProfilesPanel {
    application: Arc<Application>,
    sidebar_list: gpui_kit::ListState,
    sidebar_rows_cache: std::cell::RefCell<Option<std::rc::Rc<Vec<virtual_rows::SidebarRow>>>>,
    #[cfg(test)]
    sidebar_rendered_rows: std::cell::RefCell<Vec<usize>>,
    #[cfg(test)]
    sidebar_model_builds: std::cell::Cell<usize>,
    sidebar_row_keys: std::cell::RefCell<Vec<String>>,
    state: ConnectionWorkspaceState,
    selected_profile_id: Option<String>,
    selected_profile_focus: gpui_kit::FocusHandle,
    selected_query_target: Option<QueryTarget>,
    notice: Option<PanelNotice>,
    failed_profiles: HashSet<String>,
    collapsed_groups: HashSet<Option<String>>,
    collapsed_schemas: HashSet<(String, u64, String, String)>,
    expanded_databases: HashSet<(String, u64, String)>,
    expanded_tables: HashSet<catalog_tree::CatalogTableKey>,
    collapsed_details: HashSet<(catalog_tree::CatalogTableKey, u8)>,
    table_details:
        std::collections::HashMap<catalog_tree::CatalogTableKey, catalog_tree::CatalogDetail>,
    detail_generation: u64,
    selected_catalog_table: Option<catalog_tree::CatalogTableKey>,
    context_menu: Option<(
        Entity<crate::ui::components::ContextMenu>,
        gpui_kit::Point<Pixels>,
        Subscription,
    )>,
    profile_menu_state: Option<profile_menu::ProfileMenuState>,
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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct ProfileActions {
    pub(super) connect: bool,
    pub(super) disconnect: bool,
    pub(super) edit: bool,
    pub(super) delete: bool,
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
    fn notify_sidebar(&self, cx: &mut Context<Self>) {
        // ListState also notifies on scroll, so row invalidation belongs to state changes.
        self.sidebar_rows_cache.borrow_mut().take();
        cx.notify();
    }

    pub(super) fn new(
        application: Arc<Application>,
        settings: Entity<ShellSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut panel = Self::new_unloaded(application, settings, window, cx);
        panel.refresh_profiles(cx);
        panel
    }

    fn new_unloaded(
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
        let settings_observation = cx.observe(&settings, |panel, _, cx| {
            panel.sidebar_list.remeasure();
            panel.notify_sidebar(cx);
        });
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
        Self {
            application,
            #[cfg(test)]
            sidebar_rendered_rows: std::cell::RefCell::new(Vec::new()),
            #[cfg(test)]
            sidebar_model_builds: std::cell::Cell::new(0),
            sidebar_rows_cache: std::cell::RefCell::new(None),
            sidebar_list: gpui_kit::ListState::new(0, gpui_kit::ListAlignment::Top, px(100.0)),
            sidebar_row_keys: std::cell::RefCell::new(Vec::new()),
            state: ConnectionWorkspaceState::default(),
            selected_profile_id: None,
            selected_profile_focus: cx.focus_handle().tab_stop(true),
            selected_query_target: None,
            notice: None,
            failed_profiles: HashSet::new(),
            collapsed_groups: HashSet::new(),
            collapsed_schemas: HashSet::new(),
            expanded_databases: HashSet::new(),
            expanded_tables: HashSet::new(),
            collapsed_details: HashSet::new(),
            table_details: std::collections::HashMap::new(),
            detail_generation: 0,
            selected_catalog_table: None,
            context_menu: None,
            profile_menu_state: None,
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
        }
    }
}
#[cfg(test)]
mod tests;
