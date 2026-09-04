use super::registry::{
    protocol::{success, validate_context},
    state::{
        snapshot_from_state, McpUsePhase, OwnershipKey, RegistryEntry, RegistryState,
        MAX_CLOSED_TOMBSTONES,
    },
};
use super::*;
use crate::mcp_auth::constant_time_eq;
use crate::mcp_sync::{
    McpControlCommand, McpSyncContext, McpSyncRequest, McpSyncResponse, PROTOCOL_VERSION,
};
use serde_json::Value;
use uuid::Uuid;

fn context(service_id: Uuid, session_id: Uuid) -> McpSyncContext {
    McpSyncContext {
        protocol_version: PROTOCOL_VERSION,
        service_id,
        session_id,
        operation_id: Uuid::new_v4(),
    }
}

async fn acquire(
    registry: &McpSyncRegistry,
    service_id: Uuid,
    session_id: Uuid,
    connection_id: &str,
    revision: i64,
) -> McpSyncResponse {
    registry
        .apply(
            service_id,
            McpSyncRequest::Acquire {
                context: context(service_id, session_id),
                connection_id: connection_id.into(),
                profile_revision: revision,
            },
        )
        .await
}

async fn connected(
    registry: &McpSyncRegistry,
    service_id: Uuid,
    session_id: Uuid,
    connection_id: &str,
    generation: u64,
) -> McpSyncResponse {
    registry
        .apply(
            service_id,
            McpSyncRequest::Connected {
                context: context(service_id, session_id),
                connection_id: connection_id.into(),
                generation,
            },
        )
        .await
}

async fn released(
    registry: &McpSyncRegistry,
    service_id: Uuid,
    session_id: Uuid,
    connection_id: &str,
    generation: u64,
) -> McpSyncResponse {
    registry
        .apply(
            service_id,
            McpSyncRequest::Released {
                context: context(service_id, session_id),
                connection_id: connection_id.into(),
                generation,
            },
        )
        .await
}

async fn wait_for_control_poll(registry: &McpSyncRegistry) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while registry.retained_control_notifies().await == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("control poll must install its notifier");
}

#[tokio::test]
async fn acquire_uses_canonical_shared_id_and_never_serializes_profile_or_credentials() {
    let registry = McpSyncRegistry::without_app_events();
    registry.allow_test_profile("shared-id", 7, true).await;
    let service_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();

    let response = acquire(&registry, service_id, session_id, "shared-id", 7).await;
    assert!(response.ok);
    assert!(response.generation.is_some());
    let snapshot = registry.snapshot().await;
    assert_eq!(snapshot.connections.len(), 1);
    assert_eq!(snapshot.connections[0].id, "shared-id");
    assert_eq!(snapshot.connections[0].profile_revision, 7);
    assert!(snapshot.connections[0].mcp_in_use);
    assert!(!snapshot.connections[0].mcp_connected);

    let value = serde_json::to_value(snapshot).expect("serialize snapshot");
    let serialized = serde_json::to_string(&value).expect("serialize JSON");
    for forbidden in [
        "password",
        "password_env",
        "username",
        "host",
        "database",
        "credential",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "snapshot leaked forbidden field {forbidden}"
        );
    }
}

#[tokio::test]
async fn acquire_rejects_unknown_disabled_or_stale_shared_profiles() {
    let registry = McpSyncRegistry::without_app_events();
    registry.allow_test_profile("disabled", 2, false).await;
    registry.allow_test_profile("current", 5, true).await;
    let service_id = Uuid::new_v4();

    for (id, revision) in [("missing", 1), ("disabled", 2), ("current", 4)] {
        let response = acquire(&registry, service_id, Uuid::new_v4(), id, revision).await;
        assert!(!response.ok, "{id} should be rejected");
    }
    assert!(registry.snapshot().await.connections.is_empty());
}

#[tokio::test]
async fn identical_connection_ids_aggregate_across_http_sessions() {
    let registry = McpSyncRegistry::without_app_events();
    registry.allow_test_profile("shared", 3, true).await;
    let service_id = Uuid::new_v4();
    let session_a = Uuid::new_v4();
    let session_b = Uuid::new_v4();
    let generation_a = acquire(&registry, service_id, session_a, "shared", 3)
        .await
        .generation
        .unwrap();
    let generation_b = acquire(&registry, service_id, session_b, "shared", 3)
        .await
        .generation
        .unwrap();
    assert_ne!(generation_a, generation_b);
    connected(&registry, service_id, session_a, "shared", generation_a).await;
    connected(&registry, service_id, session_b, "shared", generation_b).await;

    let snapshot = registry.snapshot().await;
    assert_eq!(snapshot.connections.len(), 1);
    assert_eq!(snapshot.connections[0].mcp_session_count, 2);
    assert!(snapshot.connections[0].mcp_connected);

    released(&registry, service_id, session_a, "shared", generation_a).await;
    let snapshot = registry.snapshot().await;
    assert_eq!(snapshot.connections[0].mcp_session_count, 1);
    assert!(registry.is_connection_in_use("shared").await);
}

