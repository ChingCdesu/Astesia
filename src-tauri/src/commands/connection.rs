use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_shell::{process::CommandEvent, ShellExt};

use crate::connection_repository::{
    ConnectionProfilesSnapshot, ConnectionRepositoryError, ConnectionRepositoryErrorCode,
    CredentialVerificationReport, CredentialVerificationScope, DeleteConnectionResult,
    LegacyMigrationResult, SaveConnectionRequest, SharedConnectionProfile,
};
use crate::db::{ConnectionConfig, DatabaseDriver, DbType};
use crate::mcp::CREDENTIAL_VERIFY_MARKER;
use crate::state::{create_driver, AppState};

const MCP_SIDECAR_NAME: &str = "astesia-mcp";
const CREDENTIAL_VERIFY_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_CREDENTIAL_VERIFY_OUTPUT: usize = 16_384;

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionResult {
    pub success: bool,
    pub message: String,
}

#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    config: ConnectionConfig,
) -> Result<ConnectionResult, String> {
    let config = if config.password.is_empty() {
        match state.connection_repository.get(&config.id).await {
            Ok(_) => state
                .connection_repository
                .resolve_matching_config(&config)
                .await
                .map_err(|error| error.to_string())?,
            Err(error) if error.code == ConnectionRepositoryErrorCode::ProfileNotFound => config,
            Err(error) => return Err(error.to_string()),
        }
    } else {
        config
    };
    let driver = create_driver(&config);
    match driver.test_connection().await {
        Ok(_) => Ok(ConnectionResult {
            success: true,
            message: "连接成功".to_string(),
        }),
        Err(e) => Ok(ConnectionResult {
            success: false,
            message: format!("连接失败: {}", e),
        }),
    }
}

