use std::{
    collections::{hash_map::Entry, HashSet},
    sync::Arc,
    time::Duration,
};

use tokio::sync::{oneshot, Notify};
use uuid::Uuid;

use crate::mcp_sync::{McpControlCommand, McpSyncContext, McpSyncResponse};

use super::{
    protocol::{failure, success, validate_connection_id},
    state::{
        next_control_for_session, remove_control, remove_controls_for_session, snapshot_from_state,
        McpUsePhase, QueuedControl,
    },
    McpSyncRegistry, SessionKey,
};
use crate::mcp_sync_server::types::{ForceDisconnectError, ForceDisconnectResult};

const MAX_CONTROL_ERROR_BYTES: usize = 4_096;
const CONTROL_POLL_TIMEOUT: Duration = Duration::from_secs(55);
const FORCE_DISCONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FORCE_DISCONNECT_TIMEOUT_MESSAGE: &str =
    "Timed out waiting for the Streamable HTTP MCP session to disconnect";

impl McpSyncRegistry {
    /// Ask every App-managed HTTP session using `connection_id` to release the
    /// exact generation it currently owns.
    ///
    /// The queued command remains generation-scoped after a timeout. A delayed
    /// command therefore cannot disconnect a later reconnect.
    pub async fn force_disconnect(
        &self,
        connection_id: &str,
    ) -> Result<ForceDisconnectResult, ForceDisconnectError> {
        validate_connection_id(connection_id).map_err(|error| ForceDisconnectError {
            requested: 0,
            completed: 0,
            error,
        })?;
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
                if let Entry::Vacant(entry) = state.controls.entry(command.command_id) {
                    entry.insert(QueuedControl {
                        owner: owner.clone(),
                        command: command.clone(),
                        was_connected,
                    });
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
        Err(ForceDisconnectError {
            requested,
            completed,
            error: errors.join("; "),
        })
    }

    pub(super) async fn poll_control(&self, context: McpSyncContext) -> McpSyncResponse {
        let session = (context.service_id, context.session_id);
        let notify = self.control_notify(session).await;
        let deadline = tokio::time::Instant::now() + CONTROL_POLL_TIMEOUT;
        loop {
            let control = {
                let mut state = self.inner.lock().await;
                if state.closed_services.contains(&context.service_id)
                    || state.closed_sessions.contains(&session)
                {
                    None
                } else {
                    Some(next_control_for_session(&mut state, session))
                }
            };
            let Some(control) = control else {
                self.remove_control_notify(session, &notify).await;
                return failure("MCP session is closed");
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
    pub(super) async fn control_result(
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

    pub(super) async fn close_session(&self, context: McpSyncContext) -> McpSyncResponse {
        let session = (context.service_id, context.session_id);
        let (snapshot, completed_controls) = {
            let mut state = self.inner.lock().await;
            if state.closed_services.contains(&context.service_id) {
                return failure("MCP synchronization service is closed");
            }
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
        self.close_session_notify(session).await;
        self.emit(snapshot).await;
        success(None, None)
    }

    pub(in crate::mcp_sync_server) async fn reset_service(&self, service_id: Uuid) {
        let (snapshot, completed_controls) = {
            let mut state = self.inner.lock().await;
            state.closed_services.insert(service_id);
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
            (snapshot, control_ids)
        };
        self.resolve_waiters(completed_controls, Ok(())).await;
        self.close_service_notifies(service_id).await;
        self.emit(snapshot).await;
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
        let closed = {
            let state = self.inner.lock().await;
            state.closed_services.contains(&session.0) || state.closed_sessions.contains(&session)
        };
        if closed {
            self.remove_control_notify(session, &notify).await;
        }
        Self::wake_control_pollers(&notify);
    }

    async fn close_session_notify(&self, session: SessionKey) {
        let notify = self.control_notifies.lock().await.remove(&session);
        if let Some(notify) = notify {
            Self::wake_control_pollers(&notify);
        }
    }

    async fn close_service_notifies(&self, service_id: Uuid) {
        let notifies = {
            let mut control_notifies = self.control_notifies.lock().await;
            let sessions = control_notifies
                .keys()
                .filter(|(owner_service, _)| *owner_service == service_id)
                .copied()
                .collect::<Vec<_>>();
            sessions
                .into_iter()
                .filter_map(|session| control_notifies.remove(&session))
                .collect::<Vec<_>>()
        };
        for notify in notifies {
            Self::wake_control_pollers(&notify);
        }
    }

    async fn remove_control_notify(&self, session: SessionKey, expected: &Arc<Notify>) {
        let mut notifies = self.control_notifies.lock().await;
        if notifies
            .get(&session)
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            notifies.remove(&session);
        }
    }

    fn wake_control_pollers(notify: &Notify) {
        // A stored permit covers the gap between the closed-state check and
        // parking the long poll on this already-removed notifier.
        notify.notify_waiters();
        notify.notify_one();
    }

    pub(super) async fn resolve_waiters(&self, command_ids: Vec<Uuid>, result: Result<(), String>) {
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
}
