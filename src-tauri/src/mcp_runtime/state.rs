use serde::Serialize;

use crate::mcp_sync_server::McpSyncServerHandle;
use crate::platform::SidecarControlHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpServicePhase {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpServiceStatus {
    pub state: McpServicePhase,
    pub available: bool,
    pub pid: Option<u32>,
    pub endpoint: Option<String>,
    pub transport: &'static str,
    pub binary_path: Option<String>,
    pub version: Option<String>,
    pub started_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MonitorOwnership {
    Active,
    Detached,
}

#[derive(Clone)]
pub(super) struct ManagedProcess {
    pub(super) control: SidecarControlHandle,
    pub(super) pid: u32,
    pub(super) monitor: MonitorOwnership,
}

pub(super) struct ServiceResources {
    pub(super) process: ManagedProcess,
    pub(super) sync_server: McpSyncServerHandle,
}

pub(super) enum FailureOwnership {
    Clean,
    Process(ManagedProcess),
}

pub(super) enum RuntimeState {
    Stopped {
        generation: u64,
    },
    Preparing {
        generation: u64,
        endpoint: String,
    },
    Starting {
        generation: u64,
        endpoint: String,
        resources: ServiceResources,
    },
    Running {
        generation: u64,
        endpoint: String,
        started_at: String,
        resources: ServiceResources,
    },
    Stopping {
        generation: u64,
        endpoint: Option<String>,
        started_at: Option<String>,
        process: ManagedProcess,
    },
    Failed {
        generation: u64,
        endpoint: Option<String>,
        started_at: Option<String>,
        ownership: FailureOwnership,
        error: String,
    },
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self::Stopped { generation: 0 }
    }
}

impl RuntimeState {
    pub(super) fn generation(&self) -> u64 {
        match self {
            Self::Stopped { generation }
            | Self::Preparing { generation, .. }
            | Self::Starting { generation, .. }
            | Self::Running { generation, .. }
            | Self::Stopping { generation, .. }
            | Self::Failed { generation, .. } => *generation,
        }
    }

    pub(super) fn phase(&self) -> McpServicePhase {
        match self {
            Self::Stopped { .. } => McpServicePhase::Stopped,
            Self::Preparing { .. } | Self::Starting { .. } => McpServicePhase::Starting,
            Self::Running { .. } => McpServicePhase::Running,
            Self::Stopping { .. } => McpServicePhase::Stopping,
            Self::Failed { .. } => McpServicePhase::Error,
        }
    }

    pub(super) fn pid(&self) -> Option<u32> {
        match self {
            Self::Starting { resources, .. } | Self::Running { resources, .. } => {
                Some(resources.process.pid)
            }
            Self::Stopping { process, .. } => Some(process.pid),
            Self::Failed {
                ownership: FailureOwnership::Process(process),
                ..
            } => Some(process.pid),
            Self::Stopped { .. } | Self::Preparing { .. } | Self::Failed { .. } => None,
        }
    }

    pub(super) fn endpoint(&self) -> Option<&str> {
        match self {
            Self::Preparing { endpoint, .. }
            | Self::Starting { endpoint, .. }
            | Self::Running { endpoint, .. } => Some(endpoint),
            Self::Stopping { endpoint, .. } | Self::Failed { endpoint, .. } => endpoint.as_deref(),
            Self::Stopped { .. } => None,
        }
    }

    pub(super) fn started_at(&self) -> Option<&str> {
        match self {
            Self::Running { started_at, .. } => Some(started_at),
            Self::Stopping { started_at, .. } | Self::Failed { started_at, .. } => {
                started_at.as_deref()
            }
            Self::Stopped { .. } | Self::Preparing { .. } | Self::Starting { .. } => None,
        }
    }

    pub(super) fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }
}

pub(super) fn take_state(state: &mut RuntimeState) -> RuntimeState {
    let generation = state.generation();
    std::mem::replace(state, RuntimeState::Stopped { generation })
}
