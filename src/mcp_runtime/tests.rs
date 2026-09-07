use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex as StdMutex,
};
use std::time::Duration;

use tokio::sync::oneshot;

use super::config::{endpoint, validate_auth_token, validate_port, TEST_TRANSPORT};
use super::monitor::{is_ready_line, spawn_event_monitor};
use super::*;
use crate::connection_repository::{CredentialVerificationReport, CredentialVerificationScope};
use crate::credential_vault::test_support::MemoryCredentialVault;
use crate::mcp::CREDENTIAL_VERIFY_MARKER;
use crate::platform::{
    sidecar::{SidecarControl, SidecarHost, SidecarInstallation},
    SidecarControlHandle, SidecarEvent, SidecarRequest, SpawnedSidecar, UiEventBus,
};

#[derive(Default)]
struct FakeSidecarControl {
    terminate_results: StdMutex<VecDeque<Result<(), String>>>,
    terminate_calls: AtomicUsize,
}

impl FakeSidecarControl {
    fn with_terminate_results(results: Vec<Result<(), String>>) -> Self {
        Self {
            terminate_results: StdMutex::new(results.into()),
            terminate_calls: AtomicUsize::new(0),
        }
    }

    fn terminate_calls(&self) -> usize {
        self.terminate_calls.load(Ordering::SeqCst)
    }
}

impl SidecarControl for FakeSidecarControl {
    fn terminate(&self) -> Result<(), String> {
        self.terminate_calls.fetch_add(1, Ordering::SeqCst);
        self.terminate_results
            .lock()
            .map_err(|_| "fake sidecar control unavailable".to_string())?
            .pop_front()
            .unwrap_or(Ok(()))
    }
}

#[derive(Default)]
struct BlockingTerminationControl {
    terminate_started: AtomicBool,
    release_termination: AtomicBool,
    terminate_calls: AtomicUsize,
}

impl SidecarControl for BlockingTerminationControl {
    fn terminate(&self) -> Result<(), String> {
        self.terminate_calls.fetch_add(1, Ordering::SeqCst);
        self.terminate_started.store(true, Ordering::SeqCst);
        while !self.release_termination.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        Err("termination failed".to_string())
    }
}

struct FakeSidecarHost {
    events: StdMutex<Option<Vec<SidecarEvent>>>,
    control: Arc<FakeSidecarControl>,
    verified: AtomicBool,
}

impl FakeSidecarHost {
    fn new(events: Vec<SidecarEvent>) -> Self {
        Self::with_control(events, Arc::new(FakeSidecarControl::default()))
    }

    fn with_control(events: Vec<SidecarEvent>, control: Arc<FakeSidecarControl>) -> Self {
        Self {
            events: StdMutex::new(Some(events)),
            control,
            verified: AtomicBool::new(false),
        }
    }
}

impl SidecarHost for FakeSidecarHost {
    fn installation(&self) -> SidecarInstallation {
        SidecarInstallation {
            executable_path: None,
        }
    }

    fn spawn(&self, request: SidecarRequest) -> Result<SpawnedSidecar, String> {
        if !matches!(request, SidecarRequest::VerifySharedCredentials) {
            return Err("unexpected serve request".to_string());
        }
        self.verified.store(true, Ordering::SeqCst);
        let events = self
            .events
            .lock()
            .map_err(|_| "fake event queue unavailable".to_string())?
            .take()
            .ok_or_else(|| "fake process already spawned".to_string())?;
        let (sender, receiver) = tokio::sync::mpsc::channel(events.len().max(1));
        for event in events {
            sender.try_send(event).map_err(|error| error.to_string())?;
        }
        drop(sender);
        Ok(SpawnedSidecar {
            pid: 42,
            control: self.control.clone(),
            events: receiver,
        })
    }
}

struct HangingSidecarHost {
    control: Arc<FakeSidecarControl>,
    event_sender: StdMutex<Option<tokio::sync::mpsc::Sender<SidecarEvent>>>,
    verified: AtomicBool,
}

impl HangingSidecarHost {
    fn new(control: Arc<FakeSidecarControl>) -> Self {
        Self {
            control,
            event_sender: StdMutex::new(None),
            verified: AtomicBool::new(false),
        }
    }
}

impl SidecarHost for HangingSidecarHost {
    fn installation(&self) -> SidecarInstallation {
        SidecarInstallation {
            executable_path: None,
        }
    }

