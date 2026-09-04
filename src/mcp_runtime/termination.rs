use std::time::Duration;

use crate::mcp_sync_server::McpSyncServerHandle;

use super::service::ManagedMcpService;
use super::state::{
    take_state, FailureOwnership, ManagedProcess, McpServicePhase, McpServiceStatus,
    MonitorOwnership, RuntimeState,
};
use super::status::snapshot;

const STOP_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(25);

enum StopPlan {
    Complete(Result<McpServiceStatus, String>),
    Wait {
        process: ManagedProcess,
        sync_server: Option<McpSyncServerHandle>,
        generation: u64,
    },
}

impl ManagedMcpService {
    pub(super) async fn stop(&self) -> Result<McpServiceStatus, String> {
        self.stop_with_timeout(STOP_TIMEOUT).await
    }

    async fn stop_with_timeout(&self, stop_timeout: Duration) -> Result<McpServiceStatus, String> {
        let cleanup_guard = self.cleanup.lock().await;
        let plan = {
            let mut state = self.state.lock().await;
            let previous = take_state(&mut state);
            match previous {
                RuntimeState::Stopped { generation } => {
                    *state = RuntimeState::Stopped { generation };
                    StopPlan::Complete(Ok(snapshot(self.binary_path.as_ref(), &state)))
                }
                RuntimeState::Preparing { generation, .. } => {
                    *state = RuntimeState::Stopped {
                        generation: generation.wrapping_add(1),
                    };
                    StopPlan::Complete(Ok(snapshot(self.binary_path.as_ref(), &state)))
                }
                RuntimeState::Starting {
                    generation,
                    endpoint,
                    resources,
                } => {
                    let process = resources.process;
                    *state = RuntimeState::Stopping {
                        generation,
                        endpoint: Some(endpoint),
                        started_at: None,
                        process: process.clone(),
                    };
                    StopPlan::Wait {
                        process,
                        sync_server: Some(resources.sync_server),
                        generation,
                    }
                }
                RuntimeState::Running {
                    generation,
                    endpoint,
                    started_at,
                    resources,
                } => {
                    let process = resources.process;
                    *state = RuntimeState::Stopping {
                        generation,
                        endpoint: Some(endpoint),
                        started_at: Some(started_at),
                        process: process.clone(),
                    };
                    StopPlan::Wait {
                        process,
                        sync_server: Some(resources.sync_server),
                        generation,
                    }
                }
                RuntimeState::Stopping {
                    generation,
                    endpoint,
                    started_at,
                    process,
                } => {
                    *state = RuntimeState::Stopping {
                        generation,
                        endpoint,
                        started_at,
                        process: process.clone(),
                    };
                    StopPlan::Wait {
                        process,
                        sync_server: None,
                        generation,
                    }
                }
                RuntimeState::Failed {
                    generation,
                    ownership: FailureOwnership::Clean,
                    ..
                } => {
                    *state = RuntimeState::Stopped { generation };
                    StopPlan::Complete(Ok(snapshot(self.binary_path.as_ref(), &state)))
                }
                RuntimeState::Failed {
                    generation,
                    endpoint,
                    started_at,
                    ownership: FailureOwnership::Process(process),
                    ..
                } => {
                    *state = RuntimeState::Stopping {
                        generation,
                        endpoint,
                        started_at,
                        process: process.clone(),
                    };
                    StopPlan::Wait {
                        process,
                        sync_server: None,
                        generation,
                    }
                }
            }
        };

        let (process, sync_server, generation) = match plan {
            StopPlan::Complete(result) => {
                drop(cleanup_guard);
                return result;
            }
            StopPlan::Wait {
                process,
                sync_server,
                generation,
            } => (process, sync_server, generation),
        };

        let termination_error = process
            .control
            .terminate()
            .err()
            .map(|error| format!("Unable to signal MCP sidecar: {error}"));
        let sync_error = shutdown_sync_server(sync_server, "while stopping the MCP sidecar").await;
        if process.monitor == MonitorOwnership::Detached {
            let result = self
                .finish_detached_stop(generation, process, termination_error, sync_error)
                .await;
            drop(cleanup_guard);
            return result;
        }

        drop(cleanup_guard);
        let deadline = tokio::time::Instant::now() + stop_timeout;
        loop {
            {
                let state = self.state.lock().await;
                if state.generation() != generation || state.phase() == McpServicePhase::Stopped {
                    return sync_error
                        .map_or_else(|| Ok(snapshot(self.binary_path.as_ref(), &state)), Err);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(STOP_POLL_INTERVAL).await;
        }

        let mut state = self.state.lock().await;
        if state.generation() != generation || state.phase() == McpServicePhase::Stopped {
            return sync_error.map_or_else(|| Ok(snapshot(self.binary_path.as_ref(), &state)), Err);
        }
        let message = match (sync_error, termination_error) {
            (Some(sync_error), termination_error) => combine_errors(sync_error, termination_error),
            (None, Some(termination_error)) => termination_error,
            (None, None) => {
                "MCP sidecar did not terminate within 2 seconds; its state is unknown".to_string()
            }
        };
        let previous = take_state(&mut state);
        *state = match previous {
            RuntimeState::Stopping {
                generation,
                endpoint,
                started_at,
                process,
            } => RuntimeState::Failed {
                generation,
                endpoint,
                started_at,
                ownership: FailureOwnership::Process(process),
                error: message.clone(),
            },
            other => other,
        };
        Err(message)
    }

    async fn finish_detached_stop(
        &self,
        generation: u64,
        process: ManagedProcess,
        termination_error: Option<String>,
        sync_error: Option<String>,
    ) -> Result<McpServiceStatus, String> {
        let mut state = self.state.lock().await;
        if state.generation() != generation || state.phase() == McpServicePhase::Stopped {
            return sync_error.map_or_else(|| Ok(snapshot(self.binary_path.as_ref(), &state)), Err);
        }
        let previous = take_state(&mut state);
        let (endpoint, started_at) = match previous {
            RuntimeState::Stopping {
                endpoint,
                started_at,
                ..
            } => (endpoint, started_at),
            other => {
                *state = other;
                return sync_error
                    .map_or_else(|| Ok(snapshot(self.binary_path.as_ref(), &state)), Err);
            }
        };
        if let Some(termination_error) = termination_error {
            let error = combine_errors(termination_error, sync_error);
            *state = RuntimeState::Failed {
                generation,
                endpoint,
                started_at,
                ownership: FailureOwnership::Process(process),
                error: error.clone(),
            };
            return Err(error);
        }
        if let Some(sync_error) = sync_error {
            *state = RuntimeState::Failed {
                generation,
                endpoint: None,
                started_at: None,
                ownership: FailureOwnership::Clean,
                error: sync_error.clone(),
            };
            return Err(sync_error);
        }
        *state = RuntimeState::Stopped { generation };
        Ok(snapshot(self.binary_path.as_ref(), &state))
    }

    pub(super) async fn fail_and_terminate(&self, generation: u64, error: String) -> bool {
        let cleanup_guard = self.cleanup.lock().await;
        let (failed_generation, process, sync_server) = {
            let mut state = self.state.lock().await;
            let previous = take_state(&mut state);
            match previous {
                RuntimeState::Starting {
                    generation: current,
                    resources,
                    ..
                } if current == generation => {
                    let failed_generation = generation.wrapping_add(1);
                    *state = RuntimeState::Failed {
                        generation: failed_generation,
                        endpoint: None,
                        started_at: None,
                        ownership: FailureOwnership::Clean,
                        error,
                    };
                    (failed_generation, resources.process, resources.sync_server)
                }
                other => {
                    *state = other;
                    return false;
                }
            }
        };

        let termination_error =
            process.control.terminate().err().map(|error| {
                format!("Unable to terminate MCP sidecar after startup failed: {error}")
            });
        let sync_error =
            shutdown_sync_server(Some(sync_server), "after MCP sidecar startup failed").await;
        let mut state = self.state.lock().await;
        if state.generation() == failed_generation {
            let previous = take_state(&mut state);
            if let RuntimeState::Failed {
                generation,
                endpoint,
                started_at,
                mut ownership,
                mut error,
            } = previous
            {
                if let Some(termination_error) = termination_error {
                    ownership = FailureOwnership::Process(ManagedProcess {
                        monitor: MonitorOwnership::Detached,
                        ..process
                    });
                    append_error(&mut error, termination_error);
                }
                if let Some(sync_error) = sync_error {
                    append_error(&mut error, sync_error);
                }
                *state = RuntimeState::Failed {
                    generation,
                    endpoint,
                    started_at,
                    ownership,
                    error,
                };
            } else {
                *state = previous;
            }
        }
        drop(cleanup_guard);
        true
    }

    pub(super) async fn record_termination(
        &self,
        generation: u64,
        error: String,
        terminate_unobserved: bool,
    ) {
        let cleanup_guard = self.cleanup.lock().await;
        let (mut process, sync_server, was_stopping) = {
            let mut state = self.state.lock().await;
            if state.generation() != generation {
                return;
            }
            let previous = take_state(&mut state);
            match previous {
                RuntimeState::Starting { resources, .. }
                | RuntimeState::Running { resources, .. } => {
                    (resources.process, Some(resources.sync_server), false)
                }
                RuntimeState::Stopping { process, .. } => (process, None, true),
                RuntimeState::Failed {
                    ownership: FailureOwnership::Process(process),
                    ..
                } if process.monitor == MonitorOwnership::Active => (process, None, false),
                other => {
                    *state = other;
                    return;
                }
            }
        };
        process.monitor = MonitorOwnership::Detached;
        let termination_error = terminate_unobserved
            .then(|| process.control.terminate().err())
            .flatten();
        let sync_error =
            shutdown_sync_server(sync_server, "after the MCP sidecar terminated").await;

        let mut state = self.state.lock().await;
        if state.generation() != generation {
            return;
        }
        if let Some(termination_error) = termination_error {
            *state = RuntimeState::Failed {
                generation,
                endpoint: None,
                started_at: None,
                ownership: FailureOwnership::Process(process),
                error: format!(
                    "{error}; unable to terminate the unobserved MCP sidecar: {termination_error}"
                ),
            };
        } else if was_stopping {
            *state = RuntimeState::Stopped { generation };
        } else {
            *state = RuntimeState::Failed {
                generation,
                endpoint: None,
                started_at: None,
                ownership: FailureOwnership::Clean,
                error,
            };
        }
        if let Some(sync_error) = sync_error {
            let previous = take_state(&mut state);
            *state = match previous {
                RuntimeState::Failed {
                    generation,
                    endpoint,
                    started_at,
                    ownership,
                    mut error,
                } => {
                    append_error(&mut error, sync_error);
                    RuntimeState::Failed {
                        generation,
                        endpoint,
                        started_at,
                        ownership,
                        error,
                    }
                }
                RuntimeState::Stopped { generation } => RuntimeState::Failed {
                    generation,
                    endpoint: None,
                    started_at: None,
                    ownership: FailureOwnership::Clean,
                    error: sync_error,
                },
                other => other,
            };
        }
        drop(cleanup_guard);
    }
}

pub(super) async fn shutdown_sync_server(
    sync_server: Option<McpSyncServerHandle>,
    context: &str,
) -> Option<String> {
    let sync_server = sync_server?;
    match sync_server.shutdown().await {
        Ok(()) => None,
        Err(error) => {
            let message = format!("Unable to stop MCP synchronization server {context}: {error}");
            log::warn!("{message}");
            Some(message)
        }
    }
}

fn append_error(target: &mut String, error: String) {
    if !target.is_empty() {
        target.push_str("; ");
    }
    target.push_str(&error);
}

fn combine_errors(mut primary: String, secondary: Option<String>) -> String {
    if let Some(secondary) = secondary {
        primary.push_str("; ");
        primary.push_str(&secondary);
    }
    primary
}

#[cfg(test)]
impl ManagedMcpService {
    pub(super) async fn seed_failed_process(
        &self,
        control: crate::platform::SidecarControlHandle,
        generation: u64,
        monitor: bool,
    ) {
        use super::config::endpoint;

        *self.state.lock().await = RuntimeState::Failed {
            generation,
            endpoint: Some(endpoint(24872)),
            started_at: None,
            ownership: FailureOwnership::Process(ManagedProcess {
                control,
                pid: 42,
                monitor: if monitor {
                    MonitorOwnership::Active
                } else {
                    MonitorOwnership::Detached
                },
            }),
            error: "seeded process".to_string(),
        };
    }

    pub(super) async fn seed_stopping(
        &self,
        control: crate::platform::SidecarControlHandle,
        generation: u64,
    ) {
        use chrono::Utc;

        use super::config::endpoint;

        *self.state.lock().await = RuntimeState::Stopping {
            generation,
            endpoint: Some(endpoint(24872)),
            started_at: Some(Utc::now().to_rfc3339()),
            process: ManagedProcess {
                control,
                pid: 42,
                monitor: MonitorOwnership::Active,
            },
        };
    }

    pub(super) async fn fail_test_start(&self, generation: u64, error: &str) -> bool {
        self.fail_and_terminate(generation, error.to_string()).await
    }

    pub(super) async fn record_test_termination(
        &self,
        generation: u64,
        error: &str,
        unobserved: bool,
    ) {
        self.record_termination(generation, error.to_string(), unobserved)
            .await;
    }

    pub(super) async fn stop_with_test_timeout(
        &self,
        timeout: Duration,
    ) -> Result<McpServiceStatus, String> {
        self.stop_with_timeout(timeout).await
    }
}
