use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use rmcp::{model::CallToolResult, service::ElicitationError, ServiceError};
use serde_json::{json, Value};
use tokio::sync::{mpsc, Notify};

use crate::{
    connection_repository::{ConnectionRepositoryError, ConnectionRepositoryErrorCode},
    db::DbType,
    mcp_auth::{constant_time_eq, MAX_TOKEN_BYTES},
};

use super::{
    approval::{map_elicitation_failure, ApprovalKind},
    catalog::CatalogError,
    execution::{MAX_RESULT_ROWS, MAX_SELECTOR_BYTES},
    failure::{
        ERROR_CODE_APPROVAL_CANCELLED, ERROR_CODE_APPROVAL_DECLINED,
        ERROR_CODE_APPROVAL_INVALID_RESPONSE, ERROR_CODE_APPROVAL_TIMEOUT,
        ERROR_CODE_APPROVAL_UNAVAILABLE, ERROR_CODE_APPROVAL_UNSUPPORTED, ERROR_CODE_TOOL_FAILED,
    },
    session::{ActiveConnectionTests, ActiveTestMarker, PendingSyncLease, SyncLeaseClient},
    transport::validate_http_auth_token,
    AstesiaMcp,
};

fn structured_error_payload(result: CallToolResult) -> Value {
    assert_eq!(result.is_error, Some(true));
    result
        .structured_content
        .expect("structured MCP error payload")
}

#[derive(Debug, PartialEq, Eq)]
enum SyncLeaseEvent {
    Connected(String, u64),
    Released(String, u64),
}

struct RecordingSyncLeaseClient {
    events: mpsc::UnboundedSender<SyncLeaseEvent>,
    connected_error: Option<String>,
    release_gate: Option<Arc<Notify>>,
}

