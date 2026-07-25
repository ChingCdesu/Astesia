use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::sync::{oneshot, Mutex, OwnedMutexGuard};

use crate::mcp_sync::{MCP_AUTH_TOKEN_ENV, SYNC_ENDPOINT_ENV, SYNC_SERVICE_ID_ENV, SYNC_TOKEN_ENV};
use crate::mcp_sync_server::{
    ForceDisconnectResult, McpConnectionsSnapshot, McpSyncRegistry, McpSyncServerHandle,
};

const SIDECAR_NAME: &str = "astesia-mcp";
const READY_PREFIX: &str = "ASTESIA_MCP_READY";
const START_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(25);
const TRANSPORT: &str = "streamable_http";

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

struct ServiceConfig {
    port: u16,
    auth_token: String,
}

#[derive(Debug)]
struct RuntimeState {
    phase: McpServicePhase,
    child: Option<CommandChild>,
    sync_server: Option<McpSyncServerHandle>,
    pid: Option<u32>,
    endpoint: Option<String>,
    started_at: Option<String>,
    last_error: Option<String>,
    generation: u64,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            phase: McpServicePhase::Stopped,
            child: None,
            sync_server: None,
            pid: None,
            endpoint: None,
            started_at: None,
            last_error: None,
            generation: 0,
        }
    }
}

fn transition_failed_start(
    inner: &mut RuntimeState,
    generation: u64,
    error: String,
) -> Option<(Option<CommandChild>, Option<McpSyncServerHandle>)> {
    if inner.generation != generation || inner.phase != McpServicePhase::Starting {
        return None;
    }

    // Invalidate the monitor before killing so its termination event cannot
    // replace the more useful startup error.
    inner.generation = inner.generation.wrapping_add(1);
    inner.phase = McpServicePhase::Error;
    inner.pid = None;
    inner.endpoint = None;
    inner.started_at = None;
    inner.last_error = Some(error);
    Some((inner.child.take(), inner.sync_server.take()))
}

#[derive(Clone)]
pub struct McpHelperState {
    app_handle: AppHandle,
    inner: Arc<Mutex<RuntimeState>>,
    sync_cleanup: Arc<Mutex<()>>,
    binary_path: Option<PathBuf>,
    sync_registry: McpSyncRegistry,
}

impl McpHelperState {
    pub fn new(app_handle: AppHandle) -> Self {
        let sync_registry = McpSyncRegistry::new(app_handle.clone());
        Self {
            app_handle,
            inner: Arc::new(Mutex::new(RuntimeState::default())),
            sync_cleanup: Arc::new(Mutex::new(())),
            binary_path: resolve_sidecar_path(),
            sync_registry,
        }
    }

    pub async fn status(&self) -> McpServiceStatus {
        let inner = self.inner.lock().await;
        self.snapshot(&inner)
    }

    pub async fn synced_connections(&self) -> McpConnectionsSnapshot {
        self.sync_registry.snapshot().await
    }

    pub async fn lock_connection_lifecycle(&self, connection_id: &str) -> OwnedMutexGuard<()> {
        self.sync_registry
            .lock_connection_lifecycle(connection_id)
            .await
    }

    pub async fn is_connection_in_use(&self, connection_id: &str) -> bool {
        self.sync_registry.is_connection_in_use(connection_id).await
    }

    pub async fn force_disconnect(
        &self,
        connection_id: &str,
    ) -> Result<ForceDisconnectResult, String> {
        self.sync_registry.force_disconnect(connection_id).await
    }

