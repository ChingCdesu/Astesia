use std::{path::PathBuf, sync::Arc};

use tokio::sync::mpsc;

pub enum SidecarRequest {
    Serve {
        http_port: u16,
        auth_token: String,
        sync_endpoint: String,
        sync_token: String,
        sync_service_id: String,
    },
    VerifySharedCredentials,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarInstallation {
    pub executable_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Error(String),
    Terminated {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

pub trait SidecarControl: Send + Sync {
    /// Concurrent and repeated calls are safe; success means the process cannot keep executing.
    fn terminate(&self) -> Result<(), String>;
}

pub type SidecarControlHandle = Arc<dyn SidecarControl>;

pub struct SpawnedSidecar {
    pub pid: u32,
    pub control: SidecarControlHandle,
    pub events: mpsc::Receiver<SidecarEvent>,
}

pub trait SidecarHost: Send + Sync {
    fn installation(&self) -> SidecarInstallation;
    fn spawn(&self, request: SidecarRequest) -> Result<SpawnedSidecar, String>;
}

pub type SidecarHostHandle = Arc<dyn SidecarHost>;
