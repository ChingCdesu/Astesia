use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::Hash,
};

use uuid::Uuid;

use crate::mcp_sync::{McpControlCommand, McpSyncResponse};

use super::SessionKey;
use crate::mcp_sync_server::types::{McpConnectionSnapshot, McpConnectionsSnapshot};

pub(in crate::mcp_sync_server) const MAX_CLOSED_TOMBSTONES: usize = super::MAX_RETAINED_KEYS;

pub(in crate::mcp_sync_server) struct BoundedTombstones<T> {
    entries: HashSet<T>,
    order: VecDeque<T>,
}

impl<T> Default for BoundedTombstones<T> {
    fn default() -> Self {
        Self {
            entries: HashSet::new(),
            order: VecDeque::new(),
        }
    }
}

impl<T> BoundedTombstones<T>
where
    T: Copy + Eq + Hash,
{
    pub(in crate::mcp_sync_server) fn contains(&self, value: &T) -> bool {
        self.entries.contains(value)
    }

    pub(in crate::mcp_sync_server) fn insert(&mut self, value: T) {
        if !self.entries.insert(value) {
            self.order.retain(|existing| *existing != value);
        }
        self.order.push_back(value);
        while self.order.len() > MAX_CLOSED_TOMBSTONES {
            if let Some(expired) = self.order.pop_front() {
                self.entries.remove(&expired);
            }
        }
    }

    pub(in crate::mcp_sync_server) fn retain(&mut self, keep: impl Fn(&T) -> bool) {
        self.entries.retain(|value| keep(value));
        self.order.retain(|value| self.entries.contains(value));
    }

    #[cfg(test)]
    pub(in crate::mcp_sync_server) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(in crate::mcp_sync_server) struct OwnershipKey {
    pub(in crate::mcp_sync_server) service_id: Uuid,
    pub(in crate::mcp_sync_server) session_id: Uuid,
    pub(in crate::mcp_sync_server) connection_id: String,
}

impl OwnershipKey {
    pub(in crate::mcp_sync_server) fn new(
        service_id: Uuid,
        session_id: Uuid,
        connection_id: String,
    ) -> Self {
        Self {
            service_id,
            session_id,
            connection_id,
        }
    }

    pub(in crate::mcp_sync_server) fn session_key(&self) -> SessionKey {
        (self.service_id, self.session_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mcp_sync_server) enum McpUsePhase {
    Acquired,
    Connected,
    Disconnecting { was_connected: bool },
}

impl McpUsePhase {
    pub(in crate::mcp_sync_server) fn is_connected(self) -> bool {
        matches!(
            self,
            Self::Connected
                | Self::Disconnecting {
                    was_connected: true
                }
        )
    }

    pub(in crate::mcp_sync_server) fn is_disconnecting(self) -> bool {
        matches!(self, Self::Disconnecting { .. })
    }
}

#[derive(Debug, Clone)]
pub(in crate::mcp_sync_server) struct RegistryEntry {
    pub(in crate::mcp_sync_server) profile_revision: i64,
    pub(in crate::mcp_sync_server) generation: u64,
    pub(in crate::mcp_sync_server) phase: McpUsePhase,
    pub(in crate::mcp_sync_server) last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(in crate::mcp_sync_server) struct QueuedControl {
    pub(in crate::mcp_sync_server) owner: OwnershipKey,
    pub(in crate::mcp_sync_server) command: McpControlCommand,
    pub(in crate::mcp_sync_server) was_connected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::mcp_sync_server) struct OperationKey {
    pub(in crate::mcp_sync_server) service_id: Uuid,
    pub(in crate::mcp_sync_server) session_id: Uuid,
    pub(in crate::mcp_sync_server) operation_id: Uuid,
}

#[derive(Default)]
pub(in crate::mcp_sync_server) struct RegistryState {
    pub(in crate::mcp_sync_server) revision: u64,
    pub(in crate::mcp_sync_server) next_generation: u64,
    pub(in crate::mcp_sync_server) entries: HashMap<OwnershipKey, RegistryEntry>,
    pub(in crate::mcp_sync_server) controls: HashMap<Uuid, QueuedControl>,
    pub(in crate::mcp_sync_server) control_queues: HashMap<SessionKey, VecDeque<Uuid>>,
    pub(in crate::mcp_sync_server) closed_sessions: BoundedTombstones<SessionKey>,
    pub(in crate::mcp_sync_server) closed_services: BoundedTombstones<Uuid>,
    pub(in crate::mcp_sync_server) completed_operations: HashMap<OperationKey, McpSyncResponse>,
    pub(in crate::mcp_sync_server) operation_order: VecDeque<OperationKey>,
}

pub(in crate::mcp_sync_server) fn next_control_for_session(
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

pub(in crate::mcp_sync_server) fn remove_control(
    state: &mut RegistryState,
    command_id: Uuid,
) -> Option<QueuedControl> {
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

pub(in crate::mcp_sync_server) fn remove_controls_for_owner(
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

pub(in crate::mcp_sync_server) fn remove_controls_for_session(
    state: &mut RegistryState,
    session: SessionKey,
) -> Vec<Uuid> {
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

pub(in crate::mcp_sync_server) fn snapshot_from_state(
    state: &RegistryState,
) -> McpConnectionsSnapshot {
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

pub(in crate::mcp_sync_server) fn ensure_owner_is_open(
    state: &RegistryState,
    key: &OwnershipKey,
) -> Option<String> {
    if state.closed_services.contains(&key.service_id) {
        Some("MCP synchronization service is closed".into())
    } else if state.closed_sessions.contains(&key.session_key()) {
        Some("MCP session is closed".into())
    } else {
        None
    }
}