#[tauri::command]
pub async fn connect_database(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<ConnectionResult, String> {
    let (config, revision) = state
        .connection_repository
        .resolve_config(&connection_id)
        .await
        .map_err(|error| error.to_string())?;
    let mut driver = create_driver(&config);
    match driver.connect().await {
        Ok(_) => {
            let coordinator = state.shared_driver_coordinator.lock().await;
            let current = state.connection_repository.get(&config.id).await;
            match current {
                Ok(profile) if profile.revision == revision => {}
                Ok(_) => {
                    drop(coordinator);
                    let _ = driver.disconnect().await;
                    return Ok(ConnectionResult {
                        success: false,
                        message: "连接配置在建立连接期间已被修改，请刷新后重试".to_string(),
                    });
                }
                Err(error) => {
                    drop(coordinator);
                    let _ = driver.disconnect().await;
                    return Err(error.to_string());
                }
            }

            let replaced = {
                // Lock order is coordinator -> connections -> revisions.
                let mut connections = state.connections.lock().await;
                let mut revisions = state.shared_driver_revisions.lock().await;
                let replaced = connections.insert(config.id.clone(), driver);
                revisions.insert(config.id.clone(), revision);
                replaced
            };
            drop(coordinator);
            if let Some(mut replaced) = replaced {
                let _ = replaced.disconnect().await;
            }
            Ok(ConnectionResult {
                success: true,
                message: "连接成功".to_string(),
            })
        }
        Err(e) => Ok(ConnectionResult {
            success: false,
            message: format!("连接失败: {}", e),
        }),
    }
}

#[tauri::command]
pub async fn list_connection_profiles(
    state: State<'_, AppState>,
) -> Result<Vec<SharedConnectionProfile>, ConnectionRepositoryError> {
    Ok(snapshot_and_reconcile(&state).await?.profiles)
}

#[tauri::command]
pub async fn connection_profiles_snapshot(
    state: State<'_, AppState>,
) -> Result<ConnectionProfilesSnapshot, ConnectionRepositoryError> {
    snapshot_and_reconcile(&state).await
}

#[tauri::command]
pub async fn shared_connections_revision(
    state: State<'_, AppState>,
) -> Result<i64, ConnectionRepositoryError> {
    state.connection_repository.current_revision().await
}

#[tauri::command]
pub async fn save_connection_profile(
    state: State<'_, AppState>,
    request: SaveConnectionRequest,
) -> Result<SharedConnectionProfile, ConnectionRepositoryError> {
    let coordinator = state.shared_driver_coordinator.lock().await;
    let profile = state.connection_repository.save(request).await?;
    let driver = detach_shared_driver(&state, &profile.id).await;
    drop(coordinator);
    disconnect_driver(driver).await;
    Ok(profile)
}

#[tauri::command]
pub async fn delete_connection_profile(
    state: State<'_, AppState>,
    connection_id: String,
    expected_revision: i64,
) -> Result<DeleteConnectionResult, ConnectionRepositoryError> {
    let coordinator = state.shared_driver_coordinator.lock().await;
    let result = state
        .connection_repository
        .delete(&connection_id, expected_revision)
        .await?;
    let driver = detach_shared_driver(&state, &connection_id).await;
    drop(coordinator);
    disconnect_driver(driver).await;
    Ok(result)
}

#[tauri::command]
pub async fn migrate_legacy_connections(
    app: AppHandle,
    state: State<'_, AppState>,
    connections: Vec<ConnectionConfig>,
) -> Result<LegacyMigrationResult, ConnectionRepositoryError> {
    let result = state
        .connection_repository
        .migrate_legacy(connections)
        .await?;
    state
        .connection_repository
        .migrate_all_credential_storage()
        .await?;
    let expected_scope = state
        .connection_repository
        .credential_verification_scope()
        .await?;
    ensure_migration_revision(result.revision, &expected_scope, "准备 sidecar 凭据验证")?;
    verify_sidecar_credential_access(&app, &expected_scope).await?;
    let actual_scope = state
        .connection_repository
        .credential_verification_scope()
        .await?;
    ensure_migration_revision(result.revision, &actual_scope, "完成 sidecar 凭据验证")?;
    if actual_scope != expected_scope {
        return Err(ConnectionRepositoryError::verification_scope_changed(
            &expected_scope,
            &actual_scope,
        ));
    }
    Ok(result)
}

fn ensure_migration_revision(
    migration_revision: i64,
    scope: &CredentialVerificationScope,
    stage: &str,
) -> Result<(), ConnectionRepositoryError> {
    if scope.repository_revision != migration_revision {
        return Err(ConnectionRepositoryError::migration_revision_changed(
            migration_revision,
            scope.repository_revision,
            stage,
        ));
    }
    Ok(())
}

async fn verify_sidecar_credential_access(
    app: &AppHandle,
    expected_scope: &CredentialVerificationScope,
) -> Result<(), ConnectionRepositoryError> {
    let command = app
        .shell()
        .sidecar(MCP_SIDECAR_NAME)
        .map_err(|error| {
            ConnectionRepositoryError::credential_verifier_unavailable(format!(
                "无法解析打包的 astesia-mcp 凭据验证程序：{error}"
            ))
        })?
        .arg("--verify-shared-credentials");
    let (mut events, child) = command.spawn().map_err(|error| {
        ConnectionRepositoryError::credential_verifier_unavailable(format!(
            "无法启动打包的 astesia-mcp 凭据验证程序：{error}"
        ))
    })?;
    let mut child = Some(child);
    let deadline = tokio::time::Instant::now() + CREDENTIAL_VERIFY_TIMEOUT;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = None;

    loop {
        let event = match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(event) => event,
            Err(_) => {
                if let Some(child) = child.take() {
                    let _ = child.kill();
                }
                return Err(ConnectionRepositoryError::credential_verifier_unavailable(
                    "astesia-mcp 在 120 秒内未完成系统凭据库验证；操作系统授权提示可能未被处理",
                ));
            }
        };
        match event {
            Some(CommandEvent::Stdout(bytes)) => {
                append_bounded(&mut stdout, &bytes);
                append_bounded(&mut stdout, b"\n");
            }
            Some(CommandEvent::Stderr(bytes)) => {
                append_bounded(&mut stderr, &bytes);
                append_bounded(&mut stderr, b"\n");
            }
            Some(CommandEvent::Error(error)) => {
                append_bounded(&mut stderr, error.as_bytes());
                append_bounded(&mut stderr, b"\n");
            }
            Some(CommandEvent::Terminated(payload)) => {
                exit_code = payload.code;
                break;
            }
            Some(_) => {}
            None => break,
        }
    }
    drop(child.take());

    if exit_code != Some(0) {
        let detail = String::from_utf8_lossy(&stderr).trim().to_string();
        return Err(ConnectionRepositoryError::credential_verifier_unavailable(
            if detail.is_empty() {
                format!("astesia-mcp 凭据验证进程异常退出：{:?}", exit_code)
            } else {
                format!("astesia-mcp 凭据验证进程异常退出：{detail}")
            },
        ));
    }

    let stdout = String::from_utf8_lossy(&stdout);
    let report = stdout
        .lines()
        .find_map(|line| line.strip_prefix(CREDENTIAL_VERIFY_MARKER))
        .and_then(|json| serde_json::from_str::<CredentialVerificationReport>(json).ok());
    let Some(report) = report else {
        let detail = String::from_utf8_lossy(&stderr).trim().to_string();
        let detail = if detail.is_empty() {
            format!("进程退出码为 {:?}", exit_code)
        } else {
            detail
        };
        return Err(ConnectionRepositoryError::credential_verifier_unavailable(
            format!("astesia-mcp 未返回有效的凭据验证结果：{detail}"),
        ));
    };

    validate_credential_verification_report(report, expected_scope)
}