    fn spawn(&self, request: SidecarRequest) -> Result<SpawnedSidecar, String> {
        if !matches!(request, SidecarRequest::VerifySharedCredentials) {
            return Err("unexpected serve request".to_string());
        }
        let (sender, events) = tokio::sync::mpsc::channel(1);
        *self
            .event_sender
            .lock()
            .map_err(|_| "fake event sender unavailable".to_string())? = Some(sender);
        self.verified.store(true, Ordering::SeqCst);
        Ok(SpawnedSidecar {
            pid: 42,
            control: self.control.clone(),
            events,
        })
    }
}

struct BlockingTerminationHost {
    control: Arc<BlockingTerminationControl>,
    spawn_calls: AtomicUsize,
}

impl SidecarHost for BlockingTerminationHost {
    fn installation(&self) -> SidecarInstallation {
        SidecarInstallation {
            executable_path: None,
        }
    }

    fn spawn(&self, request: SidecarRequest) -> Result<SpawnedSidecar, String> {
        if !matches!(request, SidecarRequest::VerifySharedCredentials) {
            return Err("unexpected serve request".to_string());
        }
        self.spawn_calls.fetch_add(1, Ordering::SeqCst);
        let (sender, events) = tokio::sync::mpsc::channel(1);
        drop(sender);
        Ok(SpawnedSidecar {
            pid: 42,
            control: self.control.clone(),
            events,
        })
    }
}

fn runtime_with_host(host: Arc<dyn SidecarHost>) -> (tempfile::TempDir, McpRuntime) {
    let directory = tempfile::TempDir::new().expect("tempdir");
    let repository = crate::connection_repository::SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    let registry = McpSyncRegistry::new(repository, Arc::new(UiEventBus::new()));
    (directory, McpRuntime::new(host, registry))
}

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

#[tokio::test]
async fn readiness_marker_can_span_output_chunks() {
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(4);
    let (ready_tx, ready_rx) = oneshot::channel();
    spawn_event_monitor(event_rx, endpoint(24872), Some(ready_tx), |_, _| async {});
    event_tx
        .send(SidecarEvent::Stderr(b"ASTESIA_MCP_".to_vec()))
        .await
        .expect("first output chunk");
    event_tx
        .send(SidecarEvent::Stderr(
            b"READY http://127.0.0.1:24872/mcp".to_vec(),
        ))
        .await
        .expect("second output chunk");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), ready_rx)
            .await
            .expect("readiness timeout")
            .expect("readiness sender"),
        Ok(())
    );
    event_tx
        .send(SidecarEvent::Terminated {
            code: Some(0),
            signal: None,
        })
        .await
        .expect("termination event");
}

#[tokio::test]
async fn verification_uses_the_sidecar_host_and_parses_split_output() {
    let scope = CredentialVerificationScope {
        repository_id: "repository-1".to_string(),
        repository_revision: 9,
        profile_count: 3,
        credential_count: 2,
        profile_digest: "digest".to_string(),
    };
    let output = format!(
        "{CREDENTIAL_VERIFY_MARKER}{}\n",
        serde_json::to_string(&CredentialVerificationReport::success(scope.clone()))
            .expect("verification report")
    );
    let split_at = CREDENTIAL_VERIFY_MARKER.len() / 2;
    let host = Arc::new(FakeSidecarHost::new(vec![
        SidecarEvent::Stdout(output.as_bytes()[..split_at].to_vec()),
        SidecarEvent::Stdout(output.as_bytes()[split_at..].to_vec()),
        SidecarEvent::Terminated {
            code: Some(0),
            signal: None,
        },
    ]));
    let (_directory, helper) = runtime_with_host(host.clone());
    let report = helper
        .verify_shared_credentials()
        .await
        .expect("verification succeeds");
    assert!(host.verified.load(Ordering::SeqCst));
    assert!(report.ok);
    assert_eq!(report.verified, 2);
    assert_eq!(report.scope.expect("scope"), scope);
    assert!(report.error.is_none());
}

#[tokio::test]
async fn verification_terminates_the_process_when_the_event_channel_closes() {
    let host = Arc::new(FakeSidecarHost::new(Vec::new()));
    let control = Arc::clone(&host.control);
    let (_directory, helper) = runtime_with_host(host);
    let error = helper
        .verify_shared_credentials()
        .await
        .expect_err("a closed event channel cannot verify credentials");
    assert!(error.to_string().contains("输出通道意外关闭"));
    assert_eq!(control.terminate_calls(), 1);
}

#[tokio::test]
async fn default_runtime_state_is_stopped_and_empty() {
    let host = Arc::new(FakeSidecarHost::new(Vec::new()));
    let (_directory, helper) = runtime_with_host(host);
    let status = helper.status().await;
    assert_eq!(status.state, McpServicePhase::Stopped);
    assert!(status.pid.is_none());
    assert!(status.endpoint.is_none());
    assert!(status.started_at.is_none());
    assert!(status.last_error.is_none());
}

