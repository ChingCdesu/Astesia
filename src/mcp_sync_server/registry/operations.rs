use crate::mcp_sync::{McpSyncContext, McpSyncResponse};

use super::{
    protocol::{failure, success, validate_connection_id},
    state::{
        ensure_owner_is_open, remove_controls_for_owner, snapshot_from_state, McpUsePhase,
        OwnershipKey, RegistryEntry,
    },
    McpSyncRegistry,
};

impl McpSyncRegistry {
    pub(super) async fn acquire(
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

    pub(super) async fn connected(
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

    pub(super) async fn released(
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
}