#[tokio::test]
async fn lifecycle_guard_linearizes_profile_mutation_and_acquire() {
    let registry = McpSyncRegistry::without_app_events();
    registry.allow_test_profile("guarded", 1, true).await;
    let guard = registry.lock_connection_lifecycle("guarded").await;
    let acquire_registry = registry.clone();
    let service_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let task = tokio::spawn(async move {
        acquire(&acquire_registry, service_id, session_id, "guarded", 1).await
    });

    tokio::task::yield_now().await;
    assert!(!registry.is_connection_in_use("guarded").await);
    assert!(!task.is_finished());
    drop(guard);
    assert!(task.await.expect("acquire task").ok);
    assert!(registry.is_connection_in_use("guarded").await);
}

#[tokio::test]
async fn force_disconnect_is_pushed_to_the_target_session_and_acknowledged() {
    let registry = McpSyncRegistry::without_app_events();
    registry.allow_test_profile("force", 9, true).await;
    let service_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let generation = acquire(&registry, service_id, session_id, "force", 9)
        .await
        .generation
        .unwrap();
    assert!(
        connected(&registry, service_id, session_id, "force", generation)
            .await
            .ok
    );

    let force_registry = registry.clone();
    let force_task = tokio::spawn(async move { force_registry.force_disconnect("force").await });
    tokio::task::yield_now().await;
    let poll = registry
        .apply(
            service_id,
            McpSyncRequest::PollControl {
                context: context(service_id, session_id),
            },
        )
        .await;
    let command = poll.control.expect("force-disconnect command");
    assert_eq!(command.connection_id, "force");
    assert_eq!(command.generation, generation);

    let acknowledgement = registry
        .apply(
            service_id,
            McpSyncRequest::ControlResult {
                context: context(service_id, session_id),
                command_id: command.command_id,
                connection_id: command.connection_id,
                generation: command.generation,
                ok: true,
                error: None,
            },
        )
        .await;
    assert!(acknowledgement.ok);
    let result = force_task
        .await
        .expect("force task")
        .expect("force disconnect");
    assert_eq!(
        result,
        ForceDisconnectResult {
            requested: 1,
            completed: 1
        }
    );
    assert!(!registry.is_connection_in_use("force").await);
    assert!(registry.snapshot().await.connections.is_empty());
}

#[tokio::test]
async fn force_disconnect_validation_error_has_zero_progress() {
    let registry = McpSyncRegistry::without_app_events();

    let error = registry
        .force_disconnect(" ")
        .await
        .expect_err("blank identifier must fail validation");

    assert_eq!(
        error,
        ForceDisconnectError {
            requested: 0,
            completed: 0,
            error: "MCP connection identifier must not be empty".to_string(),
        }
    );
}

#[tokio::test]
async fn delayed_force_command_cannot_remove_a_reconnected_generation() {
    let registry = McpSyncRegistry::without_app_events();
    registry.allow_test_profile("aba", 4, true).await;
    let service_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let first_generation = acquire(&registry, service_id, session_id, "aba", 4)
        .await
        .generation
        .unwrap();
    connected(&registry, service_id, session_id, "aba", first_generation).await;

    let force_registry = registry.clone();
    let force_task = tokio::spawn(async move { force_registry.force_disconnect("aba").await });
    tokio::task::yield_now().await;
    let command = registry
        .apply(
            service_id,
            McpSyncRequest::PollControl {
                context: context(service_id, session_id),
            },
        )
        .await
        .control
        .expect("queued control");
    assert_eq!(command.generation, first_generation);

    released(&registry, service_id, session_id, "aba", first_generation).await;
    assert!(force_task.await.expect("force task").is_ok());
    let second_generation = acquire(&registry, service_id, session_id, "aba", 4)
        .await
        .generation
        .unwrap();
    assert_ne!(first_generation, second_generation);
    connected(&registry, service_id, session_id, "aba", second_generation).await;

    let late_ack = registry
        .apply(
            service_id,
            McpSyncRequest::ControlResult {
                context: context(service_id, session_id),
                command_id: command.command_id,
                connection_id: command.connection_id,
                generation: command.generation,
                ok: true,
                error: None,
            },
        )
        .await;
    assert!(late_ack.ok);
    let snapshot = registry.snapshot().await;
    assert_eq!(snapshot.connections.len(), 1);
    assert!(snapshot.connections[0].mcp_connected);
    let state = registry.inner.lock().await;
    let entry = state.entries.values().next().expect("reconnected entry");
    assert_eq!(entry.generation, second_generation);
}