fn validate_credential_verification_report(
    report: CredentialVerificationReport,
    expected_scope: &CredentialVerificationScope,
) -> Result<(), ConnectionRepositoryError> {
    match (report.ok, report.scope, report.error) {
        (true, Some(scope), None)
            if report.verified == scope.credential_count && &scope == expected_scope =>
        {
            Ok(())
        }
        (true, Some(scope), None) => Err(
            ConnectionRepositoryError::credential_verifier_unavailable(format!(
                "astesia-mcp 验证范围与 App 不一致：预期仓库 {}、revision {}、{} 个连接（{} 个凭据），实际仓库 {}、revision {}、{} 个连接（{} 个凭据）",
                expected_scope.repository_id,
                expected_scope.repository_revision,
                expected_scope.profile_count,
                expected_scope.credential_count,
                scope.repository_id,
                scope.repository_revision,
                scope.profile_count,
                scope.credential_count,
            )),
        ),
        (false, _, Some(error)) => Err(error),
        _ => Err(ConnectionRepositoryError::credential_verifier_unavailable(
            "astesia-mcp 返回了不一致的凭据验证结果",
        )),
    }
}

fn append_bounded(target: &mut Vec<u8>, bytes: &[u8]) {
    let remaining = MAX_CREDENTIAL_VERIFY_OUTPUT.saturating_sub(target.len());
    target.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
}