#[async_trait]
impl SyncLeaseClient for RecordingSyncLeaseClient {
    async fn connected(&self, connection_id: String, generation: u64) -> Result<(), String> {
        self.events
            .send(SyncLeaseEvent::Connected(connection_id, generation))
            .expect("record connected transition");
        match &self.connected_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    async fn released(&self, connection_id: String, generation: u64) -> Result<(), String> {
        self.events
            .send(SyncLeaseEvent::Released(connection_id, generation))
            .expect("record released transition");
        if let Some(gate) = &self.release_gate {
            gate.notified().await;
        }
        Ok(())
    }
}

#[test]
fn failure_payload_keeps_the_legacy_error_string_and_adds_metadata() {
    let payload = structured_error_payload(AstesiaMcp::failure("legacy failure message"));

    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"], "legacy failure message");
    assert_eq!(payload["error_code"], ERROR_CODE_TOOL_FAILED);
    assert_eq!(payload["retryable"], false);
    assert_eq!(payload["details"], json!({}));
    assert!(payload.get("remediation").is_none());
}

#[test]
fn repository_failure_preserves_migration_reason_and_remediation() {
    let payload = structured_error_payload(AstesiaMcp::failure(CatalogError::Repository(
        ConnectionRepositoryError {
            code: ConnectionRepositoryErrorCode::CredentialMigrationRequired,
            message: "旧版凭据尚未完成安全迁移".to_string(),
            remediation: "请先打开 Astesia App 完成强制凭据迁移。".to_string(),
            retryable: false,
            details: Box::new(json!({ "connection_id": "legacy-connection" })),
        },
    )));

    assert_eq!(payload["error"], "旧版凭据尚未完成安全迁移");
    assert_eq!(
        payload["error_code"],
        ConnectionRepositoryErrorCode::CredentialMigrationRequired.as_str()
    );
    assert_eq!(
        payload["remediation"],
        "请先打开 Astesia App 完成强制凭据迁移。"
    );
    assert_eq!(payload["retryable"], false);
    assert_eq!(
        payload["details"],
        json!({ "connection_id": "legacy-connection" })
    );
}

#[test]
fn maps_elicitation_errors_to_stable_codes() {
    let parse_error = serde_json::from_value::<bool>(json!({ "confirm": true }))
        .expect_err("object must not deserialize as a boolean");
    let cases = vec![
        (
            ElicitationError::CapabilityNotSupported,
            ERROR_CODE_APPROVAL_UNSUPPORTED,
            false,
        ),
        (
            ElicitationError::UserDeclined,
            ERROR_CODE_APPROVAL_DECLINED,
            false,
        ),
        (
            ElicitationError::UserCancelled,
            ERROR_CODE_APPROVAL_CANCELLED,
            false,
        ),
        (
            ElicitationError::Service(ServiceError::Timeout {
                timeout: Duration::from_secs(1),
            }),
            ERROR_CODE_APPROVAL_TIMEOUT,
            true,
        ),
        (
            ElicitationError::ParseError {
                error: parse_error,
                data: json!({ "confirm": true }),
            },
            ERROR_CODE_APPROVAL_INVALID_RESPONSE,
            false,
        ),
        (
            ElicitationError::NoContent,
            ERROR_CODE_APPROVAL_INVALID_RESPONSE,
            false,
        ),
        (
            ElicitationError::Service(ServiceError::UnexpectedResponse),
            ERROR_CODE_APPROVAL_UNAVAILABLE,
            true,
        ),
    ];

    for (error, expected_code, expected_retryable) in cases {
        let failure = map_elicitation_failure(ApprovalKind::Update, error);
        assert_eq!(failure.code, expected_code);
        assert_eq!(failure.retryable, expected_retryable);
        assert_eq!(
            failure.details.as_ref(),
            &json!({ "approval_kind": "update" })
        );
        assert!(failure.message.contains("更新操作"));
        if expected_code == ERROR_CODE_APPROVAL_UNSUPPORTED {
            assert!(failure.message.contains("未声明 MCP form elicitation 能力"));
            assert!(failure.message.contains("没有执行该操作"));
        } else if expected_code == ERROR_CODE_APPROVAL_DECLINED {
            assert_eq!(failure.message, "用户拒绝确认，更新操作未执行");
        } else if expected_code == ERROR_CODE_APPROVAL_CANCELLED {
            assert_eq!(failure.message, "用户取消确认，更新操作未执行");
        }
    }
}

#[test]
fn unsupported_elicitation_explains_refusal_for_every_approval_kind() {
    for kind in [ApprovalKind::Destructive, ApprovalKind::Update] {
        let failure = map_elicitation_failure(kind, ElicitationError::CapabilityNotSupported);

        assert_eq!(failure.code, ERROR_CODE_APPROVAL_UNSUPPORTED);
        assert!(!failure.retryable);
        assert_eq!(
            failure.details.as_ref(),
            &json!({ "approval_kind": kind.as_str() })
        );
        assert!(failure.message.contains("未声明 MCP form elicitation 能力"));
        assert!(failure.message.contains("没有执行该操作"));

        let payload = structured_error_payload(failure.into());
        assert_eq!(payload["ok"], false);
        assert_eq!(payload["error_code"], ERROR_CODE_APPROVAL_UNSUPPORTED);
        assert_eq!(payload["retryable"], false);
        assert_eq!(
            payload["details"],
            json!({ "approval_kind": kind.as_str() })
        );
    }
}

#[test]
fn declined_and_cancelled_elicitation_explain_that_nothing_was_executed() {
    for kind in [ApprovalKind::Destructive, ApprovalKind::Update] {
        let declined = map_elicitation_failure(kind, ElicitationError::UserDeclined);
        assert_eq!(declined.code, ERROR_CODE_APPROVAL_DECLINED);
        assert!(declined.message.contains("用户拒绝确认"));
        assert!(declined.message.contains("未执行"));

        let cancelled = map_elicitation_failure(kind, ElicitationError::UserCancelled);
        assert_eq!(cancelled.code, ERROR_CODE_APPROVAL_CANCELLED);
        assert!(cancelled.message.contains("用户取消确认"));
        assert!(cancelled.message.contains("未执行"));
    }
}

#[test]
fn exposes_the_complete_tool_set() {
    let tools = AstesiaMcp::tool_router().list_all();
    let names = tools
        .iter()
        .map(|tool| tool.name.as_ref())
        .collect::<Vec<_>>();

    assert_eq!(tools.len(), 18);
    for expected in [
        "list_connections",
        "test_connection",
        "connect_connection",
        "disconnect_connection",
        "create_database_object",
        "delete_database_object",
        "create_schema",
        "delete_schema",
        "create_table",
        "delete_table",
        "list_queries",
        "create_query",
        "execute_query",
        "delete_query",
        "read_rows",
        "insert_row",
        "update_row",
        "delete_rows",
    ] {
        assert!(names.contains(&expected), "missing MCP tool {expected}");
    }
    assert!(!names.contains(&"create_connection"));
    assert!(!names.contains(&"delete_connection"));
}

#[test]
fn marks_high_risk_tools_as_destructive() {
    let tools = AstesiaMcp::tool_router().list_all();

    for name in [
        "delete_database_object",
        "delete_schema",
        "delete_table",
        "execute_query",
        "delete_query",
        "update_row",
        "delete_rows",
    ] {
        let tool = tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("missing MCP tool {name}"));
        let annotations = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("missing annotations for {name}"));
        assert_eq!(
            annotations.destructive_hint,
            Some(true),
            "{name} must advertise destructive behavior"
        );
    }
}

#[test]
fn validates_http_auth_tokens() {
    assert!(validate_http_auth_token(&"a".repeat(32)).is_ok());
    assert!(validate_http_auth_token(&format!("{}-_.~", "b".repeat(32))).is_ok());
    assert!(validate_http_auth_token("short").is_err());
    assert!(validate_http_auth_token(&"a".repeat(MAX_TOKEN_BYTES + 1)).is_err());
    assert!(validate_http_auth_token(&format!("{} token", "a".repeat(32))).is_err());
}