#[tokio::test]
async fn failed_force_disconnect_keeps_profile_in_use_and_reports_error() {
    let registry = McpSyncRegistry::without_app_events();
    registry.allow_test_profile("nack", 1, true).await;
    let service_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let generation = acquire(&registry, service_id, session_id, "nack", 1)
        .await
        .generation
        .unwrap();
    connected(&registry, service_id, session_id, "nack", generation).await;

    let force_registry = registry.clone();
    let force_task = tokio::spawn(async move { force_registry.force_disconnect("nack").await });
    tokio::task::yield_now().await;
    let command = registry
        .apply(
            service_id,
            McpSyncRequest::PollControl {
                context: context(service_id, session_id),
            },
        )
        .await
        .control
        .expect("queued control");
    registry
        .apply(
            service_id,
            McpSyncRequest::ControlResult {
                context: context(service_id, session_id),
                command_id: command.command_id,
                connection_id: command.connection_id,
                generation: command.generation,
                ok: false,
                error: Some("driver refused to close".into()),
            },
        )
        .await;

    let error = force_task
        .await
        .expect("force task")
        .expect_err("nack must fail force disconnect");
    assert_eq!(
        error,
        ForceDisconnectError {
            requested: 1,
            completed: 0,
            error: "driver refused to close".to_string(),
        }
    );
    assert_eq!(
            error.to_string(),
            "Unable to disconnect all Streamable HTTP MCP sessions (0/1 completed): driver refused to close"
        );
    assert!(registry.is_connection_in_use("nack").await);
    let snapshot = registry.snapshot().await;
    assert!(snapshot.connections[0].mcp_connected);
    assert!(!snapshot.connections[0].disconnecting);
    assert_eq!(
        snapshot.connections[0].last_error.as_deref(),
        Some("driver refused to close")
    );
}

#[tokio::test]
async fn closing_session_releases_usage_and_completes_pending_force_disconnect() {
    let registry = McpSyncRegistry::without_app_events();
    registry.allow_test_profile("close", 2, true).await;
    let service_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let generation = acquire(&registry, service_id, session_id, "close", 2)
        .await
        .generation
        .unwrap();
    connected(&registry, service_id, session_id, "close", generation).await;

    let force_registry = registry.clone();
    let force_task = tokio::spawn(async move { force_registry.force_disconnect("close").await });
    tokio::task::yield_now().await;
    let closed = registry
        .apply(
            service_id,
            McpSyncRequest::SessionClosed {
                context: context(service_id, session_id),
            },
        )
        .await;
    assert!(closed.ok);
    assert!(force_task.await.expect("force task").is_ok());
    assert!(!registry.is_connection_in_use("close").await);
    assert_eq!(registry.retained_control_notifies().await, 0);

    let late = acquire(&registry, service_id, session_id, "close", 2).await;
    assert!(!late.ok);
}

#[tokio::test]
async fn session_close_wakes_long_poll_and_removes_its_notifier() {
    let registry = McpSyncRegistry::without_app_events();
    let service_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let poll_registry = registry.clone();
    let poll = tokio::spawn(async move {
        poll_registry
            .apply(
                service_id,
                McpSyncRequest::PollControl {
                    context: context(service_id, session_id),
                },
            )
            .await
    });
    wait_for_control_poll(&registry).await;

    let closed = registry
        .apply(
            service_id,
            McpSyncRequest::SessionClosed {
                context: context(service_id, session_id),
            },
        )
        .await;

    assert!(closed.ok);
    let poll_result = tokio::time::timeout(std::time::Duration::from_secs(1), poll)
        .await
        .expect("closed poll must wake")
        .expect("poll task");
    assert!(!poll_result.ok);
    assert_eq!(registry.retained_control_notifies().await, 0);
    let state = registry.inner.lock().await;
    assert!(state.closed_sessions.contains(&(service_id, session_id)));
}

#[tokio::test]
async fn service_reset_wakes_long_poll_and_removes_its_notifier() {
    let registry = McpSyncRegistry::without_app_events();
    let service_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let poll_registry = registry.clone();
    let poll = tokio::spawn(async move {
        poll_registry
            .apply(
                service_id,
                McpSyncRequest::PollControl {
                    context: context(service_id, session_id),
                },
            )
            .await
    });
    wait_for_control_poll(&registry).await;

    registry.reset_service(service_id).await;

    let poll_result = tokio::time::timeout(std::time::Duration::from_secs(1), poll)
        .await
        .expect("reset poll must wake")
        .expect("poll task");
    assert!(!poll_result.ok);
    assert_eq!(registry.retained_control_notifies().await, 0);
    let state = registry.inner.lock().await;
    assert!(state.closed_services.contains(&service_id));
}