    pub async fn start(&self, port: u16, auth_token: String) -> Result<McpServiceStatus, String> {
        validate_port(port)?;
        validate_auth_token(&auth_token)?;

        let config = ServiceConfig { port, auth_token };
        let expected_endpoint = endpoint(config.port);
        // Serialize setup with cleanup so a concurrent stop cannot return before
        // resources created by this start attempt have an owner.
        let cleanup_guard = self.sync_cleanup.lock().await;
        let (generation, stale_sync_server) = {
            let mut inner = self.inner.lock().await;
            match inner.phase {
                McpServicePhase::Starting | McpServicePhase::Running => {
                    return Ok(self.snapshot(&inner));
                }
                McpServicePhase::Stopping => {
                    return Err("MCP service is currently stopping".to_string());
                }
                McpServicePhase::Error if inner.child.is_some() || inner.pid.is_some() => {
                    return Err(inner.last_error.clone().unwrap_or_else(|| {
                        "Stop the previous MCP sidecar before starting it again".to_string()
                    }));
                }
                McpServicePhase::Stopped | McpServicePhase::Error => {}
            }

            inner.generation = inner.generation.wrapping_add(1);
            inner.phase = McpServicePhase::Starting;
            inner.child = None;
            inner.pid = None;
            inner.endpoint = Some(expected_endpoint.clone());
            inner.started_at = None;
            inner.last_error = None;
            (inner.generation, inner.sync_server.take())
        };

        if let Some(error) =
            shutdown_sync_server(stale_sync_server, "before starting a new MCP sidecar").await
        {
            self.set_start_error(generation, error.clone()).await;
            return Err(error);
        }

        let sync_server = match McpSyncServerHandle::start(self.sync_registry.clone()).await {
            Ok(server) => server,
            Err(error) => {
                let message = format!("Unable to start MCP synchronization server: {error}");
                self.set_start_error(generation, message.clone()).await;
                return Err(message);
            }
        };
        let sync_endpoint = sync_server.endpoint().to_string();
        let sync_token = sync_server.token().to_string();
        let sync_service_id = sync_server.service_id().to_string();

        let command = match self.app_handle.shell().sidecar(SIDECAR_NAME) {
            Ok(command) => command,
            Err(error) => {
                let message = format!("Unable to resolve MCP sidecar: {error}");
                self.set_start_error(generation, message.clone()).await;
                shutdown_sync_server(
                    Some(sync_server),
                    "after the MCP sidecar could not be resolved",
                )
                .await;
                return Err(message);
            }
        };

        let (receiver, child) = match command
            .args(["--http-port", &config.port.to_string()])
            .env(MCP_AUTH_TOKEN_ENV, &config.auth_token)
            .env(SYNC_ENDPOINT_ENV, &sync_endpoint)
            .env(SYNC_TOKEN_ENV, &sync_token)
            .env(SYNC_SERVICE_ID_ENV, &sync_service_id)
            .spawn()
        {
            Ok(process) => process,
            Err(error) => {
                let message = format!("Unable to start MCP sidecar: {error}");
                self.set_start_error(generation, message.clone()).await;
                shutdown_sync_server(Some(sync_server), "after the MCP sidecar failed to start")
                    .await;
                return Err(message);
            }
        };

        let pid = child.pid();
        {
            let mut inner = self.inner.lock().await;
            if inner.generation != generation || inner.phase != McpServicePhase::Starting {
                drop(inner);
                let _ = child.kill();
                shutdown_sync_server(
                    Some(sync_server),
                    "after a concurrent MCP start cancellation",
                )
                .await;
                drop(cleanup_guard);
                return Ok(self.status().await);
            }
            inner.pid = Some(pid);
            inner.child = Some(child);
            inner.sync_server = Some(sync_server);
        }

        let (ready_tx, ready_rx) = oneshot::channel();
        spawn_event_monitor(
            Arc::clone(&self.inner),
            Arc::clone(&self.sync_cleanup),
            receiver,
            generation,
            expected_endpoint,
            Some(ready_tx),
        );
        drop(cleanup_guard);

        match tokio::time::timeout(START_TIMEOUT, ready_rx).await {
            Ok(Ok(Ok(()))) => {
                let mut inner = self.inner.lock().await;
                if inner.generation == generation && inner.phase == McpServicePhase::Starting {
                    inner.phase = McpServicePhase::Running;
                    inner.started_at = Some(Utc::now().to_rfc3339());
                }
                Ok(self.snapshot(&inner))
            }
            Ok(Ok(Err(error))) => {
                if self.fail_and_kill(generation, error.clone()).await {
                    Err(error)
                } else {
                    Ok(self.status().await)
                }
            }
            Ok(Err(_)) => {
                let error = "MCP sidecar stopped before reporting readiness".to_string();
                if self.fail_and_kill(generation, error.clone()).await {
                    Err(error)
                } else {
                    Ok(self.status().await)
                }
            }
            Err(_) => {
                let error = "MCP sidecar did not become ready within 5 seconds".to_string();
                if self.fail_and_kill(generation, error.clone()).await {
                    Err(error)
                } else {
                    Ok(self.status().await)
                }
            }
        }
    }