#[tokio::test]
async fn late_start_failure_cannot_overwrite_a_concurrent_stop() {
    let host = Arc::new(FakeSidecarHost::new(Vec::new()));
    let (_directory, helper) = runtime_with_host(host);
    let control: SidecarControlHandle = Arc::new(FakeSidecarControl::default());
    helper.service.seed_stopping(control, 17).await;
    assert!(
        !helper
            .service
            .fail_test_start(17, "startup timed out")
            .await
    );
    let status = helper.status().await;
    assert_eq!(status.state, McpServicePhase::Stopping);
    assert_eq!(status.pid, Some(42));
    assert_eq!(
        status.endpoint.as_deref(),
        Some("http://127.0.0.1:24872/mcp")
    );
    assert!(status.last_error.is_none());
}

#[tokio::test]
async fn termination_finishes_an_in_progress_stop() {
    let host = Arc::new(FakeSidecarHost::new(Vec::new()));
    let (_directory, helper) = runtime_with_host(host);
    let control: SidecarControlHandle = Arc::new(FakeSidecarControl::default());
    helper.service.seed_stopping(control, 7).await;
    helper
        .service
        .record_test_termination(7, "terminated", false)
        .await;
    assert_eq!(helper.status().await.state, McpServicePhase::Stopped);
}

#[tokio::test]
async fn unexpected_termination_records_the_error() {
    let host = Arc::new(FakeSidecarHost::new(Vec::new()));
    let (_directory, helper) = runtime_with_host(host);
    let control: SidecarControlHandle = Arc::new(FakeSidecarControl::default());
    helper.service.seed_failed_process(control, 11, true).await;
    helper
        .service
        .record_test_termination(11, "unexpected exit", false)
        .await;
    let status = helper.status().await;
    assert_eq!(status.state, McpServicePhase::Error);
    assert_eq!(status.last_error.as_deref(), Some("unexpected exit"));
    assert!(status.pid.is_none());
    assert!(status.endpoint.is_none());
}

#[tokio::test]
async fn closed_monitor_channel_terminates_the_unobserved_process() {
    let host = Arc::new(FakeSidecarHost::new(Vec::new()));
    let (_directory, helper) = runtime_with_host(host);
    let control = Arc::new(FakeSidecarControl::default());
    helper
        .service
        .seed_failed_process(control.clone(), 13, true)
        .await;
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(1);
    let service = helper.service.clone();
    spawn_event_monitor(
        event_rx,
        endpoint(24872),
        None,
        move |message, unobserved| {
            let service = service.clone();
            async move {
                service
                    .record_test_termination(13, &message, unobserved)
                    .await;
            }
        },
    );
    drop(event_tx);
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let status = helper.status().await;
            if status.pid.is_none()
                && status.last_error.as_deref()
                    == Some("MCP sidecar output channel closed unexpectedly")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("event monitor cleanup");
    assert_eq!(control.terminate_calls(), 1);
}

#[tokio::test]
async fn failed_stop_retains_control_for_a_later_attempt() {
    let control = Arc::new(FakeSidecarControl::with_terminate_results(vec![
        Err("temporarily unavailable".to_string()),
        Ok(()),
    ]));
    let host = Arc::new(FakeSidecarHost::new(Vec::new()));
    let (_directory, helper) = runtime_with_host(host);
    helper
        .service
        .seed_failed_process(control.clone(), 19, false)
        .await;
    helper
        .service
        .stop_with_test_timeout(Duration::from_millis(1))
        .await
        .expect_err("the first stop signal fails");
    let failed = helper.status().await;
    assert_eq!(failed.state, McpServicePhase::Error);
    assert_eq!(failed.pid, Some(42));
    let status = helper
        .service
        .stop_with_test_timeout(Duration::from_millis(1))
        .await
        .expect("a successful retry recovers without an event monitor");
    assert_eq!(control.terminate_calls(), 2);
    assert_eq!(status.state, McpServicePhase::Stopped);
    assert!(status.pid.is_none());
}

#[tokio::test]
async fn failed_unobserved_termination_keeps_process_control_and_pid() {
    let control = Arc::new(FakeSidecarControl::with_terminate_results(vec![Err(
        "process access denied".to_string(),
    )]));
    let host = Arc::new(FakeSidecarHost::new(Vec::new()));
    let (_directory, helper) = runtime_with_host(host);
    helper
        .service
        .seed_failed_process(control.clone(), 23, true)
        .await;
    helper
        .service
        .record_test_termination(23, "event channel closed", true)
        .await;
    let status = helper.status().await;
    assert_eq!(control.terminate_calls(), 1);
    assert_eq!(status.state, McpServicePhase::Error);
    assert_eq!(status.pid, Some(42));
    assert!(status
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("process access denied")));
}