#[tokio::test]
async fn closed_markers_are_bounded_and_service_reset_clears_session_state() {
    let registry = McpSyncRegistry::without_app_events();
    let service_id = Uuid::new_v4();

    for _ in 0..(MAX_CLOSED_TOMBSTONES + 32) {
        let session_id = Uuid::new_v4();
        let response = registry
            .apply(
                service_id,
                McpSyncRequest::SessionClosed {
                    context: context(service_id, session_id),
                },
            )
            .await;
        assert!(response.ok);
    }
    {
        let state = registry.inner.lock().await;
        assert_eq!(state.closed_sessions.len(), MAX_CLOSED_TOMBSTONES);
    }

    registry.reset_service(service_id).await;
    {
        let state = registry.inner.lock().await;
        assert_eq!(state.closed_sessions.len(), 0);
        assert!(state.closed_services.contains(&service_id));
    }
    assert_eq!(registry.retained_control_notifies().await, 0);

    let late = acquire(&registry, service_id, Uuid::new_v4(), "closed-service", 1).await;
    assert!(!late.ok);

    for _ in 0..(MAX_CLOSED_TOMBSTONES + 32) {
        registry.reset_service(Uuid::new_v4()).await;
    }
    let state = registry.inner.lock().await;
    assert_eq!(state.closed_services.len(), MAX_CLOSED_TOMBSTONES);
}

#[tokio::test]
async fn duplicate_operation_id_returns_the_original_acquire_generation() {
    let registry = McpSyncRegistry::without_app_events();
    registry.allow_test_profile("idempotent", 1, true).await;
    let service_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let request = McpSyncRequest::Acquire {
        context: context(service_id, session_id),
        connection_id: "idempotent".into(),
        profile_revision: 1,
    };

    let first = registry.apply(service_id, request.clone()).await;
    let second = registry.apply(service_id, request).await;
    assert!(first.ok);
    assert_eq!(first, second);
    assert_eq!(registry.snapshot().await.connections.len(), 1);
}

#[test]
fn snapshot_contract_contains_only_shared_usage_state() {
    let mut state = RegistryState {
        revision: 3,
        ..RegistryState::default()
    };
    state.entries.insert(
        OwnershipKey::new(Uuid::new_v4(), Uuid::new_v4(), "shared".into()),
        RegistryEntry {
            profile_revision: 8,
            generation: 2,
            phase: McpUsePhase::Connected,
            last_error: None,
        },
    );
    let value = serde_json::to_value(snapshot_from_state(&state)).unwrap();
    assert_eq!(value["revision"], 3);
    assert_eq!(value["connections"][0]["id"], "shared");
    assert_eq!(value["connections"][0]["profile_revision"], 8);
    assert_eq!(value["connections"][0]["mcp_in_use"], true);
    assert_eq!(value["connections"][0]["mcp_connected"], true);
    assert_eq!(value["connections"][0]["mcp_session_count"], 1);
    assert_eq!(value["connections"][0]["disconnecting"], false);
    let object = value["connections"][0].as_object().unwrap();
    for forbidden in [
        "name",
        "db_type",
        "host",
        "port",
        "username",
        "database",
        "password",
        "password_env",
        "source",
        "app_connected",
    ] {
        assert!(!object.contains_key(forbidden), "unexpected {forbidden}");
    }
}

#[test]
fn bearer_comparison_is_exact() {
    assert!(constant_time_eq(b"Bearer abc", b"Bearer abc"));
    assert!(!constant_time_eq(b"Bearer abc", b"Bearer abd"));
    assert!(!constant_time_eq(b"Bearer abc", b"Bearer abc "));
}

#[test]
fn protocol_version_rejects_old_transient_profile_clients() {
    let service_id = Uuid::new_v4();
    let mut request_context = context(service_id, Uuid::new_v4());
    request_context.protocol_version = 1;
    assert!(validate_context(&request_context, service_id).is_err());
}

#[test]
fn response_serializes_control_without_any_connection_profile() {
    let command = McpControlCommand {
        command_id: Uuid::new_v4(),
        connection_id: "shared".into(),
        generation: 4,
    };
    let value = serde_json::to_value(success(None, Some(command))).unwrap();
    assert_eq!(value["ok"], Value::Bool(true));
    assert_eq!(value["control"]["connection_id"], "shared");
    assert_eq!(value["control"]["generation"], 4);
    assert!(value.get("generation").is_none());
}