    pub async fn stop(&self) -> Result<McpServiceStatus, String> {
        enum StopAction {
            Immediate {
                result: Result<McpServiceStatus, String>,
                sync_server: Option<McpSyncServerHandle>,
            },
            Wait {
                child: Option<CommandChild>,
                sync_server: Option<McpSyncServerHandle>,
                generation: u64,
            },
        }

        // Cleanup owns this gate before it removes handles from RuntimeState.
        // This makes concurrent stop/shutdown callers wait for registry reset.
        let cleanup_guard = self.sync_cleanup.lock().await;
        let action = {
            let mut inner = self.inner.lock().await;
            match inner.phase {
                McpServicePhase::Stopped => StopAction::Immediate {
                    result: Ok(self.snapshot(&inner)),
                    sync_server: inner.sync_server.take(),
                },
                McpServicePhase::Stopping => StopAction::Wait {
                    child: None,
                    sync_server: inner.sync_server.take(),
                    generation: inner.generation,
                },
                McpServicePhase::Error if inner.child.is_none() => {
                    if inner.pid.is_some() {
                        StopAction::Immediate {
                            result: Err(inner
                                .last_error
                                .clone()
                                .unwrap_or_else(|| "MCP sidecar state is unknown".to_string())),
                            sync_server: inner.sync_server.take(),
                        }
                    } else {
                        inner.phase = McpServicePhase::Stopped;
                        inner.pid = None;
                        inner.endpoint = None;
                        inner.started_at = None;
                        inner.last_error = None;
                        StopAction::Immediate {
                            result: Ok(self.snapshot(&inner)),
                            sync_server: inner.sync_server.take(),
                        }
                    }
                }
                McpServicePhase::Starting if inner.child.is_none() => {
                    // Invalidate a concurrent start before it installs its child.
                    inner.generation = inner.generation.wrapping_add(1);
                    inner.phase = McpServicePhase::Stopped;
                    inner.pid = None;
                    inner.endpoint = None;
                    inner.started_at = None;
                    inner.last_error = None;
                    StopAction::Immediate {
                        result: Ok(self.snapshot(&inner)),
                        sync_server: inner.sync_server.take(),
                    }
                }
                McpServicePhase::Starting | McpServicePhase::Running | McpServicePhase::Error => {
                    inner.phase = McpServicePhase::Stopping;
                    StopAction::Wait {
                        child: inner.child.take(),
                        sync_server: inner.sync_server.take(),
                        generation: inner.generation,
                    }
                }
            }
        };

        let (child, sync_server, generation) = match action {
            StopAction::Immediate {
                result,
                sync_server,
            } => {
                let sync_error =
                    shutdown_sync_server(sync_server, "while stopping the MCP sidecar").await;
                drop(cleanup_guard);
                return sync_error.map_or(result, Err);
            }
            StopAction::Wait {
                child,
                sync_server,
                generation,
            } => (child, sync_server, generation),
        };

        let kill_error = child.and_then(|child| {
            child
                .kill()
                .err()
                .map(|error| format!("Unable to signal MCP sidecar: {error}"))
        });
        let sync_error = shutdown_sync_server(sync_server, "while stopping the MCP sidecar").await;
        drop(cleanup_guard);
        let deadline = tokio::time::Instant::now() + STOP_TIMEOUT;

        loop {
            {
                let inner = self.inner.lock().await;
                if inner.generation != generation || inner.phase == McpServicePhase::Stopped {
                    return sync_error.map_or_else(|| Ok(self.snapshot(&inner)), Err);
                }
            }

            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(STOP_POLL_INTERVAL).await;
        }

        let mut inner = self.inner.lock().await;
        if inner.generation != generation || inner.phase == McpServicePhase::Stopped {
            return sync_error.map_or_else(|| Ok(self.snapshot(&inner)), Err);
        }

        let message = sync_error.or(kill_error).unwrap_or_else(|| {
            "MCP sidecar did not terminate within 2 seconds; its state is unknown".to_string()
        });
        inner.phase = McpServicePhase::Error;
        inner.last_error = Some(message.clone());
        Err(message)
    }

    pub async fn restart(&self, port: u16, auth_token: String) -> Result<McpServiceStatus, String> {
        validate_port(port)?;
        validate_auth_token(&auth_token)?;
        self.stop().await?;
        self.start(port, auth_token).await
    }

    pub async fn shutdown(&self) {
        if let Err(error) = self.stop().await {
            log::warn!("Unable to stop MCP sidecar during shutdown: {error}");
        }
    }

