use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    sync::{Arc, Weak},
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
    sync::{oneshot, Mutex, Notify, OwnedMutexGuard},
    task::JoinHandle,
};
use uuid::Uuid;

use crate::{
    connection_repository::SharedConnectionRepository,
    mcp_sync::{
        McpControlCommand, McpSyncContext, McpSyncRequest, McpSyncResponse, PROTOCOL_VERSION,
        SYNC_PATH,
    },
    state::AppState,
};

pub const MCP_CONNECTIONS_CHANGED_EVENT: &str = "mcp-connections-changed";

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_REMEMBERED_OPERATIONS: usize = 4_096;
const MAX_IDENTIFIER_BYTES: usize = 256;
const MAX_CONTROL_ERROR_BYTES: usize = 4_096;
const CONTROL_POLL_TIMEOUT: Duration = Duration::from_secs(55);
const FORCE_DISCONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const FORCE_DISCONNECT_TIMEOUT_MESSAGE: &str =
    "Timed out waiting for the Streamable HTTP MCP session to disconnect";

type SessionKey = (Uuid, Uuid);
type LifecycleLocks = Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>;
type ControlNotifies = Arc<Mutex<HashMap<SessionKey, Arc<Notify>>>>;
type ControlWaiters = Arc<Mutex<HashMap<Uuid, Vec<oneshot::Sender<Result<(), String>>>>>>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct McpConnectionSnapshot {
    /// Canonical identifier from the shared connection repository.
    pub id: String,
    pub profile_revision: i64,
    pub mcp_in_use: bool,
    pub mcp_connected: bool,
    pub mcp_session_count: usize,
    pub disconnecting: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct McpConnectionsSnapshot {
    pub revision: u64,
    pub connections: Vec<McpConnectionSnapshot>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ForceDisconnectResult {
    pub requested: usize,
    pub completed: usize,
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

    fn session_key(&self) -> SessionKey {
        (self.service_id, self.session_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpUsePhase {
    Acquired,
    Connected,
    Disconnecting { was_connected: bool },
}

impl McpUsePhase {
    fn is_connected(self) -> bool {
        matches!(
            self,
            Self::Connected
                | Self::Disconnecting {
                    was_connected: true
                }
        )
    }

    fn is_disconnecting(self) -> bool {
        matches!(self, Self::Disconnecting { .. })
    }
}

#[derive(Debug, Clone)]
struct RegistryEntry {
    profile_revision: i64,
    generation: u64,
    phase: McpUsePhase,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct QueuedControl {
    owner: OwnershipKey,
    command: McpControlCommand,
    was_connected: bool,
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
    next_generation: u64,
    entries: HashMap<OwnershipKey, RegistryEntry>,
    controls: HashMap<Uuid, QueuedControl>,
    control_queues: HashMap<SessionKey, VecDeque<Uuid>>,
    closed_sessions: HashSet<SessionKey>,
    closed_services: HashSet<Uuid>,
    completed_operations: HashMap<OperationKey, McpSyncResponse>,
    operation_order: VecDeque<OperationKey>,
}

#[derive(Clone)]
pub struct McpSyncRegistry {
    inner: Arc<Mutex<RegistryState>>,
    operation_locks: Arc<Mutex<HashMap<OperationKey, Weak<Mutex<()>>>>>,
    lifecycle_locks: LifecycleLocks,
    control_notifies: ControlNotifies,
    control_waiters: ControlWaiters,
    repository: Option<SharedConnectionRepository>,
    app_handle: Option<AppHandle>,
    #[cfg(test)]
    test_profiles: Arc<Mutex<HashMap<String, (i64, bool)>>>,
}

impl McpSyncRegistry {
    pub fn new(app_handle: AppHandle) -> Self {
        let repository = app_handle.state::<AppState>().connection_repository.clone();
        Self {
            inner: Arc::new(Mutex::new(RegistryState::default())),
            operation_locks: Arc::new(Mutex::new(HashMap::new())),
            lifecycle_locks: Arc::new(Mutex::new(HashMap::new())),
            control_notifies: Arc::new(Mutex::new(HashMap::new())),
            control_waiters: Arc::new(Mutex::new(HashMap::new())),
            repository: Some(repository),
            app_handle: Some(app_handle),
            #[cfg(test)]
            test_profiles: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    fn without_app_events() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RegistryState::default())),
            operation_locks: Arc::new(Mutex::new(HashMap::new())),
            lifecycle_locks: Arc::new(Mutex::new(HashMap::new())),
            control_notifies: Arc::new(Mutex::new(HashMap::new())),
            control_waiters: Arc::new(Mutex::new(HashMap::new())),
            repository: None,
            app_handle: None,
            test_profiles: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    async fn allow_test_profile(&self, connection_id: &str, revision: i64, enabled: bool) {
        self.test_profiles
            .lock()
            .await
            .insert(connection_id.to_string(), (revision, enabled));
    }

    pub async fn snapshot(&self) -> McpConnectionsSnapshot {
        let state = self.inner.lock().await;
        snapshot_from_state(&state)
    }

    /// Serialize profile mutation with HTTP MCP acquire/release transitions.
    ///
    /// App commands must retain this guard across both
    /// `is_connection_in_use` and the repository save/delete operation.
    pub async fn lock_connection_lifecycle(&self, connection_id: &str) -> OwnedMutexGuard<()> {
        let lifecycle = {
            let mut lifecycles = self.lifecycle_locks.lock().await;
            if lifecycles.len() >= MAX_REMEMBERED_OPERATIONS {
                lifecycles.retain(|_, lifecycle| lifecycle.strong_count() > 0);
            }
            if let Some(lifecycle) = lifecycles.get(connection_id).and_then(Weak::upgrade) {
                lifecycle
            } else {
                let lifecycle = Arc::new(Mutex::new(()));
                lifecycles.insert(connection_id.to_string(), Arc::downgrade(&lifecycle));
                lifecycle
            }
        };
        lifecycle.lock_owned().await
    }

    pub async fn is_connection_in_use(&self, connection_id: &str) -> bool {
        self.inner
            .lock()
            .await
            .entries
            .keys()
            .any(|key| key.connection_id == connection_id)
    }

    /// Ask every App-managed HTTP session using `connection_id` to release the
    /// exact generation it currently owns.
    ///
    /// The queued command remains generation-scoped after a timeout. A delayed
    /// command therefore cannot disconnect a later reconnect.
    pub async fn force_disconnect(
        &self,
        connection_id: &str,
    ) -> Result<ForceDisconnectResult, String> {
        validate_connection_id(connection_id)?;
        let _lifecycle = self.lock_connection_lifecycle(connection_id).await;
        let mut waiter_receivers = Vec::new();
        let mut sessions_to_wake = HashSet::new();
        let mut command_ids = Vec::new();
        let snapshot = {
            // Register waiters while the registry state is locked so an
            // already-delivered command cannot complete between observation
            // and waiter installation.
            let mut waiters = self.control_waiters.lock().await;
            let mut state = self.inner.lock().await;
            let owners = state
                .entries
                .iter()
                .filter(|(key, _)| key.connection_id == connection_id)
                .map(|(key, entry)| (key.clone(), entry.generation, entry.phase.is_connected()))
                .collect::<Vec<_>>();
            if owners.is_empty() {
                return Ok(ForceDisconnectResult {
                    requested: 0,
                    completed: 0,
                });
            }

            for (owner, generation, was_connected) in owners {
                let existing_command = state
                    .controls
                    .values()
                    .find(|queued| queued.owner == owner && queued.command.generation == generation)
                    .map(|queued| queued.command.clone());
                let command = existing_command.unwrap_or_else(|| McpControlCommand {
                    command_id: Uuid::new_v4(),
                    connection_id: connection_id.to_string(),
                    generation,
                });
                if !state.controls.contains_key(&command.command_id) {
                    state.controls.insert(
                        command.command_id,
                        QueuedControl {
                            owner: owner.clone(),
                            command: command.clone(),
                            was_connected,
                        },
                    );
                    state
                        .control_queues
                        .entry(owner.session_key())
                        .or_default()
                        .push_back(command.command_id);
                }
                if let Some(entry) = state.entries.get_mut(&owner) {
                    entry.phase = McpUsePhase::Disconnecting { was_connected };
                    entry.last_error = None;
                }
                let (sender, receiver) = oneshot::channel();
                waiters.entry(command.command_id).or_default().push(sender);
                waiter_receivers.push(receiver);
                command_ids.push(command.command_id);
                sessions_to_wake.insert(owner.session_key());
            }
            state.revision = state.revision.saturating_add(1);
            snapshot_from_state(&state)
        };
        drop(_lifecycle);

        self.emit(Some(snapshot)).await;
        for session in sessions_to_wake {
            self.wake_session(session).await;
        }

        let requested = waiter_receivers.len();
        let deadline = tokio::time::Instant::now() + FORCE_DISCONNECT_TIMEOUT;
        let mut completed = 0;
        let mut errors = Vec::new();
        for receiver in waiter_receivers {
            match tokio::time::timeout_at(deadline, receiver).await {
                Ok(Ok(Ok(()))) => completed += 1,
                Ok(Ok(Err(error))) => errors.push(error),
                Ok(Err(_)) => errors
                    .push("Streamable HTTP MCP disconnect acknowledgement was dropped".to_string()),
                Err(_) => errors.push(FORCE_DISCONNECT_TIMEOUT_MESSAGE.to_string()),
            }
        }

        if errors.is_empty() {
            return Ok(ForceDisconnectResult {
                requested,
                completed,
            });
        }

        let timeout_snapshot = {
            let mut state = self.inner.lock().await;
            let mut changed = false;
            for command_id in command_ids {
                let Some(queued) = state.controls.get(&command_id).cloned() else {
                    continue;
                };
                if let Some(entry) = state.entries.get_mut(&queued.owner) {
                    if entry.generation == queued.command.generation {
                        entry.last_error = Some(errors.join("; "));
                        changed = true;
                    }
                }
            }
            changed.then(|| {
                state.revision = state.revision.saturating_add(1);
                snapshot_from_state(&state)
            })
        };
        self.emit(timeout_snapshot).await;
        Err(format!(
            "Unable to disconnect all Streamable HTTP MCP sessions ({completed}/{requested} completed): {}",
            errors.join("; ")
        ))
    }

    async fn apply(&self, expected_service_id: Uuid, request: McpSyncRequest) -> McpSyncResponse {
        let context = request_context(&request);
        if let Err(error) = validate_context(context, expected_service_id) {
            return failure(error);
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
            McpSyncRequest::Acquire {
                context,
                connection_id,
                profile_revision,
            } => self.acquire(context, connection_id, profile_revision).await,
            McpSyncRequest::Connected {
                context,
                connection_id,
                generation,
            } => self.connected(context, connection_id, generation).await,
            McpSyncRequest::Released {
                context,
                connection_id,
                generation,
            } => self.released(context, connection_id, generation).await,
            McpSyncRequest::PollControl { context } => self.poll_control(context).await,
            McpSyncRequest::ControlResult {
                context,
                command_id,
                connection_id,
                generation,
                ok,
                error,
            } => {
                self.control_result(context, command_id, connection_id, generation, ok, error)
                    .await
            }
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

    async fn acquire(
        &self,
        context: McpSyncContext,
        connection_id: String,
        profile_revision: i64,
    ) -> McpSyncResponse {
        if let Err(error) = validate_connection_id(&connection_id) {
            return failure(error);
        }
        if profile_revision < 0 {
            return failure("Shared connection profile revision must not be negative");
        }
        let key = OwnershipKey::new(
            context.service_id,
            context.session_id,
            connection_id.clone(),
        );
        let _lifecycle = self.lock_connection_lifecycle(&connection_id).await;

        {
            let state = self.inner.lock().await;
            if let Some(error) = ensure_owner_is_open(&state, &key) {
                return failure(error);
            }
            if let Some(entry) = state.entries.get(&key) {
                if entry.profile_revision != profile_revision {
                    return failure(
                        "The MCP session already owns a different revision of this connection",
                    );
                }
                if entry.phase.is_disconnecting() {
                    return failure("The shared connection is being forcibly disconnected");
                }
                return success(Some(entry.generation), None);
            }
        }

        if let Err(error) = self
            .validate_shared_profile(&connection_id, profile_revision)
            .await
        {
            return failure(error);
        }

        let snapshot_and_generation = {
            let mut state = self.inner.lock().await;
            if let Some(error) = ensure_owner_is_open(&state, &key) {
                return failure(error);
            }
            if let Some(entry) = state.entries.get(&key) {
                if entry.profile_revision == profile_revision && !entry.phase.is_disconnecting() {
                    return success(Some(entry.generation), None);
                }
                return failure("The MCP session connection ownership changed while acquiring");
            }
            state.next_generation = state.next_generation.saturating_add(1).max(1);
            let generation = state.next_generation;
            state.entries.insert(
                key,
                RegistryEntry {
                    profile_revision,
                    generation,
                    phase: McpUsePhase::Acquired,
                    last_error: None,
                },
            );
            state.revision = state.revision.saturating_add(1);
            (snapshot_from_state(&state), generation)
        };
        self.emit(Some(snapshot_and_generation.0)).await;
        success(Some(snapshot_and_generation.1), None)
    }

    async fn connected(
        &self,
        context: McpSyncContext,
        connection_id: String,
        generation: u64,
    ) -> McpSyncResponse {
        if let Err(error) = validate_connection_id(&connection_id) {
            return failure(error);
        }
        if generation == 0 {
            return failure("MCP connection generation must be greater than zero");
        }
        let key = OwnershipKey::new(
            context.service_id,
            context.session_id,
            connection_id.clone(),
        );
        let _lifecycle = self.lock_connection_lifecycle(&connection_id).await;
        let snapshot = {
            let mut state = self.inner.lock().await;
            if let Some(error) = ensure_owner_is_open(&state, &key) {
                return failure(error);
            }
            let Some(entry) = state.entries.get_mut(&key) else {
                return failure("The shared connection has not been acquired");
            };
            if entry.generation != generation {
                return failure("The shared connection generation is stale");
            }
            match entry.phase {
                McpUsePhase::Acquired => {
                    entry.phase = McpUsePhase::Connected;
                    entry.last_error = None;
                    state.revision = state.revision.saturating_add(1);
                    Some(snapshot_from_state(&state))
                }
                McpUsePhase::Connected => None,
                McpUsePhase::Disconnecting { .. } => {
                    return failure("The shared connection is being forcibly disconnected")
                }
            }
        };
        self.emit(snapshot).await;
        success(Some(generation), None)
    }

    async fn released(
        &self,
        context: McpSyncContext,
        connection_id: String,
        generation: u64,
    ) -> McpSyncResponse {
        if let Err(error) = validate_connection_id(&connection_id) {
            return failure(error);
        }
        if generation == 0 {
            return failure("MCP connection generation must be greater than zero");
        }
        let key = OwnershipKey::new(
            context.service_id,
            context.session_id,
            connection_id.clone(),
        );
        let _lifecycle = self.lock_connection_lifecycle(&connection_id).await;
        let (snapshot, completed_controls) = {
            let mut state = self.inner.lock().await;
            if let Some(error) = ensure_owner_is_open(&state, &key) {
                return failure(error);
            }
            let matches_generation = state
                .entries
                .get(&key)
                .is_some_and(|entry| entry.generation == generation);
            if !matches_generation {
                return success(Some(generation), None);
            }
            state.entries.remove(&key);
            let completed_controls = remove_controls_for_owner(&mut state, &key, Some(generation));
            state.revision = state.revision.saturating_add(1);
            (Some(snapshot_from_state(&state)), completed_controls)
        };
        self.resolve_waiters(completed_controls, Ok(())).await;
        self.emit(snapshot).await;
        success(Some(generation), None)
    }

    async fn poll_control(&self, context: McpSyncContext) -> McpSyncResponse {
        let session = (context.service_id, context.session_id);
        let notify = self.control_notify(session).await;
        let deadline = tokio::time::Instant::now() + CONTROL_POLL_TIMEOUT;
        loop {
            let control = {
                let mut state = self.inner.lock().await;
                if state.closed_services.contains(&context.service_id)
                    || state.closed_sessions.contains(&session)
                {
                    return failure("MCP session is closed");
                }
                next_control_for_session(&mut state, session)
            };
            if let Some(control) = control {
                return success(None, Some(control));
            }

            let notified = notify.notified();
            match tokio::time::timeout_at(deadline, notified).await {
                Ok(()) => {}
                Err(_) => return success(None, None),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn control_result(
        &self,
        context: McpSyncContext,
        command_id: Uuid,
        connection_id: String,
        generation: u64,
        ok: bool,
        error: Option<String>,
    ) -> McpSyncResponse {
        if command_id.is_nil() {
            return failure("MCP control command identifier must not be nil");
        }
        if let Err(error) = validate_connection_id(&connection_id) {
            return failure(error);
        }
        if generation == 0 {
            return failure("MCP connection generation must be greater than zero");
        }
        if ok && error.is_some() {
            return failure("A successful MCP control result must not include an error");
        }
        if !ok && error.as_deref().is_none_or(str::is_empty) {
            return failure("A failed MCP control result must include an error");
        }
        if error
            .as_ref()
            .is_some_and(|message| message.len() > MAX_CONTROL_ERROR_BYTES)
        {
            return failure(format!(
                "MCP control error must not exceed {MAX_CONTROL_ERROR_BYTES} bytes"
            ));
        }

        let _lifecycle = self.lock_connection_lifecycle(&connection_id).await;
        let (snapshot, completion) = {
            let mut state = self.inner.lock().await;
            let Some(queued) = state.controls.get(&command_id).cloned() else {
                // A normal release or session close may have completed the
                // generation while this acknowledgement was in flight.
                return success(Some(generation), None);
            };
            if queued.owner.service_id != context.service_id
                || queued.owner.session_id != context.session_id
                || queued.command.connection_id != connection_id
                || queued.command.generation != generation
            {
                return failure("MCP control acknowledgement does not match the queued command");
            }

            remove_control(&mut state, command_id);
            let mut changed = true;
            let completion = if ok {
                let matches_generation = state
                    .entries
                    .get(&queued.owner)
                    .is_some_and(|entry| entry.generation == generation);
                if matches_generation {
                    state.entries.remove(&queued.owner);
                }
                Ok(())
            } else {
                let message = error.unwrap_or_else(|| "MCP disconnect failed".into());
                if let Some(entry) = state.entries.get_mut(&queued.owner) {
                    if entry.generation == generation {
                        entry.phase = if queued.was_connected {
                            McpUsePhase::Connected
                        } else {
                            McpUsePhase::Acquired
                        };
                        entry.last_error = Some(message.clone());
                    } else {
                        changed = false;
                    }
                } else {
                    changed = false;
                }
                Err(message)
            };
            let snapshot = changed.then(|| {
                state.revision = state.revision.saturating_add(1);
                snapshot_from_state(&state)
            });
            (snapshot, completion)
        };
        self.resolve_waiters(vec![command_id], completion).await;
        self.emit(snapshot).await;
        success(Some(generation), None)
    }

    async fn close_session(&self, context: McpSyncContext) -> McpSyncResponse {
        let session = (context.service_id, context.session_id);
        let (snapshot, completed_controls) = {
            let mut state = self.inner.lock().await;
            state.closed_sessions.insert(session);
            let keys = state
                .entries
                .keys()
                .filter(|key| key.session_key() == session)
                .cloned()
                .collect::<Vec<_>>();
            for key in &keys {
                state.entries.remove(key);
            }
            let completed_controls = remove_controls_for_session(&mut state, session);
            let snapshot = (!keys.is_empty()).then(|| {
                state.revision = state.revision.saturating_add(1);
                snapshot_from_state(&state)
            });
            (snapshot, completed_controls)
        };
        self.resolve_waiters(completed_controls, Ok(())).await;
        self.wake_session(session).await;
        self.emit(snapshot).await;
        success(None, None)
    }

    async fn reset_service(&self, service_id: Uuid) {
        let (snapshot, completed_controls, sessions) = {
            let mut state = self.inner.lock().await;
            state.closed_services.insert(service_id);
            let sessions = state
                .entries
                .keys()
                .filter(|key| key.service_id == service_id)
                .map(OwnershipKey::session_key)
                .collect::<HashSet<_>>();
            let keys = state
                .entries
                .keys()
                .filter(|key| key.service_id == service_id)
                .cloned()
                .collect::<Vec<_>>();
            for key in &keys {
                state.entries.remove(key);
            }
            let control_ids = state
                .controls
                .iter()
                .filter(|(_, queued)| queued.owner.service_id == service_id)
                .map(|(command_id, _)| *command_id)
                .collect::<Vec<_>>();
            for command_id in &control_ids {
                remove_control(&mut state, *command_id);
            }
            state
                .completed_operations
                .retain(|key, _| key.service_id != service_id);
            state
                .operation_order
                .retain(|key| key.service_id != service_id);
            state
                .closed_sessions
                .retain(|(owner_service, _)| *owner_service != service_id);
            let snapshot = (!keys.is_empty()).then(|| {
                state.revision = state.revision.saturating_add(1);
                snapshot_from_state(&state)
            });
            (snapshot, control_ids, sessions)
        };
        self.resolve_waiters(completed_controls, Ok(())).await;
        for session in sessions {
            self.wake_session(session).await;
        }
        self.emit(snapshot).await;
    }

    async fn validate_shared_profile(
        &self,
        connection_id: &str,
        expected_revision: i64,
    ) -> Result<(), String> {
        if let Some(repository) = self.repository.as_ref() {
            let profile = repository
                .get(connection_id)
                .await
                .map_err(|error| error.to_string())?;
            if !profile.mcp_enabled {
                return Err(format!("连接 {connection_id} 未允许 MCP 使用"));
            }
            if profile.revision != expected_revision {
                return Err(format!(
                    "连接 {connection_id} revision 已从 {expected_revision} 变为 {}，请重新调用 list_connections",
                    profile.revision
                ));
            }
            return Ok(());
        }

        #[cfg(test)]
        {
            let profiles = self.test_profiles.lock().await;
            let Some((revision, enabled)) = profiles.get(connection_id).copied() else {
                return Err(format!("连接 {connection_id} 不存在"));
            };
            if !enabled {
                return Err(format!("连接 {connection_id} 未允许 MCP 使用"));
            }
            if revision != expected_revision {
                return Err(format!(
                    "连接 {connection_id} revision 已从 {expected_revision} 变为 {revision}"
                ));
            }
            return Ok(());
        }

        #[cfg(not(test))]
        Err("Astesia App shared connection repository is unavailable".into())
    }

    async fn control_notify(&self, session: SessionKey) -> Arc<Notify> {
        let mut notifies = self.control_notifies.lock().await;
        notifies
            .entry(session)
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }

    async fn wake_session(&self, session: SessionKey) {
        let notify = self.control_notify(session).await;
        // `notify_waiters` wakes currently parked polls; `notify_one` also
        // stores a permit when the poll is between requests.
        notify.notify_waiters();
        notify.notify_one();
    }

    async fn resolve_waiters(&self, command_ids: Vec<Uuid>, result: Result<(), String>) {
        if command_ids.is_empty() {
            return;
        }
        let senders = {
            let mut waiters = self.control_waiters.lock().await;
            command_ids
                .into_iter()
                .flat_map(|command_id| waiters.remove(&command_id).unwrap_or_default())
                .collect::<Vec<_>>()
        };
        for sender in senders {
            let _ = sender.send(result.clone());
        }
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

fn next_control_for_session(
    state: &mut RegistryState,
    session: SessionKey,
) -> Option<McpControlCommand> {
    let queue = state.control_queues.get_mut(&session)?;
    while let Some(command_id) = queue.front().copied() {
        if let Some(control) = state.controls.get(&command_id) {
            return Some(control.command.clone());
        }
        queue.pop_front();
    }
    state.control_queues.remove(&session);
    None
}

fn remove_control(state: &mut RegistryState, command_id: Uuid) -> Option<QueuedControl> {
    let removed = state.controls.remove(&command_id)?;
    let session = removed.owner.session_key();
    if let Some(queue) = state.control_queues.get_mut(&session) {
        queue.retain(|queued_id| *queued_id != command_id);
        if queue.is_empty() {
            state.control_queues.remove(&session);
        }
    }
    Some(removed)
}

fn remove_controls_for_owner(
    state: &mut RegistryState,
    owner: &OwnershipKey,
    generation: Option<u64>,
) -> Vec<Uuid> {
    let command_ids = state
        .controls
        .iter()
        .filter(|(_, queued)| {
            &queued.owner == owner
                && generation.is_none_or(|expected| queued.command.generation == expected)
        })
        .map(|(command_id, _)| *command_id)
        .collect::<Vec<_>>();
    for command_id in &command_ids {
        remove_control(state, *command_id);
    }
    command_ids
}

fn remove_controls_for_session(state: &mut RegistryState, session: SessionKey) -> Vec<Uuid> {
    let command_ids = state
        .controls
        .iter()
        .filter(|(_, queued)| queued.owner.session_key() == session)
        .map(|(command_id, _)| *command_id)
        .collect::<Vec<_>>();
    for command_id in &command_ids {
        remove_control(state, *command_id);
    }
    state.control_queues.remove(&session);
    command_ids
}

fn snapshot_from_state(state: &RegistryState) -> McpConnectionsSnapshot {
    #[derive(Default)]
    struct Aggregate {
        profile_revision: i64,
        connected: bool,
        session_count: usize,
        disconnecting: bool,
        last_error: Option<String>,
    }

    let mut aggregates = HashMap::<String, Aggregate>::new();
    for (key, entry) in &state.entries {
        let aggregate = aggregates.entry(key.connection_id.clone()).or_default();
        aggregate.profile_revision = aggregate.profile_revision.max(entry.profile_revision);
        aggregate.connected |= entry.phase.is_connected();
        aggregate.session_count += 1;
        aggregate.disconnecting |= entry.phase.is_disconnecting();
        if aggregate.last_error.is_none() {
            aggregate.last_error.clone_from(&entry.last_error);
        }
    }
    let mut connections = aggregates
        .into_iter()
        .map(|(id, aggregate)| McpConnectionSnapshot {
            id,
            profile_revision: aggregate.profile_revision,
            mcp_in_use: true,
            mcp_connected: aggregate.connected,
            mcp_session_count: aggregate.session_count,
            disconnecting: aggregate.disconnecting,
            last_error: aggregate.last_error,
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
    } else if state.closed_sessions.contains(&key.session_key()) {
        Some("MCP session is closed".into())
    } else {
        None
    }
}

fn validate_connection_id(connection_id: &str) -> Result<(), String> {
    if connection_id.trim().is_empty() {
        return Err("MCP connection identifier must not be empty".into());
    }
    if connection_id.len() > MAX_IDENTIFIER_BYTES {
        return Err(format!(
            "MCP connection identifier must not exceed {MAX_IDENTIFIER_BYTES} bytes"
        ));
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
        McpSyncRequest::Acquire { context, .. }
        | McpSyncRequest::Connected { context, .. }
        | McpSyncRequest::Released { context, .. }
        | McpSyncRequest::PollControl { context }
        | McpSyncRequest::ControlResult { context, .. }
        | McpSyncRequest::SessionClosed { context } => context,
    }
}

fn success(generation: Option<u64>, control: Option<McpControlCommand>) -> McpSyncResponse {
    McpSyncResponse {
        ok: true,
        error: None,
        generation,
        control,
    }
}

fn failure(message: impl Into<String>) -> McpSyncResponse {
    McpSyncResponse {
        ok: false,
        error: Some(message.into()),
        generation: None,
        control: None,
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
        return response(StatusCode::UNAUTHORIZED, failure("Unauthorized"));
    }
    let body = match to_bytes(request.into_body(), MAX_REQUEST_BYTES).await {
        Ok(body) => body,
        Err(_) => {
            return response(
                StatusCode::BAD_REQUEST,
                failure("Invalid MCP synchronization request body"),
            )
        }
    };
    let request = match serde_json::from_slice::<McpSyncRequest>(&body) {
        Ok(request) => request,
        Err(_) => {
            return response(
                StatusCode::BAD_REQUEST,
                failure("Invalid MCP synchronization request"),
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

    async fn acquire(
        registry: &McpSyncRegistry,
        service_id: Uuid,
        session_id: Uuid,
        connection_id: &str,
        revision: i64,
    ) -> McpSyncResponse {
        registry
            .apply(
                service_id,
                McpSyncRequest::Acquire {
                    context: context(service_id, session_id),
                    connection_id: connection_id.into(),
                    profile_revision: revision,
                },
            )
            .await
    }

    async fn connected(
        registry: &McpSyncRegistry,
        service_id: Uuid,
        session_id: Uuid,
        connection_id: &str,
        generation: u64,
    ) -> McpSyncResponse {
        registry
            .apply(
                service_id,
                McpSyncRequest::Connected {
                    context: context(service_id, session_id),
                    connection_id: connection_id.into(),
                    generation,
                },
            )
            .await
    }

    async fn released(
        registry: &McpSyncRegistry,
        service_id: Uuid,
        session_id: Uuid,
        connection_id: &str,
        generation: u64,
    ) -> McpSyncResponse {
        registry
            .apply(
                service_id,
                McpSyncRequest::Released {
                    context: context(service_id, session_id),
                    connection_id: connection_id.into(),
                    generation,
                },
            )
            .await
    }

    #[tokio::test]
    async fn acquire_uses_canonical_shared_id_and_never_serializes_profile_or_credentials() {
        let registry = McpSyncRegistry::without_app_events();
        registry.allow_test_profile("shared-id", 7, true).await;
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();

        let response = acquire(&registry, service_id, session_id, "shared-id", 7).await;
        assert!(response.ok);
        assert!(response.generation.is_some());
        let snapshot = registry.snapshot().await;
        assert_eq!(snapshot.connections.len(), 1);
        assert_eq!(snapshot.connections[0].id, "shared-id");
        assert_eq!(snapshot.connections[0].profile_revision, 7);
        assert!(snapshot.connections[0].mcp_in_use);
        assert!(!snapshot.connections[0].mcp_connected);

        let value = serde_json::to_value(snapshot).expect("serialize snapshot");
        let serialized = serde_json::to_string(&value).expect("serialize JSON");
        for forbidden in [
            "password",
            "password_env",
            "username",
            "host",
            "database",
            "credential",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "snapshot leaked forbidden field {forbidden}"
            );
        }
    }

    #[tokio::test]
    async fn acquire_rejects_unknown_disabled_or_stale_shared_profiles() {
        let registry = McpSyncRegistry::without_app_events();
        registry.allow_test_profile("disabled", 2, false).await;
        registry.allow_test_profile("current", 5, true).await;
        let service_id = Uuid::new_v4();

        for (id, revision) in [("missing", 1), ("disabled", 2), ("current", 4)] {
            let response = acquire(&registry, service_id, Uuid::new_v4(), id, revision).await;
            assert!(!response.ok, "{id} should be rejected");
        }
        assert!(registry.snapshot().await.connections.is_empty());
    }

    #[tokio::test]
    async fn identical_connection_ids_aggregate_across_http_sessions() {
        let registry = McpSyncRegistry::without_app_events();
        registry.allow_test_profile("shared", 3, true).await;
        let service_id = Uuid::new_v4();
        let session_a = Uuid::new_v4();
        let session_b = Uuid::new_v4();
        let generation_a = acquire(&registry, service_id, session_a, "shared", 3)
            .await
            .generation
            .unwrap();
        let generation_b = acquire(&registry, service_id, session_b, "shared", 3)
            .await
            .generation
            .unwrap();
        assert_ne!(generation_a, generation_b);
        connected(&registry, service_id, session_a, "shared", generation_a).await;
        connected(&registry, service_id, session_b, "shared", generation_b).await;

        let snapshot = registry.snapshot().await;
        assert_eq!(snapshot.connections.len(), 1);
        assert_eq!(snapshot.connections[0].mcp_session_count, 2);
        assert!(snapshot.connections[0].mcp_connected);

        released(&registry, service_id, session_a, "shared", generation_a).await;
        let snapshot = registry.snapshot().await;
        assert_eq!(snapshot.connections[0].mcp_session_count, 1);
        assert!(registry.is_connection_in_use("shared").await);
    }

    #[tokio::test]
    async fn lifecycle_guard_linearizes_profile_mutation_and_acquire() {
        let registry = McpSyncRegistry::without_app_events();
        registry.allow_test_profile("guarded", 1, true).await;
        let guard = registry.lock_connection_lifecycle("guarded").await;
        let acquire_registry = registry.clone();
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task = tokio::spawn(async move {
            acquire(&acquire_registry, service_id, session_id, "guarded", 1).await
        });

        tokio::task::yield_now().await;
        assert!(!registry.is_connection_in_use("guarded").await);
        assert!(!task.is_finished());
        drop(guard);
        assert!(task.await.expect("acquire task").ok);
        assert!(registry.is_connection_in_use("guarded").await);
    }

    #[tokio::test]
    async fn force_disconnect_is_pushed_to_the_target_session_and_acknowledged() {
        let registry = McpSyncRegistry::without_app_events();
        registry.allow_test_profile("force", 9, true).await;
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let generation = acquire(&registry, service_id, session_id, "force", 9)
            .await
            .generation
            .unwrap();
        assert!(
            connected(&registry, service_id, session_id, "force", generation)
                .await
                .ok
        );

        let force_registry = registry.clone();
        let force_task =
            tokio::spawn(async move { force_registry.force_disconnect("force").await });
        tokio::task::yield_now().await;
        let poll = registry
            .apply(
                service_id,
                McpSyncRequest::PollControl {
                    context: context(service_id, session_id),
                },
            )
            .await;
        let command = poll.control.expect("force-disconnect command");
        assert_eq!(command.connection_id, "force");
        assert_eq!(command.generation, generation);

        let acknowledgement = registry
            .apply(
                service_id,
                McpSyncRequest::ControlResult {
                    context: context(service_id, session_id),
                    command_id: command.command_id,
                    connection_id: command.connection_id,
                    generation: command.generation,
                    ok: true,
                    error: None,
                },
            )
            .await;
        assert!(acknowledgement.ok);
        let result = force_task
            .await
            .expect("force task")
            .expect("force disconnect");
        assert_eq!(
            result,
            ForceDisconnectResult {
                requested: 1,
                completed: 1
            }
        );
        assert!(!registry.is_connection_in_use("force").await);
        assert!(registry.snapshot().await.connections.is_empty());
    }

    #[tokio::test]
    async fn delayed_force_command_cannot_remove_a_reconnected_generation() {
        let registry = McpSyncRegistry::without_app_events();
        registry.allow_test_profile("aba", 4, true).await;
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let first_generation = acquire(&registry, service_id, session_id, "aba", 4)
            .await
            .generation
            .unwrap();
        connected(&registry, service_id, session_id, "aba", first_generation).await;

        let force_registry = registry.clone();
        let force_task = tokio::spawn(async move { force_registry.force_disconnect("aba").await });
        tokio::task::yield_now().await;
        let command = registry
            .apply(
                service_id,
                McpSyncRequest::PollControl {
                    context: context(service_id, session_id),
                },
            )
            .await
            .control
            .expect("queued control");
        assert_eq!(command.generation, first_generation);

        released(&registry, service_id, session_id, "aba", first_generation).await;
        assert!(force_task.await.expect("force task").is_ok());
        let second_generation = acquire(&registry, service_id, session_id, "aba", 4)
            .await
            .generation
            .unwrap();
        assert_ne!(first_generation, second_generation);
        connected(&registry, service_id, session_id, "aba", second_generation).await;

        let late_ack = registry
            .apply(
                service_id,
                McpSyncRequest::ControlResult {
                    context: context(service_id, session_id),
                    command_id: command.command_id,
                    connection_id: command.connection_id,
                    generation: command.generation,
                    ok: true,
                    error: None,
                },
            )
            .await;
        assert!(late_ack.ok);
        let snapshot = registry.snapshot().await;
        assert_eq!(snapshot.connections.len(), 1);
        assert!(snapshot.connections[0].mcp_connected);
        let state = registry.inner.lock().await;
        let entry = state.entries.values().next().expect("reconnected entry");
        assert_eq!(entry.generation, second_generation);
    }

    #[tokio::test]
    async fn failed_force_disconnect_keeps_profile_in_use_and_reports_error() {
        let registry = McpSyncRegistry::without_app_events();
        registry.allow_test_profile("nack", 1, true).await;
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let generation = acquire(&registry, service_id, session_id, "nack", 1)
            .await
            .generation
            .unwrap();
        connected(&registry, service_id, session_id, "nack", generation).await;

        let force_registry = registry.clone();
        let force_task = tokio::spawn(async move { force_registry.force_disconnect("nack").await });
        tokio::task::yield_now().await;
        let command = registry
            .apply(
                service_id,
                McpSyncRequest::PollControl {
                    context: context(service_id, session_id),
                },
            )
            .await
            .control
            .expect("queued control");
        registry
            .apply(
                service_id,
                McpSyncRequest::ControlResult {
                    context: context(service_id, session_id),
                    command_id: command.command_id,
                    connection_id: command.connection_id,
                    generation: command.generation,
                    ok: false,
                    error: Some("driver refused to close".into()),
                },
            )
            .await;

        let error = force_task
            .await
            .expect("force task")
            .expect_err("nack must fail force disconnect");
        assert!(error.contains("driver refused to close"));
        assert!(registry.is_connection_in_use("nack").await);
        let snapshot = registry.snapshot().await;
        assert!(snapshot.connections[0].mcp_connected);
        assert!(!snapshot.connections[0].disconnecting);
        assert_eq!(
            snapshot.connections[0].last_error.as_deref(),
            Some("driver refused to close")
        );
    }

    #[tokio::test]
    async fn closing_session_releases_usage_and_completes_pending_force_disconnect() {
        let registry = McpSyncRegistry::without_app_events();
        registry.allow_test_profile("close", 2, true).await;
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let generation = acquire(&registry, service_id, session_id, "close", 2)
            .await
            .generation
            .unwrap();
        connected(&registry, service_id, session_id, "close", generation).await;

        let force_registry = registry.clone();
        let force_task =
            tokio::spawn(async move { force_registry.force_disconnect("close").await });
        tokio::task::yield_now().await;
        let closed = registry
            .apply(
                service_id,
                McpSyncRequest::SessionClosed {
                    context: context(service_id, session_id),
                },
            )
            .await;
        assert!(closed.ok);
        assert!(force_task.await.expect("force task").is_ok());
        assert!(!registry.is_connection_in_use("close").await);

        let late = acquire(&registry, service_id, session_id, "close", 2).await;
        assert!(!late.ok);
    }

    #[tokio::test]
    async fn duplicate_operation_id_returns_the_original_acquire_generation() {
        let registry = McpSyncRegistry::without_app_events();
        registry.allow_test_profile("idempotent", 1, true).await;
        let service_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let request = McpSyncRequest::Acquire {
            context: context(service_id, session_id),
            connection_id: "idempotent".into(),
            profile_revision: 1,
        };

        let first = registry.apply(service_id, request.clone()).await;
        let second = registry.apply(service_id, request).await;
        assert!(first.ok);
        assert_eq!(first, second);
        assert_eq!(registry.snapshot().await.connections.len(), 1);
    }

    #[test]
    fn snapshot_contract_contains_only_shared_usage_state() {
        let mut state = RegistryState {
            revision: 3,
            ..RegistryState::default()
        };
        state.entries.insert(
            OwnershipKey::new(Uuid::new_v4(), Uuid::new_v4(), "shared".into()),
            RegistryEntry {
                profile_revision: 8,
                generation: 2,
                phase: McpUsePhase::Connected,
                last_error: None,
            },
        );
        let value = serde_json::to_value(snapshot_from_state(&state)).unwrap();
        assert_eq!(value["revision"], 3);
        assert_eq!(value["connections"][0]["id"], "shared");
        assert_eq!(value["connections"][0]["profile_revision"], 8);
        assert_eq!(value["connections"][0]["mcp_in_use"], true);
        assert_eq!(value["connections"][0]["mcp_connected"], true);
        assert_eq!(value["connections"][0]["mcp_session_count"], 1);
        assert_eq!(value["connections"][0]["disconnecting"], false);
        let object = value["connections"][0].as_object().unwrap();
        for forbidden in [
            "name",
            "db_type",
            "host",
            "port",
            "username",
            "database",
            "password",
            "password_env",
            "source",
            "app_connected",
        ] {
            assert!(!object.contains_key(forbidden), "unexpected {forbidden}");
        }
    }

    #[test]
    fn bearer_comparison_is_exact() {
        assert!(constant_time_eq(b"Bearer abc", b"Bearer abc"));
        assert!(!constant_time_eq(b"Bearer abc", b"Bearer abd"));
        assert!(!constant_time_eq(b"Bearer abc", b"Bearer abc "));
    }

    #[test]
    fn protocol_version_rejects_old_transient_profile_clients() {
        let service_id = Uuid::new_v4();
        let mut request_context = context(service_id, Uuid::new_v4());
        request_context.protocol_version = 1;
        assert!(validate_context(&request_context, service_id).is_err());
    }

    #[test]
    fn response_serializes_control_without_any_connection_profile() {
        let command = McpControlCommand {
            command_id: Uuid::new_v4(),
            connection_id: "shared".into(),
            generation: 4,
        };
        let value = serde_json::to_value(success(None, Some(command))).unwrap();
        assert_eq!(value["ok"], Value::Bool(true));
        assert_eq!(value["control"]["connection_id"], "shared");
        assert_eq!(value["control"]["generation"], 4);
        assert!(value.get("generation").is_none());
    }
}
