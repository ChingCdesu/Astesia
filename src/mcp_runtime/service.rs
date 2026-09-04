use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::{oneshot, Mutex};

use crate::mcp_sync_server::{McpSyncRegistry, McpSyncServerHandle};
use crate::platform::{SidecarHostHandle, SidecarRequest, SpawnedSidecar};

use super::config::{endpoint, validate_auth_token, validate_port, ServiceConfig};
use super::monitor::spawn_event_monitor;
use super::state::{
    take_state, FailureOwnership, ManagedProcess, McpServicePhase, McpServiceStatus,
    MonitorOwnership, RuntimeState, ServiceResources,
};
use super::status::snapshot;
use super::termination::shutdown_sync_server;

const START_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(super) struct ManagedMcpService {
    pub(super) host: SidecarHostHandle,
    pub(super) state: Arc<Mutex<RuntimeState>>,
    pub(super) cleanup: Arc<Mutex<()>>,
    pub(super) binary_path: Option<PathBuf>,
    pub(super) registry: McpSyncRegistry,
}

impl ManagedMcpService {
    pub(super) fn new(host: SidecarHostHandle, registry: McpSyncRegistry) -> Self {
        let binary_path = host.installation().executable_path;
        Self {
            host,
            state: Arc::new(Mutex::new(RuntimeState::default())),
            cleanup: Arc::new(Mutex::new(())),
            binary_path,
            registry,
        }
    }

    pub(super) async fn status(&self) -> McpServiceStatus {
        let state = self.state.lock().await;
        snapshot(self.binary_path.as_ref(), &state)
    }

    pub(super) async fn start(
        &self,
        port: u16,
        auth_token: String,
    ) -> Result<McpServiceStatus, String> {
        validate_port(port)?;
        validate_auth_token(&auth_token)?;

        let config = ServiceConfig { port, auth_token };
        let expected_endpoint = endpoint(config.port);
        let cleanup_guard = self.cleanup.lock().await;
        let generation = {
            let mut state = self.state.lock().await;
            match &*state {
                RuntimeState::Preparing { .. }
                | RuntimeState::Starting { .. }
                | RuntimeState::Running { .. } => {
                    return Ok(snapshot(self.binary_path.as_ref(), &state));
                }
                RuntimeState::Stopping { .. } => {
                    return Err("MCP service is currently stopping".to_string());
                }
                RuntimeState::Failed {
                    ownership: FailureOwnership::Process(_),
                    error,
                    ..
                } => return Err(error.clone()),
                RuntimeState::Stopped { .. }
                | RuntimeState::Failed {
                    ownership: FailureOwnership::Clean,
                    ..
                } => {}
            }

            let generation = state.generation().wrapping_add(1);
            *state = RuntimeState::Preparing {
                generation,
                endpoint: expected_endpoint.clone(),
            };
            generation
        };

        let sync_server = match McpSyncServerHandle::start(self.registry.clone()).await {
            Ok(server) => server,
            Err(error) => {
                let message = format!("Unable to start MCP synchronization server: {error}");
                self.set_start_error(generation, message.clone()).await;
                return Err(message);
            }
        };
        let request = SidecarRequest::Serve {
            http_port: config.port,
            auth_token: config.auth_token,
            sync_endpoint: sync_server.endpoint().to_string(),
            sync_token: sync_server.token().to_string(),
            sync_service_id: sync_server.service_id().to_string(),
        };
        let SpawnedSidecar {
            events,
            control,
            pid,
        } = match self.host.spawn(request) {
            Ok(process) => process,
            Err(error) => {
                let message = format!("Unable to start MCP sidecar: {error}");
                self.set_start_error(generation, message.clone()).await;
                shutdown_sync_server(Some(sync_server), "after the MCP sidecar failed to start")
                    .await;
                return Err(message);
            }
        };

        let process = ManagedProcess {
            control,
            pid,
            monitor: MonitorOwnership::Active,
        };
        {
            let mut state = self.state.lock().await;
            let can_install = matches!(
                &*state,
                RuntimeState::Preparing {
                    generation: current,
                    ..
                } if *current == generation
            );
            if !can_install {
                drop(state);
                let _ = process.control.terminate();
                shutdown_sync_server(
                    Some(sync_server),
                    "after a concurrent MCP start cancellation",
                )
                .await;
                drop(cleanup_guard);
                return Ok(self.status().await);
            }
            *state = RuntimeState::Starting {
                generation,
                endpoint: expected_endpoint.clone(),
                resources: ServiceResources {
                    process,
                    sync_server,
                },
            };
        }

        let (ready_tx, ready_rx) = oneshot::channel();
        let service = self.clone();
        spawn_event_monitor(
            events,
            expected_endpoint,
            Some(ready_tx),
            move |message, unobserved| async move {
                service
                    .record_termination(generation, message, unobserved)
                    .await;
            },
        );
        drop(cleanup_guard);

        let readiness = match tokio::time::timeout(START_TIMEOUT, ready_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("MCP sidecar stopped before reporting readiness".to_string()),
            Err(_) => Err("MCP sidecar did not become ready within 5 seconds".to_string()),
        };
        match readiness {
            Ok(()) => {
                let mut state = self.state.lock().await;
                let previous = take_state(&mut state);
                *state = match previous {
                    RuntimeState::Starting {
                        generation: current,
                        endpoint,
                        resources,
                    } if current == generation => RuntimeState::Running {
                        generation,
                        endpoint,
                        started_at: Utc::now().to_rfc3339(),
                        resources,
                    },
                    other => other,
                };
                Ok(snapshot(self.binary_path.as_ref(), &state))
            }
            Err(error) => {
                if self.fail_and_terminate(generation, error.clone()).await {
                    Err(error)
                } else {
                    Ok(self.status().await)
                }
            }
        }
    }

    pub(super) async fn restart(
        &self,
        port: u16,
        auth_token: String,
    ) -> Result<McpServiceStatus, String> {
        validate_port(port)?;
        validate_auth_token(&auth_token)?;
        self.stop().await?;
        self.start(port, auth_token).await
    }

    async fn set_start_error(&self, generation: u64, error: String) {
        let mut state = self.state.lock().await;
        if state.generation() == generation && state.phase() == McpServicePhase::Starting {
            *state = RuntimeState::Failed {
                generation,
                endpoint: None,
                started_at: None,
                ownership: FailureOwnership::Clean,
                error,
            };
        }
    }
}