    fn snapshot(&self, inner: &RuntimeState) -> McpServiceStatus {
        let available = self.binary_path.as_ref().is_some_and(|path| path.is_file());

        McpServiceStatus {
            state: inner.phase,
            available,
            pid: inner.pid,
            endpoint: inner.endpoint.clone(),
            transport: TRANSPORT,
            binary_path: self
                .binary_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            version: available.then(|| env!("CARGO_PKG_VERSION").to_string()),
            started_at: inner.started_at.clone(),
            last_error: inner.last_error.clone(),
        }
    }

    async fn set_start_error(&self, generation: u64, error: String) -> bool {
        let mut inner = self.inner.lock().await;
        if inner.generation == generation {
            inner.phase = McpServicePhase::Error;
            inner.child = None;
            inner.pid = None;
            inner.endpoint = None;
            inner.started_at = None;
            inner.last_error = Some(error);
            true
        } else {
            false
        }
    }

    async fn fail_and_kill(&self, generation: u64, error: String) -> bool {
        let cleanup_guard = self.sync_cleanup.lock().await;
        let (child, sync_server) = {
            let mut inner = self.inner.lock().await;
            let Some(resources) = transition_failed_start(&mut inner, generation, error) else {
                return false;
            };
            resources
        };

        if let Some(child) = child {
            let _ = child.kill();
        }
        shutdown_sync_server(sync_server, "after MCP sidecar startup failed").await;
        drop(cleanup_guard);
        true
    }
}

fn spawn_event_monitor(
    inner: Arc<Mutex<RuntimeState>>,
    sync_cleanup: Arc<Mutex<()>>,
    mut receiver: tauri::async_runtime::Receiver<CommandEvent>,
    generation: u64,
    expected_endpoint: String,
    ready_sender: Option<oneshot::Sender<Result<(), String>>>,
) {
    tauri::async_runtime::spawn(async move {
        let mut ready_sender = ready_sender;
        let mut last_stderr = None;

        while let Some(event) = receiver.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    log_output("stdout", &bytes);
                }
                CommandEvent::Stderr(bytes) => {
                    let output = String::from_utf8_lossy(&bytes);
                    for line in output.lines().map(|line| line.trim_end_matches('\r')) {
                        if is_ready_line(line, &expected_endpoint) {
                            if let Some(sender) = ready_sender.take() {
                                let _ = sender.send(Ok(()));
                            }
                        } else {
                            if !line.is_empty() {
                                last_stderr = Some(line.to_string());
                            }
                            log::debug!("MCP sidecar stderr: {line}");
                        }
                    }
                }
                CommandEvent::Error(error) => {
                    if let Some(sender) = ready_sender.take() {
                        let _ = sender.send(Err(format!("MCP sidecar output error: {error}")));
                    }
                    log::warn!("MCP sidecar output error: {error}");
                }
                CommandEvent::Terminated(payload) => {
                    let mut message = match (payload.code, payload.signal) {
                        (Some(code), _) => format!("MCP sidecar exited with code {code}"),
                        (None, Some(signal)) => {
                            format!("MCP sidecar terminated by signal {signal}")
                        }
                        (None, None) => "MCP sidecar terminated unexpectedly".to_string(),
                    };
                    if let Some(detail) = last_stderr.as_deref() {
                        message.push_str(": ");
                        message.push_str(detail);
                    }
                    if let Some(sender) = ready_sender.take() {
                        let _ = sender.send(Err(message.clone()));
                    }
                    record_termination(&inner, &sync_cleanup, generation, message).await;
                    return;
                }
                _ => {}
            }
        }

        let message = "MCP sidecar output channel closed unexpectedly".to_string();
        if let Some(sender) = ready_sender.take() {
            let _ = sender.send(Err(message.clone()));
        }
        record_termination(&inner, &sync_cleanup, generation, message).await;
    });
}

async fn record_termination(
    inner: &Arc<Mutex<RuntimeState>>,
    sync_cleanup: &Arc<Mutex<()>>,
    generation: u64,
    error: String,
) {
    let cleanup_guard = sync_cleanup.lock().await;
    let sync_server = {
        let mut inner = inner.lock().await;
        if inner.generation != generation {
            return;
        }

        inner.child = None;
        inner.pid = None;
        inner.endpoint = None;
        inner.started_at = None;
        if inner.phase == McpServicePhase::Stopping {
            inner.phase = McpServicePhase::Stopped;
            inner.last_error = None;
        } else {
            inner.phase = McpServicePhase::Error;
            inner.last_error = Some(error);
        }
        inner.sync_server.take()
    };

    if let Some(sync_error) =
        shutdown_sync_server(sync_server, "after the MCP sidecar terminated").await
    {
        let mut inner = inner.lock().await;
        if inner.generation == generation {
            let last_error = inner.last_error.get_or_insert_with(String::new);
            if !last_error.is_empty() {
                last_error.push_str("; ");
            }
            last_error.push_str(&sync_error);
            inner.phase = McpServicePhase::Error;
        }
    }
    drop(cleanup_guard);
}