#[tokio::test]
async fn failed_verifier_termination_is_retained_and_retried() {
    let control = Arc::new(FakeSidecarControl::with_terminate_results(vec![
        Err("temporarily unavailable".to_string()),
        Ok(()),
    ]));
    let host = Arc::new(FakeSidecarHost::with_control(
        Vec::new(),
        Arc::clone(&control),
    ));
    let (_directory, helper) = runtime_with_host(host);
    helper
        .verify_shared_credentials()
        .await
        .expect_err("the verifier event channel closes");
    assert_eq!(control.terminate_calls(), 1);
    assert_eq!(helper.verifier.pending_terminations(), 1);
    assert!(helper.verifier.retry_pending_terminations().is_empty());
    assert_eq!(control.terminate_calls(), 2);
    assert_eq!(helper.verifier.pending_terminations(), 0);
}

#[tokio::test]
async fn unresolved_verifier_cleanup_blocks_another_spawn() {
    let control = Arc::new(FakeSidecarControl::with_terminate_results(vec![
        Err("first termination failed".to_string()),
        Err("retry termination failed".to_string()),
    ]));
    let host = Arc::new(FakeSidecarHost::with_control(
        Vec::new(),
        Arc::clone(&control),
    ));
    let (_directory, helper) = runtime_with_host(host);
    helper
        .verify_shared_credentials()
        .await
        .expect_err("the first verifier channel closes");
    let error = helper
        .verify_shared_credentials()
        .await
        .expect_err("unresolved cleanup must block another verifier");
    assert!(error.to_string().contains("无法清理上一次"));
    assert_eq!(control.terminate_calls(), 2);
    assert_eq!(helper.verifier.pending_terminations(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verifier_stays_registered_while_termination_is_in_progress() {
    let control = Arc::new(BlockingTerminationControl::default());
    let host = Arc::new(BlockingTerminationHost {
        control: Arc::clone(&control),
        spawn_calls: AtomicUsize::new(0),
    });
    let (_directory, helper) = runtime_with_host(host.clone());
    let first_verifier = tokio::spawn({
        let helper = helper.clone();
        async move { helper.verify_shared_credentials().await }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !control.terminate_started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("termination started");
    let error = helper
        .verify_shared_credentials()
        .await
        .expect_err("terminating verifier remains active");
    assert!(error.to_string().contains("already running"));
    assert_eq!(host.spawn_calls.load(Ordering::SeqCst), 1);
    control.release_termination.store(true, Ordering::SeqCst);
    first_verifier
        .await
        .expect("first verifier task")
        .expect_err("first verifier termination fails");
    assert_eq!(control.terminate_calls.load(Ordering::SeqCst), 1);
    assert_eq!(helper.verifier.pending_terminations(), 1);
}

#[tokio::test]
async fn cancelling_verification_terminates_and_unregisters_the_process() {
    let control = Arc::new(FakeSidecarControl::default());
    let host = Arc::new(HangingSidecarHost::new(Arc::clone(&control)));
    let (_directory, helper) = runtime_with_host(host.clone());
    let verifier = tokio::spawn({
        let helper = helper.clone();
        async move { helper.verify_shared_credentials().await }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !host.verified.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("verifier spawn");
    verifier.abort();
    assert!(verifier
        .await
        .expect_err("verifier cancellation")
        .is_cancelled());
    assert_eq!(control.terminate_calls(), 1);
    assert!(!helper.verifier.has_active_process());
}

#[tokio::test]
async fn shutdown_terminates_active_verification_and_blocks_new_work() {
    let control = Arc::new(FakeSidecarControl::default());
    let host = Arc::new(HangingSidecarHost::new(Arc::clone(&control)));
    let (_directory, helper) = runtime_with_host(host.clone());
    let verifier = tokio::spawn({
        let helper = helper.clone();
        async move { helper.verify_shared_credentials().await }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !host.verified.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("verifier spawn");
    helper.shutdown().await;
    assert_eq!(control.terminate_calls(), 1);
    let error = helper
        .verify_shared_credentials()
        .await
        .expect_err("shutdown must reject new verification");
    assert!(error.to_string().contains("shutting down"));
    verifier.abort();
    let _ = verifier.await;
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
        transport: TEST_TRANSPORT,
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
