use std::{
    collections::{HashMap, HashSet, VecDeque},
    env, fmt,
    sync::{Arc, Mutex as StdMutex, Weak},
    time::Duration,
};

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        HeaderMap, Request, Response, StatusCode,
    },
    routing::post,
    Router,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    net::TcpListener,
    sync::{oneshot, Mutex},
    task::JoinHandle,
};
use uuid::Uuid;

use crate::{
    db::{ConnectionConfig, DatabaseDriver, DbType},
    mcp_sync::{
        McpSyncContext, McpSyncProfile, McpSyncRequest, McpSyncResponse, PROTOCOL_VERSION,
        SYNC_PATH,
    },
    state::{create_driver, AppState},
};

pub const MCP_CONNECTIONS_CHANGED_EVENT: &str = "mcp-connections-changed";

const SOURCE: &str = "mcp_http";
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_REMEMBERED_OPERATIONS: usize = 4_096;
const DATABASE_CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
const DATABASE_DISCONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(not(test))]
const DRIVER_MAP_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const DRIVER_MAP_LOCK_TIMEOUT: Duration = Duration::from_millis(100);

type DriverMap = Arc<Mutex<HashMap<String, Box<dyn DatabaseDriver>>>>;
type DriverLeases = Arc<StdMutex<HashMap<String, Uuid>>>;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct McpConnectionSnapshot {
    pub id: String,
    pub name: String,
    pub db_type: DbType,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub source: &'static str,
    pub mcp_session_id: String,
    pub mcp_connection_id: String,
    pub mcp_transition: u64,
    pub mcp_connected: bool,
    pub app_connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct McpConnectionsSnapshot {
    pub revision: u64,
    pub connections: Vec<McpConnectionSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OwnershipKey {
    service_id: Uuid,
    session_id: Uuid,
    connection_id: String,
}

impl OwnershipKey {
    fn new(service_id: Uuid, session_id: Uuid, connection_id: String) -> Self {
        Self {
            service_id,
            session_id,
            connection_id,
        }
    }
}

#[derive(Clone)]
struct RegistryEntry {
    app_connection_id: String,
    profile: McpSyncProfile,
    mcp_connected: bool,
    app_connected: bool,
    last_error: Option<String>,
    transition: u64,
    transitioning: bool,
    driver_lease: Option<Uuid>,
}

#[derive(Debug, Clone)]
struct DriverTarget {
    app_connection_id: String,
    lease: Uuid,
}

struct ConnectTransitionGuard {
    registry: McpSyncRegistry,
    key: OwnershipKey,
    app_connection_id: String,
    transition: u64,
    armed: bool,
}

impl ConnectTransitionGuard {
    fn new(
        registry: &McpSyncRegistry,
        key: OwnershipKey,
        app_connection_id: String,
        transition: u64,
    ) -> Self {
        Self {
            registry: registry.clone(),
            key,
            app_connection_id,
            transition,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ConnectTransitionGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let registry = self.registry.clone();
        let key = self.key.clone();
        let app_connection_id = self.app_connection_id.clone();
        let transition = self.transition;
        runtime.spawn(async move {
            let snapshot = {
                let mut state = registry.inner.lock().await;
                let Some(entry) = state.entries.get_mut(&key) else {
                    return;
                };
                if entry.app_connection_id != app_connection_id
                    || entry.transition != transition
                    || !entry.transitioning
                {
                    return;
                }
                entry.transitioning = false;
                entry.mcp_connected = true;
                entry.app_connected = false;
                entry.driver_lease = None;
                entry.last_error =
                    Some("The mirrored App connection was interrupted; retry connect".into());
                state.revision = state.revision.saturating_add(1);
                snapshot_from_state(&state)
            };
            registry.emit(Some(snapshot)).await;
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct OperationKey {
    service_id: Uuid,
    session_id: Uuid,
    operation_id: Uuid,
}

#[derive(Default)]
struct RegistryState {
    revision: u64,
    entries: HashMap<OwnershipKey, RegistryEntry>,
    closed_sessions: HashSet<(Uuid, Uuid)>,
    closed_services: HashSet<Uuid>,
    completed_operations: HashMap<OperationKey, McpSyncResponse>,
    operation_order: VecDeque<OperationKey>,
}

#[derive(Clone)]
pub struct McpSyncRegistry {
    inner: Arc<Mutex<RegistryState>>,
    operation_locks: Arc<Mutex<HashMap<OperationKey, Weak<Mutex<()>>>>>,
    drivers: DriverMap,
    driver_leases: DriverLeases,
    app_handle: Option<AppHandle>,
}

impl McpSyncRegistry {
    pub fn new(app_handle: AppHandle) -> Self {
        let drivers = app_handle.state::<AppState>().connections.clone();
        Self {
            inner: Arc::new(Mutex::new(RegistryState::default())),
            operation_locks: Arc::new(Mutex::new(HashMap::new())),
            drivers,
            driver_leases: Arc::new(StdMutex::new(HashMap::new())),
            app_handle: Some(app_handle),
        }
    }

    #[cfg(test)]
    fn without_app_events() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RegistryState::default())),
            operation_locks: Arc::new(Mutex::new(HashMap::new())),
            drivers: Arc::new(Mutex::new(HashMap::new())),
            driver_leases: Arc::new(StdMutex::new(HashMap::new())),
            app_handle: None,
        }
    }

    pub async fn snapshot(&self) -> McpConnectionsSnapshot {
        let state = self.inner.lock().await;
        snapshot_from_state(&state)
    }

    async fn apply(&self, expected_service_id: Uuid, request: McpSyncRequest) -> McpSyncResponse {
        let context = request_context(&request);
        if let Err(error) = validate_context(context, expected_service_id) {
            return failure(error, None);
        }

        let operation_key = OperationKey {
            service_id: context.service_id,
            session_id: context.session_id,
            operation_id: context.operation_id,
        };
        let operation_lock = self.operation_lock(operation_key).await;
        let _operation_guard = operation_lock.lock().await;
        if let Some(response) = self.completed_response(operation_key).await {
            return response;
        }

        let response = match request {
            McpSyncRequest::Upsert { context, profile } => self.upsert(context, profile).await,
            McpSyncRequest::Connected {
                context,
                connection_id,
            } => self.connect(context, connection_id).await,
            McpSyncRequest::Disconnected {
                context,
                connection_id,
            } => self.disconnect(context, connection_id).await,
            McpSyncRequest::Deleted {
                context,
                connection_id,
            } => self.delete(context, connection_id).await,
            McpSyncRequest::SessionClosed { context } => self.close_session(context).await,
        };

        self.remember_response(operation_key, response.clone())
            .await;
        response
    }

    async fn operation_lock(&self, key: OperationKey) -> Arc<Mutex<()>> {
        let mut locks = self.operation_locks.lock().await;
        if locks.len() >= MAX_REMEMBERED_OPERATIONS {
            locks.retain(|_, lock| lock.strong_count() > 0);
        }
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    async fn completed_response(&self, key: OperationKey) -> Option<McpSyncResponse> {
        self.inner
            .lock()
            .await
            .completed_operations
            .get(&key)
            .cloned()
    }

    async fn remember_response(&self, key: OperationKey, response: McpSyncResponse) {
        let mut state = self.inner.lock().await;
        if state.completed_operations.contains_key(&key) {
            return;
        }
        state.completed_operations.insert(key, response);
        state.operation_order.push_back(key);
        while state.operation_order.len() > MAX_REMEMBERED_OPERATIONS {
            if let Some(expired) = state.operation_order.pop_front() {
                state.completed_operations.remove(&expired);
            }
        }
    }

    async fn upsert(&self, context: McpSyncContext, profile: McpSyncProfile) -> McpSyncResponse {
        if let Err(error) = profile.validate() {
            return failure(error.to_string(), None);
        }
        let key = OwnershipKey::new(
            context.service_id,
            context.session_id,
            profile.connection_id.clone(),
        );

        let (response, snapshot) = {
            let mut state = self.inner.lock().await;
            if let Some(error) = ensure_owner_is_open(&state, &key) {
                return failure(error, None);
            }

            if let Some(entry) = state.entries.get_mut(&key) {
                let app_connection_id = entry.app_connection_id.clone();
                if entry.profile == profile {
                    return success(Some(app_connection_id));
                }
                if entry.app_connected || entry.transitioning {
                    return failure(
                        "Disconnect the mirrored App connection before changing its profile",
                        Some(app_connection_id),
                    );
                }
                entry.profile = profile;
                entry.last_error = None;
                entry.transition = entry.transition.saturating_add(1);
                state.revision = state.revision.saturating_add(1);
                (
                    success(Some(app_connection_id)),
                    Some(snapshot_from_state(&state)),
                )
            } else {
                let app_connection_id = Uuid::new_v4().to_string();
                state.entries.insert(
                    key,
                    RegistryEntry {
                        app_connection_id: app_connection_id.clone(),
                        profile,
                        mcp_connected: false,
                        app_connected: false,
                        last_error: None,
                        transition: 0,
                        transitioning: false,
                        driver_lease: None,
                    },
                );
                state.revision = state.revision.saturating_add(1);
                (
                    success(Some(app_connection_id)),
                    Some(snapshot_from_state(&state)),
                )
            }
        };
        self.emit(snapshot).await;
        response
    }

    async fn connect(&self, context: McpSyncContext, connection_id: String) -> McpSyncResponse {
        if let Err(error) = validate_connection_id(&connection_id) {
            return failure(error, None);
        }
        let key = OwnershipKey::new(context.service_id, context.session_id, connection_id);

        enum Preparation {
            Complete(McpSyncResponse, Option<McpConnectionsSnapshot>),
            Connect {
                profile: McpSyncProfile,
                app_connection_id: String,
                transition: u64,
            },
        }

        let preparation = {
            let mut state = self.inner.lock().await;
            if let Some(error) = ensure_owner_is_open(&state, &key) {
                return failure(error, None);
            }
            let Some(entry) = state.entries.get_mut(&key) else {
                return failure("MCP connection profile has not been synchronized", None);
            };
            if entry.transitioning {
                return failure(
                    "A mirrored App connection transition is already in progress",
                    Some(entry.app_connection_id.clone()),
                );
            }

            if entry.app_connected {
                let app_connection_id = entry.app_connection_id.clone();
                if !entry.mcp_connected || entry.last_error.is_some() {
                    entry.mcp_connected = true;
                    entry.last_error = None;
                    state.revision = state.revision.saturating_add(1);
                    let snapshot = snapshot_from_state(&state);
                    Preparation::Complete(success(Some(app_connection_id)), Some(snapshot))
                } else {
                    Preparation::Complete(success(Some(app_connection_id)), None)
                }
            } else {
                entry.transition = entry.transition.saturating_add(1);
                entry.transitioning = true;
                Preparation::Connect {
                    profile: entry.profile.clone(),
                    app_connection_id: entry.app_connection_id.clone(),
                    transition: entry.transition,
                }
            }
        };

        let (profile, app_connection_id, transition) = match preparation {
            Preparation::Complete(response, snapshot) => {
                self.emit(snapshot).await;
                return response;
            }
            Preparation::Connect {
                profile,
                app_connection_id,
                transition,
            } => (profile, app_connection_id, transition),
        };
        let mut transition_guard =
            ConnectTransitionGuard::new(self, key.clone(), app_connection_id.clone(), transition);

        let resolved = resolve_profile(&profile, &app_connection_id);
        let mut driver = match resolved {
            Ok(config) => create_driver(&config),
            Err(message) => {
                let response = self
                    .finish_connect_failure(&key, transition, message, &app_connection_id)
                    .await;
                transition_guard.disarm();
                return response;
            }
        };
        if !matches!(
            tokio::time::timeout(DATABASE_CONNECT_TIMEOUT, driver.connect()).await,
            Ok(Ok(()))
        ) {
            let response = self
                .finish_connect_failure(
                    &key,
                    transition,
                    "Astesia App could not establish the mirrored database connection",
                    &app_connection_id,
                )
                .await;
            transition_guard.disarm();
            return response;
        }

        let mut pending_driver = Some(driver);
        let reservation_error = {
            let state = self.inner.lock().await;
            match state.entries.get(&key) {
                None => Some(failure(
                    "MCP connection was removed while the App was connecting",
                    None,
                )),
                Some(entry)
                    if entry.transition != transition
                        || !entry.transitioning
                        || entry.app_connection_id != app_connection_id =>
                {
                    Some(failure(
                        "MCP connection changed while the App was connecting",
                        Some(entry.app_connection_id.clone()),
                    ))
                }
                Some(_) => None,
            }
        };
        if let Some(response) = reservation_error {
            if let Some(driver) = pending_driver {
                detach_driver(driver);
            }
            transition_guard.disarm();
            return response;
        }

        let new_lease = Uuid::new_v4();
        let install_result = tokio::time::timeout(DRIVER_MAP_LOCK_TIMEOUT, async {
            let mut drivers = self.drivers.lock().await;
            let mut state = self.inner.lock().await;
            let Some(entry) = state.entries.get_mut(&key) else {
                return (
                    failure(
                        "MCP connection was removed while the App was connecting",
                        None,
                    ),
                    None,
                    None,
                );
            };
            if entry.transition != transition
                || !entry.transitioning
                || entry.app_connection_id != app_connection_id
            {
                return (
                    failure(
                        "MCP connection changed while the App was connecting",
                        Some(entry.app_connection_id.clone()),
                    ),
                    None,
                    None,
                );
            }

            let mut leases = lock_driver_leases(&self.driver_leases);
            if drivers.contains_key(&app_connection_id) && !leases.contains_key(&app_connection_id)
            {
                entry.transitioning = false;
                entry.mcp_connected = true;
                entry.app_connected = false;
                entry.driver_lease = None;
                entry.last_error =
                    Some("The generated App connection identifier is already in use".into());
                state.revision = state.revision.saturating_add(1);
                return (
                    failure(
                        "The generated App connection identifier is already in use",
                        Some(app_connection_id.clone()),
                    ),
                    Some(snapshot_from_state(&state)),
                    None,
                );
            }

            let replaced_driver = drivers.insert(
                app_connection_id.clone(),
                pending_driver.take().expect("connected driver"),
            );
            leases.insert(app_connection_id.clone(), new_lease);
            entry.transitioning = false;
            entry.mcp_connected = true;
            entry.app_connected = true;
            entry.driver_lease = Some(new_lease);
            entry.last_error = None;
            state.revision = state.revision.saturating_add(1);
            (
                success(Some(app_connection_id.clone())),
                Some(snapshot_from_state(&state)),
                replaced_driver,
            )
        })
        .await;
        let (response, snapshot, replaced_driver) = match install_result {
            Ok(result) => result,
            Err(_) => {
                let response = self
                    .finish_connect_failure(
                        &key,
                        transition,
                        "Astesia App is busy; the mirrored connection was not installed",
                        &app_connection_id,
                    )
                    .await;
                if let Some(driver) = pending_driver {
                    detach_driver(driver);
                }
                transition_guard.disarm();
                return response;
            }
        };
        if let Some(driver) = replaced_driver {
            detach_driver(driver);
        }
        if let Some(driver) = pending_driver {
            detach_driver(driver);
        }
        transition_guard.disarm();
        self.emit(snapshot).await;
        response
    }

    async fn finish_connect_failure(
        &self,
        key: &OwnershipKey,
        transition: u64,
        message: impl Into<String>,
        app_connection_id: &str,
    ) -> McpSyncResponse {
        let message = message.into();
        let snapshot = {
            let mut state = self.inner.lock().await;
            let Some(entry) = state.entries.get_mut(key) else {
                return failure(
                    "MCP connection was removed while the App was connecting",
                    None,
                );
            };
            if entry.app_connection_id != app_connection_id
                || entry.transition != transition
                || !entry.transitioning
            {
                return failure(
                    "MCP connection changed while the App was connecting",
                    Some(entry.app_connection_id.clone()),
                );
            }
            entry.transitioning = false;
            entry.mcp_connected = true;
            entry.app_connected = false;
            entry.last_error = Some(message.clone());
            state.revision = state.revision.saturating_add(1);
            snapshot_from_state(&state)
        };
        self.emit(Some(snapshot)).await;
        failure(message, Some(app_connection_id.to_string()))
    }

    async fn disconnect(&self, context: McpSyncContext, connection_id: String) -> McpSyncResponse {
        if let Err(error) = validate_connection_id(&connection_id) {
            return failure(error, None);
        }
        let key = OwnershipKey::new(context.service_id, context.session_id, connection_id);

        let (app_connection_id, transition, driver_lease, snapshot) = {
            let mut state = self.inner.lock().await;
            if let Some(error) = ensure_owner_is_open(&state, &key) {
                return failure(error, None);
            }
            let Some(entry) = state.entries.get_mut(&key) else {
                return success(None);
            };
            if entry.transitioning {
                return failure(
                    "A mirrored App connection transition is already in progress",
                    Some(entry.app_connection_id.clone()),
                );
            }
            if !entry.mcp_connected && !entry.app_connected {
                return success(Some(entry.app_connection_id.clone()));
            }
            entry.transition = entry.transition.saturating_add(1);
            entry.transitioning = false;
            entry.mcp_connected = false;
            entry.app_connected = false;
            entry.last_error = None;
            (
                entry.app_connection_id.clone(),
                entry.transition,
                entry.driver_lease.take(),
                {
                    state.revision = state.revision.saturating_add(1);
                    snapshot_from_state(&state)
                },
            )
        };
        self.emit(Some(snapshot)).await;

        let cleanup_deferred = match driver_lease {
            Some(lease) => {
                self.remove_drivers(vec![DriverTarget {
                    app_connection_id: app_connection_id.clone(),
                    lease,
                }])
                .await
            }
            None => false,
        };
        if cleanup_deferred {
            let deferred_snapshot = {
                let mut state = self.inner.lock().await;
                if let Some(entry) = state.entries.get_mut(&key) {
                    if entry.app_connection_id == app_connection_id
                        && entry.transition == transition
                        && !entry.app_connected
                    {
                        entry.last_error = Some(
                            "The mirrored App connection is being cleaned up in the background"
                                .into(),
                        );
                        state.revision = state.revision.saturating_add(1);
                        Some(snapshot_from_state(&state))
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            self.emit(deferred_snapshot).await;
        }
        success(Some(app_connection_id))
    }

    async fn delete(&self, context: McpSyncContext, connection_id: String) -> McpSyncResponse {
        if let Err(error) = validate_connection_id(&connection_id) {
            return failure(error, None);
        }
        let key = OwnershipKey::new(context.service_id, context.session_id, connection_id);
        let (removed, snapshot) = {
            let mut state = self.inner.lock().await;
            if let Some(error) = ensure_owner_is_open(&state, &key) {
                return failure(error, None);
            }
            let removed = state.entries.remove(&key);
            let snapshot = removed.as_ref().map(|_| {
                state.revision = state.revision.saturating_add(1);
                snapshot_from_state(&state)
            });
            (removed, snapshot)
        };

        self.emit(snapshot).await;
        if let Some(entry) = removed {
            self.disconnect_entries(vec![entry]).await;
        }
        success(None)
    }

    async fn close_session(&self, context: McpSyncContext) -> McpSyncResponse {
        let (entries, snapshot) = {
            let mut state = self.inner.lock().await;
            state
                .closed_sessions
                .insert((context.service_id, context.session_id));
            let keys = state
                .entries
                .keys()
                .filter(|key| {
                    key.service_id == context.service_id && key.session_id == context.session_id
                })
                .cloned()
                .collect::<Vec<_>>();
            let entries = keys
                .into_iter()
                .filter_map(|key| state.entries.remove(&key))
                .collect::<Vec<_>>();
            let snapshot = if entries.is_empty() {
                None
            } else {
                state.revision = state.revision.saturating_add(1);
                Some(snapshot_from_state(&state))
            };
            (entries, snapshot)
        };

        self.emit(snapshot).await;
        self.disconnect_entries(entries).await;
        success(None)
    }

    async fn reset_service(&self, service_id: Uuid) {
        let (entries, snapshot) = {
            let mut state = self.inner.lock().await;
            state.closed_services.insert(service_id);
            let keys = state
                .entries
                .keys()
                .filter(|key| key.service_id == service_id)
                .cloned()
                .collect::<Vec<_>>();
            let entries = keys
                .into_iter()
                .filter_map(|key| state.entries.remove(&key))
                .collect::<Vec<_>>();
            state
                .completed_operations
                .retain(|key, _| key.service_id != service_id);
            state
                .operation_order
                .retain(|key| key.service_id != service_id);
            state
                .closed_sessions
                .retain(|(owner_service, _)| *owner_service != service_id);
            let snapshot = if entries.is_empty() {
                None
            } else {
                state.revision = state.revision.saturating_add(1);
                Some(snapshot_from_state(&state))
            };
            (entries, snapshot)
        };

        self.emit(snapshot).await;
        self.disconnect_entries(entries).await;
    }

    async fn disconnect_entries(&self, entries: Vec<RegistryEntry>) {
        let targets = entries
            .into_iter()
            .filter_map(|entry| {
                entry.driver_lease.map(|lease| DriverTarget {
                    app_connection_id: entry.app_connection_id,
                    lease,
                })
            })
            .collect();
        let _ = self.remove_drivers(targets).await;
    }

    /// Remove only drivers whose lease still matches the requested generation.
    ///
    /// If App database work keeps the shared driver map busy, the cleanup is
    /// detached. A later reconnect installs a new lease, causing stale cleanup
    /// tasks to leave the replacement driver untouched.
    async fn remove_drivers(&self, targets: Vec<DriverTarget>) -> bool {
        if targets.is_empty() {
            return false;
        }
        let drivers = Arc::clone(&self.drivers);
        let leases = Arc::clone(&self.driver_leases);
        let (completion_sender, completion_receiver) = oneshot::channel();
        tokio::spawn(async move {
            let removed = take_matching_drivers(&drivers, &leases, &targets).await;
            for driver in removed {
                detach_driver(driver);
            }
            let _ = completion_sender.send(());
        });

        !matches!(
            tokio::time::timeout(DRIVER_MAP_LOCK_TIMEOUT, completion_receiver).await,
            Ok(Ok(()))
        )
    }

    async fn emit(&self, snapshot: Option<McpConnectionsSnapshot>) {
        let Some(snapshot) = snapshot else {
            return;
        };
        let Some(app_handle) = self.app_handle.as_ref() else {
            return;
        };
        let Some(window) = app_handle.get_webview_window("main") else {
            log::debug!("Unable to emit MCP connection snapshot: main window is unavailable");
            return;
        };
        if let Err(error) = window.emit(MCP_CONNECTIONS_CHANGED_EVENT, snapshot) {
            log::warn!("Unable to emit MCP connection snapshot: {error}");
        }
    }
}

fn lock_driver_leases(leases: &DriverLeases) -> std::sync::MutexGuard<'_, HashMap<String, Uuid>> {
    leases
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

async fn take_matching_drivers(
    drivers: &DriverMap,
    leases: &DriverLeases,
    targets: &[DriverTarget],
) -> Vec<Box<dyn DatabaseDriver>> {
    let mut driver_map = drivers.lock().await;
    let mut lease_map = lock_driver_leases(leases);
    let mut removed = Vec::with_capacity(targets.len());
    for target in targets {
        if lease_map.get(&target.app_connection_id) != Some(&target.lease) {
            continue;
        }
        lease_map.remove(&target.app_connection_id);
        if let Some(driver) = driver_map.remove(&target.app_connection_id) {
            removed.push(driver);
        }
    }
    removed
}

fn detach_driver(mut driver: Box<dyn DatabaseDriver>) {
    tokio::spawn(async move {
        let _ = tokio::time::timeout(DATABASE_DISCONNECT_TIMEOUT, driver.disconnect()).await;
    });
}

fn snapshot_from_state(state: &RegistryState) -> McpConnectionsSnapshot {
    let mut connections = state
        .entries
        .iter()
        .map(|(key, entry)| McpConnectionSnapshot {
            id: entry.app_connection_id.clone(),
            name: entry.profile.name.clone(),
            db_type: entry.profile.db_type.clone(),
            host: entry.profile.host.clone(),
            port: entry.profile.port,
            username: entry.profile.username.clone(),
            database: entry.profile.database.clone(),
            color: entry.profile.color.clone(),
            source: SOURCE,
            mcp_session_id: key.session_id.to_string(),
            mcp_connection_id: key.connection_id.clone(),
            mcp_transition: entry.transition,
            mcp_connected: entry.mcp_connected,
            app_connected: entry.app_connected,
            last_error: entry.last_error.clone(),
        })
        .collect::<Vec<_>>();
    connections.sort_by(|left, right| left.id.cmp(&right.id));
    McpConnectionsSnapshot {
        revision: state.revision,
        connections,
    }
}

fn ensure_owner_is_open(state: &RegistryState, key: &OwnershipKey) -> Option<String> {
    if state.closed_services.contains(&key.service_id) {
        Some("MCP synchronization service is closed".into())
    } else if state
        .closed_sessions
        .contains(&(key.service_id, key.session_id))
    {
        Some("MCP session is closed".into())
    } else {
        None
    }
}

fn resolve_profile(
    profile: &McpSyncProfile,
    app_connection_id: &str,
) -> Result<ConnectionConfig, String> {
    let password = match profile.password_env.as_deref() {
        Some(variable) => env::var(variable)
            .map_err(|_| "The mirrored database credential is not available to Astesia App")?,
        None => String::new(),
    };
    Ok(ConnectionConfig {
        id: app_connection_id.to_string(),
        name: profile.name.clone(),
        db_type: profile.db_type.clone(),
        host: profile.host.clone(),
        port: profile.port,
        username: profile.username.clone(),
        password,
        database: profile.database.clone(),
        color: profile.color.clone(),
    })
}

fn validate_connection_id(connection_id: &str) -> Result<(), String> {
    if connection_id.trim().is_empty() {
        return Err("MCP connection identifier must not be empty".into());
    }
    if connection_id.len() > 256 {
        return Err("MCP connection identifier must not exceed 256 bytes".into());
    }
    Ok(())
}

fn validate_context(context: &McpSyncContext, expected_service_id: Uuid) -> Result<(), String> {
    if context.protocol_version != PROTOCOL_VERSION {
        return Err("Unsupported MCP synchronization protocol version".into());
    }
    if context.service_id != expected_service_id {
        return Err("MCP synchronization service identifier does not match".into());
    }
    if context.session_id.is_nil() || context.operation_id.is_nil() {
        return Err("MCP synchronization identifiers must not be nil UUIDs".into());
    }
    Ok(())
}

fn request_context(request: &McpSyncRequest) -> &McpSyncContext {
    match request {
        McpSyncRequest::Upsert { context, .. }
        | McpSyncRequest::Connected { context, .. }
        | McpSyncRequest::Disconnected { context, .. }
        | McpSyncRequest::Deleted { context, .. }
        | McpSyncRequest::SessionClosed { context } => context,
    }
}

fn success(app_connection_id: Option<String>) -> McpSyncResponse {
    McpSyncResponse {
        ok: true,
        error: None,
        app_connection_id,
    }
}

fn failure(message: impl Into<String>, app_connection_id: Option<String>) -> McpSyncResponse {
    McpSyncResponse {
        ok: false,
        error: Some(message.into()),
        app_connection_id,
    }
}

#[derive(Clone)]
struct ServerContext {
    service_id: Uuid,
    bearer: Arc<str>,
    registry: McpSyncRegistry,
}

pub struct McpSyncServerHandle {
    endpoint: String,
    token: String,
    service_id: Uuid,
    registry: McpSyncRegistry,
    shutdown_sender: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), String>>>,
}

impl fmt::Debug for McpSyncServerHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpSyncServerHandle")
            .field("endpoint", &self.endpoint)
            .field("token", &"[redacted]")
            .field("service_id", &self.service_id)
            .finish_non_exhaustive()
    }
}

impl McpSyncServerHandle {
    pub async fn start(registry: McpSyncRegistry) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| format!("Unable to bind MCP synchronization server: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("Unable to inspect MCP synchronization server: {error}"))?;
        let (service_id, token) = generate_credentials();
        let context = ServerContext {
            service_id,
            bearer: Arc::from(format!("Bearer {token}")),
            registry: registry.clone(),
        };
        let router = Router::new()
            .route(SYNC_PATH, post(receive_sync))
            .with_state(context);
        let (shutdown_sender, shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_receiver.await;
                })
                .await
                .map_err(|error| format!("MCP synchronization server stopped: {error}"))
        });

        Ok(Self {
            endpoint: format!("http://127.0.0.1:{}{SYNC_PATH}", address.port()),
            token,
            service_id,
            registry,
            shutdown_sender: Some(shutdown_sender),
            task: Some(task),
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn service_id(&self) -> Uuid {
        self.service_id
    }

    pub async fn shutdown(mut self) -> Result<(), String> {
        if let Some(sender) = self.shutdown_sender.take() {
            let _ = sender.send(());
        }
        let server_result = match self.task.take() {
            Some(mut task) => {
                match tokio::time::timeout(SERVER_SHUTDOWN_TIMEOUT, &mut task).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(error)) => Err(format!("MCP synchronization task failed: {error}")),
                    Err(_) => {
                        task.abort();
                        let _ = task.await;
                        Err("MCP synchronization server did not stop within 2 seconds".into())
                    }
                }
            }
            None => Ok(()),
        };
        self.registry.reset_service(self.service_id).await;
        server_result
    }
}

impl Drop for McpSyncServerHandle {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown_sender.take() {
            let _ = sender.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let registry = self.registry.clone();
        let service_id = self.service_id;
        runtime.spawn(async move {
            registry.reset_service(service_id).await;
        });
    }
}

fn generate_credentials() -> (Uuid, String) {
    (
        Uuid::new_v4(),
        format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
    )
}

async fn receive_sync(
    State(context): State<ServerContext>,
    request: Request<Body>,
) -> Response<Body> {
    if !authorizes(request.headers(), context.bearer.as_bytes()) {
        return response(StatusCode::UNAUTHORIZED, failure("Unauthorized", None));
    }
    let body = match to_bytes(request.into_body(), MAX_REQUEST_BYTES).await {
        Ok(body) => body,
        Err(_) => {
            return response(
                StatusCode::BAD_REQUEST,
                failure("Invalid MCP synchronization request body", None),
            )
        }
    };
    let request = match serde_json::from_slice::<McpSyncRequest>(&body) {
        Ok(request) => request,
        Err(_) => {
            return response(
                StatusCode::BAD_REQUEST,
                failure("Invalid MCP synchronization request", None),
            )
        }
    };
    let result = context.registry.apply(context.service_id, request).await;
    response(StatusCode::OK, result)
}

fn authorizes(headers: &HeaderMap, expected: &[u8]) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|actual| constant_time_eq(actual.as_bytes(), expected))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn response(status: StatusCode, value: McpSyncResponse) -> Response<Body> {
    let body = serde_json::to_vec(&value)
        .unwrap_or_else(|_| br#"{"ok":false,"error":"Unable to serialize response"}"#.to_vec());
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("valid synchronization response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn context(service_id: Uuid, session_id: Uuid) -> McpSyncContext {
        McpSyncContext {
            protocol_version: PROTOCOL_VERSION,
            service_id,
            session_id,
            operation_id: Uuid::new_v4(),
        }
    }

    fn profile(connection_id: &str) -> McpSyncProfile {
        McpSyncProfile {
            connection_id: connection_id.into(),
            name: "Test SQLite".into(),
            db_type: DbType::SQLite,
            host: ":memory:".into(),
            port: 0,
            username: String::new(),
            database: None,
            color: None,
            password_env: Some("ASTESIA_DB_PASSWORD_TEST".into()),
        }
    }

    async fn upsert(
        registry: &McpSyncRegistry,
        service_id: Uuid,
        session_id: Uuid,
        profile: McpSyncProfile,
    ) -> McpSyncResponse {
        registry
            .apply(
                service_id,
                McpSyncRequest::Upsert {
                    context: context(service_id, session_id),
                    profile,
                },
            )
            .await
    }

    #[tokio::test]
    async fn snapshot_is_sanitized_and_upsert_is_idempotent() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();

        let first = upsert(&registry, service_id, session_id, profile("shared-id")).await;
        let second = upsert(&registry, service_id, session_id, profile("shared-id")).await;
        assert!(first.ok);
        assert!(second.ok);
        assert_eq!(first.app_connection_id, second.app_connection_id);

        let snapshot = registry.snapshot().await;
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.connections.len(), 1);
        let value = serde_json::to_value(&snapshot).expect("serialize snapshot");
        assert_eq!(
            value
                .as_object()
                .expect("snapshot object")
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            HashSet::from(["revision".to_string(), "connections".to_string()])
        );
        let connection = value["connections"][0]
            .as_object()
            .expect("connection object");
        let keys = connection.keys().cloned().collect::<HashSet<_>>();
        assert_eq!(
            keys,
            HashSet::from([
                "id".to_string(),
                "name".to_string(),
                "db_type".to_string(),
                "host".to_string(),
                "port".to_string(),
                "username".to_string(),
                "source".to_string(),
                "mcp_session_id".to_string(),
                "mcp_connection_id".to_string(),
                "mcp_transition".to_string(),
                "mcp_connected".to_string(),
                "app_connected".to_string(),
            ])
        );
        assert_eq!(connection["source"], Value::String("mcp_http".into()));
        assert_eq!(connection["mcp_transition"], Value::Number(0_u64.into()));
    }

    #[tokio::test]
    async fn identical_mcp_ids_in_different_sessions_do_not_collide() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let session_a = Uuid::new_v4();
        let session_b = Uuid::new_v4();

        let first = upsert(&registry, service_id, session_a, profile("same")).await;
        let second = upsert(&registry, service_id, session_b, profile("same")).await;

        assert_ne!(first.app_connection_id, second.app_connection_id);
        assert_eq!(registry.snapshot().await.connections.len(), 2);
    }

    #[tokio::test]
    async fn closing_one_session_removes_only_its_entries() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let session_a = Uuid::new_v4();
        let session_b = Uuid::new_v4();
        upsert(&registry, service_id, session_a, profile("a")).await;
        upsert(&registry, service_id, session_b, profile("b")).await;

        let response = registry
            .apply(
                service_id,
                McpSyncRequest::SessionClosed {
                    context: context(service_id, session_a),
                },
            )
            .await;
        assert!(response.ok);
        let snapshot = registry.snapshot().await;
        assert_eq!(snapshot.connections.len(), 1);
        assert_eq!(snapshot.connections[0].mcp_connection_id, "b");

        let late = upsert(&registry, service_id, session_a, profile("late")).await;
        assert!(!late.ok);
    }

    #[tokio::test]
    async fn resetting_a_service_preserves_other_service_entries() {
        let registry = McpSyncRegistry::without_app_events();
        let service_a = Uuid::new_v4();
        let service_b = Uuid::new_v4();
        upsert(&registry, service_a, Uuid::new_v4(), profile("service-a")).await;
        upsert(&registry, service_b, Uuid::new_v4(), profile("service-b")).await;

        registry.reset_service(service_a).await;

        let snapshot = registry.snapshot().await;
        assert_eq!(snapshot.connections.len(), 1);
        assert_eq!(snapshot.connections[0].mcp_connection_id, "service-b");
        assert_eq!(snapshot.revision, 3);
    }

    #[tokio::test]
    async fn connected_sqlite_profile_installs_and_removes_a_real_app_driver() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut sqlite = profile("sqlite");
        sqlite.password_env = None;
        let created = upsert(&registry, service_id, session_id, sqlite).await;
        let app_id = created.app_connection_id.expect("app id");

        let connected = registry
            .apply(
                service_id,
                McpSyncRequest::Connected {
                    context: context(service_id, session_id),
                    connection_id: "sqlite".into(),
                },
            )
            .await;
        assert!(connected.ok, "{connected:?}");
        {
            let drivers = registry.drivers.lock().await;
            let driver = drivers.get(&app_id).expect("mirrored App driver");
            assert_eq!(
                driver.get_databases().await.expect("SQLite databases"),
                vec!["main"]
            );
        }
        let snapshot = registry.snapshot().await;
        assert!(snapshot.connections[0].mcp_connected);
        assert!(snapshot.connections[0].app_connected);

        let disconnected = registry
            .apply(
                service_id,
                McpSyncRequest::Disconnected {
                    context: context(service_id, session_id),
                    connection_id: "sqlite".into(),
                },
            )
            .await;
        assert!(disconnected.ok);
        assert!(!registry.drivers.lock().await.contains_key(&app_id));
    }

    #[tokio::test]
    async fn connect_does_not_hold_registry_while_waiting_for_app_driver_map() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut sqlite = profile("nonblocking");
        sqlite.password_env = None;
        upsert(&registry, service_id, session_id, sqlite).await;

        let driver_guard = registry.drivers.lock().await;
        let worker_registry = registry.clone();
        let worker = tokio::spawn(async move {
            worker_registry
                .apply(
                    service_id,
                    McpSyncRequest::Connected {
                        context: context(service_id, session_id),
                        connection_id: "nonblocking".into(),
                    },
                )
                .await
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if registry
                    .inner
                    .lock()
                    .await
                    .entries
                    .values()
                    .any(|entry| entry.transitioning)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("connect transition started");
        tokio::time::sleep(Duration::from_millis(50)).await;

        let registry_guard =
            tokio::time::timeout(Duration::from_millis(250), registry.inner.lock())
                .await
                .expect("registry remains available while App driver map is busy");
        drop(registry_guard);
        drop(driver_guard);

        assert!(worker.await.expect("connect task").ok);
    }

    #[tokio::test]
    async fn driver_map_timeout_clears_connect_transition_and_allows_retry() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut sqlite = profile("map-timeout");
        sqlite.password_env = None;
        let created = upsert(&registry, service_id, session_id, sqlite).await;
        let app_id = created.app_connection_id.expect("app id");

        let driver_guard = registry.drivers.lock().await;
        let started = tokio::time::Instant::now();
        let response = registry
            .apply(
                service_id,
                McpSyncRequest::Connected {
                    context: context(service_id, session_id),
                    connection_id: "map-timeout".into(),
                },
            )
            .await;
        assert!(!response.ok);
        assert!(started.elapsed() < Duration::from_secs(1));
        {
            let state = registry.inner.lock().await;
            let entry = state.entries.values().next().expect("registry entry");
            assert!(!entry.transitioning);
            assert!(entry.mcp_connected);
            assert!(!entry.app_connected);
            assert!(entry
                .last_error
                .as_deref()
                .is_some_and(|message| message.contains("busy")));
        }
        assert!(!driver_guard.contains_key(&app_id));
        drop(driver_guard);

        let retry = registry
            .apply(
                service_id,
                McpSyncRequest::Connected {
                    context: context(service_id, session_id),
                    connection_id: "map-timeout".into(),
                },
            )
            .await;
        assert!(retry.ok, "{retry:?}");
        assert!(registry.drivers.lock().await.contains_key(&app_id));
    }

    #[tokio::test]
    async fn deferred_disconnect_cleanup_cannot_remove_a_reconnected_driver() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut sqlite = profile("lease-race");
        sqlite.password_env = None;
        let created = upsert(&registry, service_id, session_id, sqlite).await;
        let app_id = created.app_connection_id.expect("app id");
        assert!(
            registry
                .apply(
                    service_id,
                    McpSyncRequest::Connected {
                        context: context(service_id, session_id),
                        connection_id: "lease-race".into(),
                    },
                )
                .await
                .ok
        );

        let driver_guard = registry.drivers.lock().await;
        let disconnected = registry
            .apply(
                service_id,
                McpSyncRequest::Disconnected {
                    context: context(service_id, session_id),
                    connection_id: "lease-race".into(),
                },
            )
            .await;
        assert!(disconnected.ok);

        let reconnect_registry = registry.clone();
        let reconnect = tokio::spawn(async move {
            reconnect_registry
                .apply(
                    service_id,
                    McpSyncRequest::Connected {
                        context: context(service_id, session_id),
                        connection_id: "lease-race".into(),
                    },
                )
                .await
        });
        tokio::task::yield_now().await;
        drop(driver_guard);

        let response = reconnect.await.expect("reconnect task");
        assert!(response.ok, "{response:?}");
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(registry.drivers.lock().await.contains_key(&app_id));
        let state = registry.inner.lock().await;
        let entry = state.entries.values().next().expect("registry entry");
        assert!(entry.app_connected);
        assert!(entry.driver_lease.is_some());
        assert_eq!(
            lock_driver_leases(&registry.driver_leases).get(&app_id),
            entry.driver_lease.as_ref()
        );
    }

    #[tokio::test]
    async fn driver_cleanup_survives_waiter_cancellation() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut sqlite = profile("cancelled-cleanup");
        sqlite.password_env = None;
        let created = upsert(&registry, service_id, session_id, sqlite).await;
        let app_id = created.app_connection_id.expect("app id");
        assert!(
            registry
                .apply(
                    service_id,
                    McpSyncRequest::Connected {
                        context: context(service_id, session_id),
                        connection_id: "cancelled-cleanup".into(),
                    },
                )
                .await
                .ok
        );
        let lease = registry
            .inner
            .lock()
            .await
            .entries
            .values()
            .next()
            .and_then(|entry| entry.driver_lease)
            .expect("driver lease");

        let driver_guard = registry.drivers.lock().await;
        let cleanup_registry = registry.clone();
        let cleanup_app_id = app_id.clone();
        let cleanup = tokio::spawn(async move {
            cleanup_registry
                .remove_drivers(vec![DriverTarget {
                    app_connection_id: cleanup_app_id,
                    lease,
                }])
                .await
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        cleanup.abort();
        let _ = cleanup.await;
        assert!(driver_guard.contains_key(&app_id));
        drop(driver_guard);

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !registry.drivers.lock().await.contains_key(&app_id) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cleanup worker retained ownership after waiter cancellation");
        assert!(!lock_driver_leases(&registry.driver_leases).contains_key(&app_id));
    }

    #[tokio::test]
    async fn cancelling_atomic_connect_before_registry_commit_cannot_install_a_driver() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut sqlite = profile("cancelled-connect");
        sqlite.password_env = None;
        let created = upsert(&registry, service_id, session_id, sqlite).await;
        let app_id = created.app_connection_id.expect("app id");

        let driver_guard = registry.drivers.lock().await;
        let connect_registry = registry.clone();
        let connect = tokio::spawn(async move {
            connect_registry
                .apply(
                    service_id,
                    McpSyncRequest::Connected {
                        context: context(service_id, session_id),
                        connection_id: "cancelled-connect".into(),
                    },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if registry
                    .inner
                    .lock()
                    .await
                    .entries
                    .values()
                    .any(|entry| entry.transitioning)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("connect transition started");
        tokio::time::sleep(Duration::from_millis(25)).await;

        let registry_guard = registry.inner.lock().await;
        drop(driver_guard);
        assert!(
            tokio::time::timeout(Duration::from_millis(25), registry.drivers.lock())
                .await
                .is_err(),
            "connect owns the driver map while waiting to commit registry state"
        );
        connect.abort();
        let _ = connect.await;
        drop(registry_guard);

        assert!(!registry.drivers.lock().await.contains_key(&app_id));
        assert!(!lock_driver_leases(&registry.driver_leases).contains_key(&app_id));
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let state = registry.inner.lock().await;
                let entry = state.entries.values().next().expect("registry entry");
                if !entry.transitioning {
                    assert!(entry.mcp_connected);
                    assert!(!entry.app_connected);
                    break;
                }
                drop(state);
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled transition cleanup");

        let retry = registry
            .apply(
                service_id,
                McpSyncRequest::Connected {
                    context: context(service_id, session_id),
                    connection_id: "cancelled-connect".into(),
                },
            )
            .await;
        assert!(retry.ok, "{retry:?}");
        assert!(registry.drivers.lock().await.contains_key(&app_id));
    }

    #[tokio::test]
    async fn stale_connect_cleanup_cannot_modify_a_recreated_entry() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let original = profile("aba");
        upsert(&registry, service_id, session_id, original.clone()).await;
        let key = OwnershipKey::new(service_id, session_id, "aba".into());

        let (old_app_id, transition) = {
            let mut state = registry.inner.lock().await;
            let entry = state.entries.get_mut(&key).expect("original entry");
            entry.transition = 1;
            entry.transitioning = true;
            (entry.app_connection_id.clone(), entry.transition)
        };
        let guard =
            ConnectTransitionGuard::new(&registry, key.clone(), old_app_id.clone(), transition);

        let new_app_id = Uuid::new_v4().to_string();
        {
            let mut state = registry.inner.lock().await;
            state.entries.insert(
                key.clone(),
                RegistryEntry {
                    app_connection_id: new_app_id.clone(),
                    profile: original,
                    mcp_connected: false,
                    app_connected: false,
                    last_error: None,
                    transition,
                    transitioning: true,
                    driver_lease: None,
                },
            );
        }

        let failure = registry
            .finish_connect_failure(&key, transition, "stale failure", old_app_id.as_str())
            .await;
        assert!(!failure.ok);
        drop(guard);
        tokio::time::sleep(Duration::from_millis(10)).await;

        let state = registry.inner.lock().await;
        let entry = state.entries.get(&key).expect("recreated entry");
        assert_eq!(entry.app_connection_id, new_app_id);
        assert!(entry.transitioning);
        assert!(entry.last_error.is_none());
    }

    #[tokio::test]
    async fn concurrent_duplicate_operations_share_the_first_result() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut sqlite = profile("duplicate-operation");
        sqlite.password_env = None;
        upsert(&registry, service_id, session_id, sqlite).await;

        let request = McpSyncRequest::Connected {
            context: McpSyncContext {
                protocol_version: PROTOCOL_VERSION,
                service_id,
                session_id,
                operation_id: Uuid::new_v4(),
            },
            connection_id: "duplicate-operation".into(),
        };
        let driver_guard = registry.drivers.lock().await;
        let first_registry = registry.clone();
        let first_request = request.clone();
        let first =
            tokio::spawn(async move { first_registry.apply(service_id, first_request).await });

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if registry
                    .inner
                    .lock()
                    .await
                    .entries
                    .values()
                    .any(|entry| entry.transitioning)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first duplicate operation started");

        let second_registry = registry.clone();
        let second_request = request.clone();
        let second =
            tokio::spawn(async move { second_registry.apply(service_id, second_request).await });
        tokio::task::yield_now().await;
        assert!(
            !second.is_finished(),
            "duplicate request must wait for the in-flight operation"
        );
        drop(driver_guard);

        let first_response = first.await.expect("first duplicate task");
        let second_response = second.await.expect("second duplicate task");
        assert!(first_response.ok, "{first_response:?}");
        assert_eq!(second_response, first_response);
        assert_eq!(
            registry.apply(service_id, request).await,
            first_response,
            "later retries must replay the completed response"
        );
    }

    #[tokio::test]
    async fn helper_shutdown_is_bounded_and_defers_busy_driver_cleanup() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut sqlite = profile("shutdown");
        sqlite.password_env = None;
        let created = upsert(&registry, service_id, session_id, sqlite).await;
        let app_id = created.app_connection_id.expect("app id");
        assert!(
            registry
                .apply(
                    service_id,
                    McpSyncRequest::Connected {
                        context: context(service_id, session_id),
                        connection_id: "shutdown".into(),
                    },
                )
                .await
                .ok
        );

        let driver_guard = registry.drivers.lock().await;
        let handle = McpSyncServerHandle {
            endpoint: "http://127.0.0.1:1/v1/sync".into(),
            token: "redacted".into(),
            service_id,
            registry: registry.clone(),
            shutdown_sender: None,
            task: Some(tokio::spawn(async { Ok(()) })),
        };
        let started = tokio::time::Instant::now();
        handle.shutdown().await.expect("bounded shutdown");
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(registry.snapshot().await.connections.is_empty());
        assert!(driver_guard.contains_key(&app_id));
        drop(driver_guard);

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !registry.drivers.lock().await.contains_key(&app_id) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("deferred shutdown cleanup");
        assert!(!lock_driver_leases(&registry.driver_leases).contains_key(&app_id));
    }

    #[tokio::test]
    async fn dropping_a_sync_server_handle_resets_its_service() {
        let registry = McpSyncRegistry::without_app_events();
        let service_id = Uuid::new_v4();
        let handle = McpSyncServerHandle {
            endpoint: "http://127.0.0.1:1/v1/sync".into(),
            token: "redacted".into(),
            service_id,
            registry: registry.clone(),
            shutdown_sender: None,
            task: Some(tokio::spawn(async {
                std::future::pending::<()>().await;
                Ok(())
            })),
        };
        let service_id = handle.service_id();
        upsert(
            &registry,
            service_id,
            Uuid::new_v4(),
            profile("dropped-handle"),
        )
        .await;
        assert_eq!(registry.snapshot().await.connections.len(), 1);

        drop(handle);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if registry.snapshot().await.connections.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("drop-triggered service reset");
    }

    #[test]
    fn server_requires_its_independent_bearer_token() {
        let (service_id, token) = generate_credentials();
        assert_ne!(service_id.simple().to_string(), token);
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));

        let expected = format!("Bearer {token}");
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer wrong".parse().unwrap());
        assert!(!authorizes(&headers, expected.as_bytes()));
        headers.insert(AUTHORIZATION, expected.parse().unwrap());
        assert!(authorizes(&headers, expected.as_bytes()));
    }
}
