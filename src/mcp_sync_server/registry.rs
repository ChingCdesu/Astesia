use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use tokio::sync::{oneshot, Mutex, Notify, OwnedMutexGuard};

mod control;
mod idempotency;
mod operations;
pub(super) mod protocol;
pub(super) mod state;

use state::{snapshot_from_state, OperationKey, RegistryState};
#[cfg(test)]
use state::{McpUsePhase, OwnershipKey, RegistryEntry};

use super::types::McpConnectionsSnapshot;
use uuid::Uuid;

use crate::{
    connection_repository::SharedConnectionRepository,
    platform::{UiEvent, UiEventSinkHandle},
};

pub(super) const MAX_RETAINED_KEYS: usize = 4_096;

type SessionKey = (Uuid, Uuid);
type LifecycleLocks = Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>;
type ControlNotifies = Arc<Mutex<HashMap<SessionKey, Arc<Notify>>>>;
type ControlWaiters = Arc<Mutex<HashMap<Uuid, Vec<oneshot::Sender<Result<(), String>>>>>>;

#[derive(Clone)]
pub struct McpSyncRegistry {
    pub(super) inner: Arc<Mutex<RegistryState>>,
    operation_locks: Arc<Mutex<HashMap<OperationKey, Weak<Mutex<()>>>>>,
    lifecycle_locks: LifecycleLocks,
    control_notifies: ControlNotifies,
    control_waiters: ControlWaiters,
    repository: Option<SharedConnectionRepository>,
    events: UiEventSinkHandle,
    #[cfg(test)]
    test_profiles: Arc<Mutex<HashMap<String, (i64, bool)>>>,
}

impl McpSyncRegistry {
    pub fn new(repository: SharedConnectionRepository, events: UiEventSinkHandle) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RegistryState::default())),
            operation_locks: Arc::new(Mutex::new(HashMap::new())),
            lifecycle_locks: Arc::new(Mutex::new(HashMap::new())),
            control_notifies: Arc::new(Mutex::new(HashMap::new())),
            control_waiters: Arc::new(Mutex::new(HashMap::new())),
            repository: Some(repository),
            events,
            #[cfg(test)]
            test_profiles: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    pub(super) fn without_app_events() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RegistryState::default())),
            operation_locks: Arc::new(Mutex::new(HashMap::new())),
            lifecycle_locks: Arc::new(Mutex::new(HashMap::new())),
            control_notifies: Arc::new(Mutex::new(HashMap::new())),
            control_waiters: Arc::new(Mutex::new(HashMap::new())),
            repository: None,
            events: Arc::new(crate::platform::UiEventBus::new()),
            test_profiles: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    pub(super) async fn allow_test_profile(
        &self,
        connection_id: &str,
        revision: i64,
        enabled: bool,
    ) {
        self.test_profiles
            .lock()
            .await
            .insert(connection_id.to_string(), (revision, enabled));
    }

    #[cfg(test)]
    pub(super) async fn retained_control_notifies(&self) -> usize {
        self.control_notifies.lock().await.len()
    }

    #[cfg(test)]
    pub(crate) async fn register_test_ownership(&self, connection_id: &str, profile_revision: i64) {
        let mut state = self.inner.lock().await;
        state.next_generation = state.next_generation.saturating_add(1).max(1);
        let generation = state.next_generation;
        state.entries.insert(
            OwnershipKey::new(Uuid::new_v4(), Uuid::new_v4(), connection_id.to_string()),
            RegistryEntry {
                profile_revision,
                generation,
                phase: McpUsePhase::Connected,
                last_error: None,
            },
        );
        state.revision = state.revision.saturating_add(1);
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
            if lifecycles.len() >= MAX_RETAINED_KEYS {
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
            Ok(())
        }

        #[cfg(not(test))]
        Err("Astesia App shared connection repository is unavailable".into())
    }

    async fn emit(&self, snapshot: Option<McpConnectionsSnapshot>) {
        let Some(snapshot) = snapshot else {
            return;
        };
        self.events.emit(UiEvent::McpConnectionsChanged(snapshot));
    }
}