async fn shutdown_sync_server(
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

fn log_output(stream: &str, bytes: &[u8]) {
    let line = String::from_utf8_lossy(bytes);
    log::debug!(
        "MCP sidecar {stream}: {}",
        line.trim_end_matches(['\r', '\n'])
    );
}

fn validate_port(port: u16) -> Result<(), String> {
    if port < 1024 {
        return Err("MCP service port must be between 1024 and 65535".to_string());
    }
    Ok(())
}

fn validate_auth_token(token: &str) -> Result<(), String> {
    if token.len() < 32 {
        return Err("MCP authentication token must contain at least 32 characters".to_string());
    }
    if token.len() > 256 {
        return Err("MCP authentication token must not exceed 256 characters".to_string());
    }
    if !token.bytes().all(is_safe_token_byte) {
        return Err(
            "MCP authentication token may only contain safe ASCII token characters".to_string(),
        );
    }
    Ok(())
}

fn is_safe_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
}

fn is_ready_line(line: &str, expected_endpoint: &str) -> bool {
    line.strip_prefix(READY_PREFIX)
        .and_then(|suffix| suffix.strip_prefix(' '))
        == Some(expected_endpoint)
}

fn endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

fn resolve_sidecar_path() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let executable_dir = executable.parent()?;
    let base_dir = if executable_dir.ends_with("deps") {
        executable_dir.parent().unwrap_or(executable_dir)
    } else {
        executable_dir
    };

    #[cfg(windows)]
    let binary_name = format!("{SIDECAR_NAME}.exe");
    #[cfg(not(windows))]
    let binary_name = SIDECAR_NAME.to_string();

    Some(base_dir.join(binary_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_validation_rejects_privileged_ports() {
        assert!(validate_port(1023).is_err());
        assert!(validate_port(1024).is_ok());
        assert!(validate_port(u16::MAX).is_ok());
    }

    #[test]
    fn auth_token_validation_accepts_url_safe_characters() {
        assert!(validate_auth_token("abcdEFGH0123-._~abcdEFGH0123abcd").is_ok());
    }

    #[test]
    fn auth_token_validation_rejects_short_or_unsafe_values() {
        assert!(validate_auth_token("too-short").is_err());
        assert!(validate_auth_token("abcdEFGH0123abcdEFGH0123abcd EFGH").is_err());
        assert!(validate_auth_token("abcdEFGH0123abcdEFGH0123abcd$EFGH").is_err());
        assert!(validate_auth_token("abcdEFGH0123abcdEFGH0123abcd+EFGH").is_err());
        assert!(validate_auth_token(&"a".repeat(257)).is_err());
    }

    #[test]
    fn readiness_requires_the_exact_marker_prefix() {
        assert!(is_ready_line(
            "ASTESIA_MCP_READY http://127.0.0.1:24872/mcp",
            "http://127.0.0.1:24872/mcp",
        ));
        assert!(!is_ready_line(
            "ASTESIA_MCP_READY",
            "http://127.0.0.1:24872/mcp",
        ));
        assert!(!is_ready_line(
            "ASTESIA_MCP_READY http://127.0.0.1:24873/mcp",
            "http://127.0.0.1:24872/mcp",
        ));
        assert!(!is_ready_line(
            "ASTESIA_MCP_READYISH http://127.0.0.1:24872/mcp",
            "http://127.0.0.1:24872/mcp",
        ));
    }

    #[test]
    fn default_runtime_state_is_stopped_and_empty() {
        let state = RuntimeState::default();
        assert_eq!(state.phase, McpServicePhase::Stopped);
        assert!(state.child.is_none());
        assert!(state.sync_server.is_none());
        assert!(state.pid.is_none());
        assert!(state.endpoint.is_none());
        assert!(state.started_at.is_none());
        assert!(state.last_error.is_none());
    }

    #[tokio::test]
    async fn late_start_failure_cannot_overwrite_a_concurrent_stop() {
        let generation = 17;
        let inner = Arc::new(Mutex::new(RuntimeState {
            phase: McpServicePhase::Starting,
            pid: Some(42),
            endpoint: Some(endpoint(24872)),
            generation,
            ..RuntimeState::default()
        }));
        let (stop_committed_tx, stop_committed_rx) = oneshot::channel();

        let stop_inner = Arc::clone(&inner);
        let stop_task = tokio::spawn(async move {
            let mut state = stop_inner.lock().await;
            state.phase = McpServicePhase::Stopping;
            stop_committed_tx
                .send(())
                .expect("startup failure task must still be waiting");
        });

        let failure_inner = Arc::clone(&inner);
        let late_failure_task = tokio::spawn(async move {
            stop_committed_rx.await.expect("stop task must complete");
            let mut state = failure_inner.lock().await;
            transition_failed_start(&mut state, generation, "startup timed out".to_string())
                .is_some()
        });

        stop_task.await.expect("stop task");
        assert!(!late_failure_task.await.expect("startup failure task"));

        {
            let mut state = inner.lock().await;
            assert_eq!(state.phase, McpServicePhase::Stopping);
            assert_eq!(state.generation, generation);
            assert_eq!(state.pid, Some(42));
            assert_eq!(
                state.endpoint.as_deref(),
                Some("http://127.0.0.1:24872/mcp")
            );
            assert!(state.last_error.is_none());

            state.phase = McpServicePhase::Stopped;
            state.pid = None;
            state.endpoint = None;
            assert!(
                transition_failed_start(&mut state, generation, "still late".to_string()).is_none()
            );
            assert_eq!(state.phase, McpServicePhase::Stopped);
            assert_eq!(state.generation, generation);
            assert!(state.last_error.is_none());
        }
    }

    #[tokio::test]
    async fn termination_finishes_an_in_progress_stop() {
        let inner = Arc::new(Mutex::new(RuntimeState {
            phase: McpServicePhase::Stopping,
            pid: Some(42),
            endpoint: Some(endpoint(24872)),
            started_at: Some(Utc::now().to_rfc3339()),
            generation: 7,
            ..RuntimeState::default()
        }));
        let cleanup = Arc::new(Mutex::new(()));

        record_termination(&inner, &cleanup, 7, "terminated".to_string()).await;

        let state = inner.lock().await;
        assert_eq!(state.phase, McpServicePhase::Stopped);
        assert!(state.pid.is_none());
        assert!(state.endpoint.is_none());
        assert!(state.started_at.is_none());
        assert!(state.last_error.is_none());
    }

    #[tokio::test]
    async fn unexpected_termination_records_the_error() {
        let inner = Arc::new(Mutex::new(RuntimeState {
            phase: McpServicePhase::Running,
            pid: Some(42),
            endpoint: Some(endpoint(24872)),
            generation: 11,
            ..RuntimeState::default()
        }));
        let cleanup = Arc::new(Mutex::new(()));

        record_termination(&inner, &cleanup, 11, "unexpected exit".to_string()).await;

        let state = inner.lock().await;
        assert_eq!(state.phase, McpServicePhase::Error);
        assert_eq!(state.last_error.as_deref(), Some("unexpected exit"));
        assert!(state.pid.is_none());
        assert!(state.endpoint.is_none());
    }

    #[test]
    fn service_phase_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&McpServicePhase::Running).unwrap(),
            "\"running\""
        );
    }

    #[test]
    fn service_status_serializes_the_frontend_contract() {
        let status = McpServiceStatus {
            state: McpServicePhase::Starting,
            available: true,
            pid: Some(42),
            endpoint: Some(endpoint(24872)),
            transport: TRANSPORT,
            binary_path: Some("/tmp/astesia-mcp".to_string()),
            version: Some("1.0.3".to_string()),
            started_at: None,
            last_error: None,
        };
        let value = serde_json::to_value(status).unwrap();

        assert_eq!(value["state"], "starting");
        assert_eq!(value["available"], true);
        assert_eq!(value["pid"], 42);
        assert_eq!(value["transport"], "streamable_http");
        assert_eq!(value["started_at"], serde_json::Value::Null);
        assert_eq!(value["last_error"], serde_json::Value::Null);
    }

    #[test]
    fn endpoint_is_loopback_only() {
        assert_eq!(endpoint(24872), "http://127.0.0.1:24872/mcp");
    }
}