#[test]
fn compares_authorization_values_without_prefix_or_length_shortcuts() {
    assert!(constant_time_eq(b"Bearer secret", b"Bearer secret"));
    assert!(!constant_time_eq(b"Bearer secret", b"Bearer other!"));
    assert!(!constant_time_eq(b"Bearer secret", b"Bearer secret-longer"));
}

#[test]
fn selectors_allow_quoted_identifiers_but_reject_invalid_shape() {
    assert!(
        AstesiaMcp::validate_database_selector(&DbType::MySQL, "analytics` ; DROP DATABASE x")
            .is_ok()
    );
    assert!(AstesiaMcp::validate_database_selector(
        &DbType::SQLServer,
        "analytics] ; DROP DATABASE x"
    )
    .is_ok());
    assert!(AstesiaMcp::validate_read_table("users\"; DELETE FROM users").is_ok());
    assert!(AstesiaMcp::validate_read_table("odd'table").is_ok());

    assert!(AstesiaMcp::validate_database_selector(&DbType::MySQL, "").is_err());
    assert!(AstesiaMcp::validate_database_selector(&DbType::SQLite, "").is_ok());
    assert!(AstesiaMcp::validate_read_table("").is_err());
    assert!(AstesiaMcp::validate_read_table("line\nbreak").is_err());
    assert!(AstesiaMcp::validate_read_table(&"x".repeat(MAX_SELECTOR_BYTES + 1)).is_err());
}

#[test]
fn bounds_page_offsets_before_the_driver_multiplies_them() {
    let page = AstesiaMcp::bounded_page(Some(u32::MAX), MAX_RESULT_ROWS as u32);
    assert!((page - 1).checked_mul(MAX_RESULT_ROWS as u32).is_some());
}

#[test]
fn rejects_credential_bearing_permission_sql() {
    let sql = "CREATE USER ada WITH PASSWORD 'do-not-store-this'";
    let analysis = AstesiaMcp::analyze_sql(&DbType::PostgreSQL, sql);
    assert!(AstesiaMcp::validate_no_credential_sql(&DbType::PostgreSQL, sql, &analysis).is_err());
}

#[tokio::test]
async fn active_test_guard_drop_unblocks_controls_and_clears_the_weak_marker() {
    let active_tests = ActiveConnectionTests::default();
    let active_test = active_tests
        .register("connection-a", 17, false, None)
        .expect("register test");
    let state = active_tests
        .current("connection-a")
        .expect("active test state");
    let waiting_state = state.clone();
    let waiter = tokio::spawn(async move {
        waiting_state.cancel();
        waiting_state.wait_until_future_dropped().await;
        waiting_state.generation
    });

    tokio::time::timeout(Duration::from_secs(1), state.cancellation.cancelled())
        .await
        .expect("cancellation observed");
    tokio::task::yield_now().await;
    assert!(
        !waiter.is_finished(),
        "control must wait until the database test future is dropped"
    );

    drop(state);
    drop(active_test);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("control waiter completed")
            .expect("control waiter task"),
        17
    );
    assert!(
        active_tests.current("connection-a").is_none(),
        "a cancelled handler future must not leave a stale active-test marker"
    );
}

#[test]
fn active_test_marker_survives_until_the_handler_finishes_sync_release() {
    let active_tests = ActiveConnectionTests::default();
    let active_test = active_tests
        .register("connection-a", 23, true, None)
        .expect("register test");

    active_test.mark_future_dropped();
    assert!(
        active_tests.current("connection-a").is_some(),
        "owned generation remains visible while Released is in flight"
    );
    drop(active_test);
    assert!(active_tests.current("connection-a").is_none());

    active_tests
        .register("connection-a", 24, false, None)
        .expect("a completed handler must allow another test");
}

#[test]
fn delayed_release_marker_prevents_generation_aba() {
    let active_tests = ActiveConnectionTests::default();
    let active_test = active_tests
        .register("connection-a", 31, true, None)
        .expect("register test");
    let marker = ActiveTestMarker::new(active_test.test.clone());

    active_test.mark_future_dropped();
    drop(active_test);
    assert!(
        active_tests.current("connection-a").is_some(),
        "the delayed Released task must keep connect/test blocked"
    );
    assert!(
        active_tests
            .register("connection-a", 32, false, None)
            .is_err(),
        "a new generation must not start before old Released completes"
    );

    drop(marker);
    assert!(active_tests.current("connection-a").is_none());
    active_tests
        .register("connection-a", 32, false, None)
        .expect("new generation after Released");
}