#[tauri::command]
pub async fn disconnect_database(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<ConnectionResult, String> {
    let coordinator = state.shared_driver_coordinator.lock().await;
    let driver = detach_shared_driver(&state, &connection_id).await;
    drop(coordinator);
    if driver.is_some() {
        disconnect_driver(driver).await;
        Ok(ConnectionResult {
            success: true,
            message: "已断开连接".to_string(),
        })
    } else {
        Ok(ConnectionResult {
            success: false,
            message: "连接不存在".to_string(),
        })
    }
}

async fn snapshot_and_reconcile(
    state: &AppState,
) -> Result<ConnectionProfilesSnapshot, ConnectionRepositoryError> {
    let coordinator = state.shared_driver_coordinator.lock().await;
    let snapshot = state.connection_repository.snapshot().await?;
    let drivers = detach_stale_shared_drivers(state, &snapshot.profiles).await;
    drop(coordinator);
    for driver in drivers {
        disconnect_driver(Some(driver)).await;
    }
    Ok(snapshot)
}

async fn detach_stale_shared_drivers(
    state: &AppState,
    profiles: &[SharedConnectionProfile],
) -> Vec<Box<dyn DatabaseDriver>> {
    // Lock order is coordinator (held by caller) -> connections -> revisions.
    let mut connections = state.connections.lock().await;
    let mut revisions = state.shared_driver_revisions.lock().await;
    let stale_ids = stale_shared_driver_ids(&revisions, profiles)
        .into_iter()
        .chain(
            revisions
                .keys()
                .filter(|connection_id| !connections.contains_key(*connection_id))
                .cloned(),
        )
        .collect::<std::collections::HashSet<_>>();

    let mut drivers = Vec::with_capacity(stale_ids.len());
    for connection_id in stale_ids {
        revisions.remove(&connection_id);
        if let Some(driver) = connections.remove(&connection_id) {
            drivers.push(driver);
        }
    }
    drivers
}

fn stale_shared_driver_ids(
    revisions: &HashMap<String, i64>,
    profiles: &[SharedConnectionProfile],
) -> Vec<String> {
    let current_revisions = profiles
        .iter()
        .map(|profile| (profile.id.as_str(), profile.revision))
        .collect::<HashMap<_, _>>();
    revisions
        .iter()
        .filter_map(|(connection_id, connected_revision)| {
            (current_revisions.get(connection_id.as_str()) != Some(connected_revision))
                .then(|| connection_id.clone())
        })
        .collect()
}

async fn detach_shared_driver(
    state: &AppState,
    connection_id: &str,
) -> Option<Box<dyn DatabaseDriver>> {
    // Lock order is coordinator (held by caller) -> connections -> revisions.
    let mut connections = state.connections.lock().await;
    let mut revisions = state.shared_driver_revisions.lock().await;
    revisions.remove(connection_id);
    connections.remove(connection_id)
}

async fn disconnect_driver(driver: Option<Box<dyn DatabaseDriver>>) {
    if let Some(mut driver) = driver {
        let _ = driver.disconnect().await;
    }
}

#[tauri::command]
pub async fn get_default_port(db_type: DbType) -> Result<u16, String> {
    let port = match db_type {
        DbType::MySQL => 3306,
        DbType::PostgreSQL => 5432,
        DbType::SQLite => 0,
        DbType::SQLServer => 1433,
        DbType::MongoDB => 27017,
        DbType::Redis => 6379,
    };
    Ok(port)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    fn profile(id: &str, revision: i64) -> SharedConnectionProfile {
        SharedConnectionProfile {
            id: id.to_string(),
            name: id.to_string(),
            db_type: DbType::SQLite,
            host: ":memory:".to_string(),
            port: 0,
            username: String::new(),
            database: None,
            color: None,
            has_credential: false,
            revision,
            mcp_enabled: true,
        }
    }

    #[test]
    fn reconciliation_invalidates_deleted_and_revision_changed_shared_drivers() {
        let revisions = HashMap::from([
            ("current".to_string(), 4),
            ("changed".to_string(), 7),
            ("deleted".to_string(), 9),
        ]);
        let profiles = vec![profile("current", 4), profile("changed", 8)];

        let stale = stale_shared_driver_ids(&revisions, &profiles)
            .into_iter()
            .collect::<HashSet<_>>();

        assert_eq!(
            stale,
            HashSet::from(["changed".to_string(), "deleted".to_string()])
        );
    }

    #[test]
    fn sidecar_report_must_match_repository_revision_profiles_credentials_and_digest() {
        let expected = CredentialVerificationScope {
            repository_id: "92e8d876-0695-4fd6-92a3-f45fe72fe330".to_string(),
            repository_revision: 7,
            profile_count: 3,
            credential_count: 2,
            profile_digest: "expected".to_string(),
        };
        assert!(validate_credential_verification_report(
            CredentialVerificationReport::success(expected.clone()),
            &expected,
        )
        .is_ok());
        let mut wrong_count = CredentialVerificationReport::success(expected.clone());
        wrong_count.verified = 1;
        assert!(validate_credential_verification_report(wrong_count, &expected).is_err());

        for mismatched in [
            CredentialVerificationScope {
                repository_id: "33d0a87b-2dd4-4ce4-af91-7841a649d24a".to_string(),
                ..expected.clone()
            },
            CredentialVerificationScope {
                repository_revision: 8,
                ..expected.clone()
            },
            CredentialVerificationScope {
                profile_count: 4,
                ..expected.clone()
            },
            CredentialVerificationScope {
                credential_count: 1,
                ..expected.clone()
            },
            CredentialVerificationScope {
                profile_digest: "different".to_string(),
                ..expected.clone()
            },
        ] {
            assert!(validate_credential_verification_report(
                CredentialVerificationReport::success(mismatched),
                &expected,
            )
            .is_err());
        }
    }

    #[test]
    fn legacy_migration_revision_must_match_every_sidecar_scope() {
        let scope = CredentialVerificationScope {
            repository_id: "92e8d876-0695-4fd6-92a3-f45fe72fe330".to_string(),
            repository_revision: 8,
            profile_count: 1,
            credential_count: 1,
            profile_digest: "scope".to_string(),
        };
        assert!(ensure_migration_revision(8, &scope, "test").is_ok());

        let error = ensure_migration_revision(7, &scope, "test")
            .expect_err("a different migration revision must keep legacy data");
        assert_eq!(
            error.code,
            ConnectionRepositoryErrorCode::MigrationIncomplete
        );
        assert!(error.retryable);
        assert_eq!(error.details["migration_revision"], 7);
        assert_eq!(error.details["actual_revision"], 8);
    }
}