#[tokio::test]
async fn cancelled_pending_sync_lease_releases_its_generation() {
    let (events, mut recorded) = mpsc::unbounded_channel();
    let sync = Arc::new(RecordingSyncLeaseClient {
        events,
        connected_error: None,
        release_gate: None,
    });
    let lease = PendingSyncLease::with_client(sync, "connection-a".to_string(), 41);
    let task = tokio::spawn(async move {
        let _lease = lease;
        std::future::pending::<()>().await;
    });

    tokio::task::yield_now().await;
    task.abort();
    let _ = task.await;

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), recorded.recv())
            .await
            .expect("release scheduled after cancellation"),
        Some(SyncLeaseEvent::Released("connection-a".to_string(), 41))
    );
}

#[tokio::test]
async fn panicking_pending_sync_lease_releases_its_generation() {
    let (events, mut recorded) = mpsc::unbounded_channel();
    let sync = Arc::new(RecordingSyncLeaseClient {
        events,
        connected_error: None,
        release_gate: None,
    });
    let lease = PendingSyncLease::with_client(sync, "connection-a".to_string(), 45);
    let task = tokio::spawn(async move {
        let _lease = lease;
        panic!("cancel the pending transition");
    });

    assert!(task.await.expect_err("task must panic").is_panic());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), recorded.recv())
            .await
            .expect("release scheduled while unwinding"),
        Some(SyncLeaseEvent::Released("connection-a".to_string(), 45))
    );
}

#[tokio::test]
async fn only_a_successful_connected_transition_commits_a_sync_lease() {
    let (events, mut recorded) = mpsc::unbounded_channel();
    let sync = Arc::new(RecordingSyncLeaseClient {
        events,
        connected_error: None,
        release_gate: None,
    });
    let mut lease = PendingSyncLease::with_client(sync, "connection-a".to_string(), 42);

    lease
        .commit_connected()
        .await
        .expect("connected transition");
    assert_eq!(
        recorded.recv().await,
        Some(SyncLeaseEvent::Connected("connection-a".to_string(), 42))
    );
    drop(lease);
    tokio::task::yield_now().await;
    assert!(
        recorded.try_recv().is_err(),
        "committed lease must not release"
    );

    let (events, mut recorded) = mpsc::unbounded_channel();
    let sync = Arc::new(RecordingSyncLeaseClient {
        events,
        connected_error: Some("connected rejected".to_string()),
        release_gate: None,
    });
    let mut lease = PendingSyncLease::with_client(sync, "connection-b".to_string(), 43);
    assert_eq!(
        lease.commit_connected().await,
        Err("connected rejected".to_string())
    );
    assert_eq!(
        recorded.recv().await,
        Some(SyncLeaseEvent::Connected("connection-b".to_string(), 43))
    );
    drop(lease);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), recorded.recv())
            .await
            .expect("failed transition keeps lease pending"),
        Some(SyncLeaseEvent::Released("connection-b".to_string(), 43))
    );
}

#[tokio::test]
async fn successful_explicit_release_finishes_without_a_drop_retry() {
    let (events, mut recorded) = mpsc::unbounded_channel();
    let sync = Arc::new(RecordingSyncLeaseClient {
        events,
        connected_error: None,
        release_gate: None,
    });
    let mut lease = PendingSyncLease::with_client(sync, "connection-a".to_string(), 44);

    lease.release().await.expect("explicit Released transition");
    assert_eq!(
        recorded.recv().await,
        Some(SyncLeaseEvent::Released("connection-a".to_string(), 44))
    );
    drop(lease);
    tokio::task::yield_now().await;
    assert!(
        recorded.try_recv().is_err(),
        "successful release must not be retried by Drop"
    );
}

#[tokio::test]
async fn abandoned_test_lease_keeps_aba_marker_until_release_finishes() {
    let active_tests = ActiveConnectionTests::default();
    let (events, mut recorded) = mpsc::unbounded_channel();
    let release_gate = Arc::new(Notify::new());
    let sync = Arc::new(RecordingSyncLeaseClient {
        events,
        connected_error: None,
        release_gate: Some(release_gate.clone()),
    });
    let lease = PendingSyncLease::with_client(sync, "connection-a".to_string(), 51);
    let active_test = active_tests
        .register("connection-a", 51, true, Some(lease))
        .expect("register owned test");

    drop(active_test);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), recorded.recv())
            .await
            .expect("delayed release started"),
        Some(SyncLeaseEvent::Released("connection-a".to_string(), 51))
    );
    assert!(active_tests.current("connection-a").is_some());
    assert!(
        active_tests
            .register("connection-a", 52, false, None)
            .is_err(),
        "old generation marker must block a new test"
    );

    release_gate.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        while active_tests.current("connection-a").is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("marker cleared after Released completed");
    active_tests
        .register("connection-a", 52, false, None)
        .expect("new generation after Released");
}
