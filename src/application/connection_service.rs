use std::collections::HashMap;

use serde::Serialize;

use super::connections::ConnectionManager;
use crate::connection_repository::{
    ConnectionRepositoryError, DeleteConnectionResult, SharedConnectionProfile,
};
use crate::db::ConnectionConfig;
use crate::mcp_sync_server::{
    ForceDisconnectError, ForceDisconnectResult, McpConnectionSnapshot, McpSyncRegistry,
};

use super::{ConnectionOutcome, ValidatedProfile};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct DatabaseSessionSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
}

impl DatabaseSessionSnapshot {
    fn from_generation(generation: Option<u64>) -> Self {
        Self { generation }
    }

    pub const fn is_connected(self) -> bool {
        self.generation.is_some()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConnectionProfileSnapshot {
    pub profile: SharedConnectionProfile,
    pub session: DatabaseSessionSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_usage: Option<McpConnectionSnapshot>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConnectionWorkspaceSnapshot {
    pub repository_revision: i64,
    pub mcp_revision: u64,
    pub profiles: Vec<ConnectionProfileSnapshot>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LoadedDatabases {
    pub session_generation: u64,
    pub databases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ManagedMcpDisconnect {
    Completed {
        requested: usize,
        completed: usize,
    },
    Failed {
        requested: usize,
        completed: usize,
        error: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ExternalMcpUsage {
    NotChecked,
    NotInUse,
    StillInUse,
    Unknown { error: ConnectionRepositoryError },
}

#[derive(Debug, Clone, Serialize)]
pub struct DisconnectReport {
    pub connection_id: String,
    pub local_session_disconnected: bool,
    pub managed_mcp: ManagedMcpDisconnect,
    pub external_mcp: ExternalMcpUsage,
}

#[derive(Debug, Clone)]
pub enum ProfileOperationCommand {
    Connect {
        connection_id: String,
    },
    Disconnect {
        connection_id: String,
    },
    Delete {
        connection_id: String,
        expected_revision: i64,
    },
}

impl ProfileOperationCommand {
    pub fn connection_id(&self) -> &str {
        match self {
            Self::Connect { connection_id }
            | Self::Disconnect { connection_id }
            | Self::Delete { connection_id, .. } => connection_id,
        }
    }

    pub const fn operation_name(&self) -> &'static str {
        match self {
            Self::Connect { .. } => "连接",
            Self::Disconnect { .. } => "断开",
            Self::Delete { .. } => "删除",
        }
    }
}

#[derive(Debug)]
pub enum ProfileOperationOutcome {
    Connected(Result<ConnectionOutcome, String>),
    Disconnected(DisconnectReport),
    Deleted(Result<DeleteConnectionResult, ConnectionRepositoryError>),
}

#[derive(Debug)]
pub struct ProfileOperationCompletion {
    pub outcome: ProfileOperationOutcome,
    pub snapshot: Result<ConnectionWorkspaceSnapshot, ConnectionRepositoryError>,
}

impl DisconnectReport {
    pub fn message(&self) -> String {
        let completed = match &self.managed_mcp {
            ManagedMcpDisconnect::Completed { completed, .. } => *completed,
            ManagedMcpDisconnect::Failed {
                requested,
                completed,
                error,
            } => {
                let failure = format!(
                    "无法断开 Streamable HTTP MCP 对连接 {} 的全部占用（已完成 {completed}/{requested}）: {error}",
                    self.connection_id
                );
                if self.local_session_disconnected || *completed > 0 {
                    return format!(
                        "{}，但{failure}",
                        disconnect_progress_message(self.local_session_disconnected, *completed)
                    );
                }
                return failure;
            }
        };

        match &self.external_mcp {
            ExternalMcpUsage::NotInUse => {
                disconnect_progress_message(self.local_session_disconnected, completed)
            }
            ExternalMcpUsage::StillInUse => {
                disconnect_external_in_use_message(self.local_session_disconnected, completed)
            }
            ExternalMcpUsage::Unknown { error } => {
                let progress =
                    disconnect_progress_message(self.local_session_disconnected, completed);
                format!(
                    "{progress}，但无法确认连接 {} 是否仍被 STDIO 或其他外部 MCP 使用: {error}",
                    self.connection_id
                )
            }
            ExternalMcpUsage::NotChecked => format!(
                "无法确认连接 {} 是否仍被 STDIO 或其他外部 MCP 使用",
                self.connection_id
            ),
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(&self.managed_mcp, ManagedMcpDisconnect::Completed { .. })
            && matches!(&self.external_mcp, ExternalMcpUsage::NotInUse)
    }
}

#[derive(Clone)]
pub struct ConnectionService {
    manager: ConnectionManager,
    mcp_registry: McpSyncRegistry,
}

impl ConnectionService {
    pub(super) fn new(manager: ConnectionManager, mcp_registry: McpSyncRegistry) -> Self {
        Self {
            manager,
            mcp_registry,
        }
    }

    pub async fn snapshot(&self) -> Result<ConnectionWorkspaceSnapshot, ConnectionRepositoryError> {
        let (repository, mcp) = tokio::join!(
            self.manager.snapshot_with_session_generations(),
            self.mcp_registry.snapshot(),
        );
        let (repository, mut session_generations) = repository?;
        let mut mcp_connections = mcp
            .connections
            .into_iter()
            .map(|connection| (connection.id.clone(), connection))
            .collect::<HashMap<_, _>>();
        let profiles = repository
            .profiles
            .into_iter()
            .map(|profile| {
                let connection_id = &profile.id;
                ConnectionProfileSnapshot {
                    session: DatabaseSessionSnapshot::from_generation(
                        session_generations.remove(connection_id),
                    ),
                    mcp_usage: mcp_connections.remove(connection_id),
                    profile,
                }
            })
            .collect();

        Ok(ConnectionWorkspaceSnapshot {
            repository_revision: repository.revision,
            mcp_revision: mcp.revision,
            profiles,
        })
    }

    pub async fn test_connection(
        &self,
        config: ConnectionConfig,
    ) -> Result<ConnectionOutcome, String> {
        self.manager.test_connection(config).await
    }

    pub async fn connect(&self, connection_id: &str) -> Result<ConnectionOutcome, String> {
        self.manager.connect(connection_id).await
    }

    pub async fn save_profile(
        &self,
        profile: ValidatedProfile,
    ) -> Result<SharedConnectionProfile, ConnectionRepositoryError> {
        let request = profile.into_request();
        let connection_id = request.config.id.clone();
        // This guard closes the acquire-versus-mutation gap until repository
        // mutation and local session retirement have both completed.
        let _mcp_guard = self
            .mcp_registry
            .lock_connection_lifecycle(&connection_id)
            .await;
        self.ensure_managed_mcp_released(&connection_id).await?;
        self.manager.save_profile(request).await
    }

    pub async fn delete_profile(
        &self,
        connection_id: &str,
        expected_revision: i64,
    ) -> Result<DeleteConnectionResult, ConnectionRepositoryError> {
        let _mcp_guard = self
            .mcp_registry
            .lock_connection_lifecycle(connection_id)
            .await;
        self.ensure_managed_mcp_released(connection_id).await?;
        self.manager
            .delete_profile(connection_id, expected_revision)
            .await
    }

    pub async fn disconnect(&self, connection_id: &str) -> DisconnectReport {
        let (local_session_disconnected, managed_result) = tokio::join!(
            self.manager.disconnect_local(connection_id),
            self.mcp_registry.force_disconnect(connection_id),
        );

        match managed_result {
            Ok(result) => DisconnectReport {
                connection_id: connection_id.to_string(),
                local_session_disconnected,
                managed_mcp: managed_disconnect_completed(result),
                // App-managed sessions must release their cross-process lease
                // before this probe can identify remaining external users.
                external_mcp: match self.manager.is_connection_externally_in_use(connection_id) {
                    Ok(true) => ExternalMcpUsage::StillInUse,
                    Ok(false) => ExternalMcpUsage::NotInUse,
                    Err(error) => ExternalMcpUsage::Unknown { error },
                },
            },
            Err(error) => DisconnectReport {
                connection_id: connection_id.to_string(),
                local_session_disconnected,
                managed_mcp: managed_disconnect_failed(error),
                external_mcp: ExternalMcpUsage::NotChecked,
            },
        }
    }

    pub async fn perform_profile_operation(
        &self,
        command: ProfileOperationCommand,
    ) -> ProfileOperationCompletion {
        let outcome = match command {
            ProfileOperationCommand::Connect { connection_id } => {
                ProfileOperationOutcome::Connected(self.connect(&connection_id).await)
            }
            ProfileOperationCommand::Disconnect { connection_id } => {
                ProfileOperationOutcome::Disconnected(self.disconnect(&connection_id).await)
            }
            ProfileOperationCommand::Delete {
                connection_id,
                expected_revision,
            } => ProfileOperationOutcome::Deleted(
                self.delete_profile(&connection_id, expected_revision).await,
            ),
        };
        let snapshot = self.snapshot().await;
        ProfileOperationCompletion { outcome, snapshot }
    }

    pub async fn load_databases(&self, connection_id: &str) -> Result<LoadedDatabases, String> {
        let (session_generation, databases) = self.manager.load_databases(connection_id).await?;
        Ok(LoadedDatabases {
            session_generation,
            databases,
        })
    }

    async fn ensure_managed_mcp_released(
        &self,
        connection_id: &str,
    ) -> Result<(), ConnectionRepositoryError> {
        if self.mcp_registry.is_connection_in_use(connection_id).await {
            Err(ConnectionRepositoryError::connection_in_use(connection_id))
        } else {
            Ok(())
        }
    }
}

fn managed_disconnect_completed(result: ForceDisconnectResult) -> ManagedMcpDisconnect {
    ManagedMcpDisconnect::Completed {
        requested: result.requested,
        completed: result.completed,
    }
}

fn managed_disconnect_failed(error: ForceDisconnectError) -> ManagedMcpDisconnect {
    ManagedMcpDisconnect::Failed {
        requested: error.requested,
        completed: error.completed,
        error: error.error,
    }
}

fn disconnect_progress_message(app_disconnected: bool, mcp_disconnected: usize) -> String {
    match (app_disconnected, mcp_disconnected) {
        (true, 0) => "已断开 App 连接".to_string(),
        (true, count) => format!("已断开 App 连接及 {count} 个 Streamable HTTP MCP 会话"),
        (false, 0) => "连接当前未连接".to_string(),
        (false, count) => format!("已断开 {count} 个 Streamable HTTP MCP 会话"),
    }
}

fn disconnect_external_in_use_message(app_disconnected: bool, mcp_disconnected: usize) -> String {
    let progress = match (app_disconnected, mcp_disconnected) {
        (true, 0) => Some("已断开 App 连接".to_string()),
        (true, count) => Some(format!(
            "已断开 App 连接及 {count} 个 Streamable HTTP MCP 会话"
        )),
        (false, 0) => None,
        (false, count) => Some(format!("已断开 {count} 个 Streamable HTTP MCP 会话")),
    };
    let prefix = progress
        .map(|progress| format!("{progress}；"))
        .unwrap_or_default();
    format!(
        "{prefix}仍有 STDIO 或其他外部 MCP 进程占用该连接。Astesia 无法向 STDIO 推送强制断开；请在对应 MCP 客户端调用 disconnect_connection，或关闭该 STDIO 进程。"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection_repository::{ConnectionRepositoryErrorCode, SharedConnectionRepository};
    use crate::credential_vault::test_support::MemoryCredentialVault;
    use crate::db::DbType;
    use crate::mcp_sync::{McpControlCommand, McpSyncContext, McpSyncRequest, PROTOCOL_VERSION};
    use crate::platform::UiEventBus;
    use std::sync::Arc;
    use uuid::Uuid;

    fn sqlite_config(id: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: id.to_string(),
            name: id.to_string(),
            db_type: DbType::SQLite,
            host: ":memory:".to_string(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            color: None,
        }
    }

    fn save_request(id: &str, expected_revision: Option<i64>) -> ValidatedProfile {
        ValidatedProfile::from_request(crate::connection_repository::SaveConnectionRequest {
            config: sqlite_config(id),
            expected_revision,
            mcp_enabled: true,
            group_name: None,
            tags: Vec::new(),
        })
    }

    fn mcp_context(service_id: Uuid, session_id: Uuid) -> McpSyncContext {
        McpSyncContext {
            protocol_version: PROTOCOL_VERSION,
            service_id,
            session_id,
            operation_id: Uuid::new_v4(),
        }
    }

    async fn register_managed_session(
        registry: &McpSyncRegistry,
        service_id: Uuid,
        session_id: Uuid,
        profile: &SharedConnectionProfile,
    ) {
        let acquired = registry
            .apply_test_request(
                service_id,
                McpSyncRequest::Acquire {
                    context: mcp_context(service_id, session_id),
                    connection_id: profile.id.clone(),
                    profile_revision: profile.revision,
                },
            )
            .await;
        let generation = acquired.generation.expect("managed generation");
        let connected = registry
            .apply_test_request(
                service_id,
                McpSyncRequest::Connected {
                    context: mcp_context(service_id, session_id),
                    connection_id: profile.id.clone(),
                    generation,
                },
            )
            .await;
        assert!(connected.ok);
    }

    async fn poll_managed_disconnect(
        registry: &McpSyncRegistry,
        service_id: Uuid,
        session_id: Uuid,
    ) -> McpControlCommand {
        registry
            .apply_test_request(
                service_id,
                McpSyncRequest::PollControl {
                    context: mcp_context(service_id, session_id),
                },
            )
            .await
            .control
            .expect("managed disconnect command")
    }

    async fn acknowledge_managed_disconnect(
        registry: &McpSyncRegistry,
        service_id: Uuid,
        session_id: Uuid,
        command: McpControlCommand,
        result: Result<(), &str>,
    ) {
        let (ok, error) = match result {
            Ok(()) => (true, None),
            Err(error) => (false, Some(error.to_string())),
        };
        let acknowledgement = registry
            .apply_test_request(
                service_id,
                McpSyncRequest::ControlResult {
                    context: mcp_context(service_id, session_id),
                    command_id: command.command_id,
                    connection_id: command.connection_id,
                    generation: command.generation,
                    ok,
                    error,
                },
            )
            .await;
        assert!(acknowledgement.ok);
    }

    fn service() -> (
        tempfile::TempDir,
        SharedConnectionRepository,
        McpSyncRegistry,
        ConnectionService,
    ) {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        );
        let events = UiEventBus::new();
        let registry = McpSyncRegistry::new(repository.clone(), Arc::new(events));
        let service =
            ConnectionService::new(ConnectionManager::new(repository.clone()), registry.clone());
        (directory, repository, registry, service)
    }

    #[tokio::test]
    async fn snapshot_combines_repository_session_and_managed_mcp_state() {
        let (_directory, _repository, registry, service) = service();
        let profile = service
            .save_profile(save_request("local", None))
            .await
            .expect("save profile");

        let initial = service.snapshot().await.expect("initial snapshot");
        assert_eq!(initial.repository_revision, 1);
        assert_eq!(initial.mcp_revision, 0);
        assert!(!initial.profiles[0].session.is_connected());
        assert!(initial.profiles[0].mcp_usage.is_none());

        assert_eq!(
            service.connect("local").await.expect("connect"),
            ConnectionOutcome::Succeeded
        );
        registry
            .register_test_ownership("local", profile.revision)
            .await;
        let active = service.snapshot().await.expect("active snapshot");
        assert!(active.profiles[0].session.is_connected());
        assert!(active.profiles[0].session.generation.is_some());
        assert_eq!(active.mcp_revision, 1);
        assert_eq!(
            active.profiles[0]
                .mcp_usage
                .as_ref()
                .expect("managed MCP state")
                .profile_revision,
            profile.revision
        );
    }

    #[tokio::test]
    async fn managed_mcp_ownership_blocks_save_and_delete() {
        let (_directory, _repository, registry, service) = service();
        let profile = service
            .save_profile(save_request("managed", None))
            .await
            .expect("save profile");
        registry
            .register_test_ownership("managed", profile.revision)
            .await;

        let mut update = save_request("managed", Some(profile.revision));
        update.request_mut().config.name = "Updated".to_string();
        let save_error = service
            .save_profile(update)
            .await
            .expect_err("managed ownership must block save");
        assert_eq!(
            save_error.code,
            ConnectionRepositoryErrorCode::ConnectionInUse
        );

        let delete_error = service
            .delete_profile("managed", profile.revision)
            .await
            .expect_err("managed ownership must block delete");
        assert_eq!(
            delete_error.code,
            ConnectionRepositoryErrorCode::ConnectionInUse
        );
    }

    #[tokio::test]
    async fn disconnect_reports_local_progress_and_external_usage() {
        let (_directory, repository, _registry, service) = service();
        service
            .save_profile(save_request("external", None))
            .await
            .expect("save profile");
        assert_eq!(
            service.connect("external").await.expect("connect"),
            ConnectionOutcome::Succeeded
        );
        let usage = repository
            .acquire_mcp_usage("external")
            .expect("external usage lease");

        let report = service.disconnect("external").await;
        assert!(report.local_session_disconnected);
        assert!(matches!(
            &report.managed_mcp,
            ManagedMcpDisconnect::Completed {
                requested: 0,
                completed: 0
            }
        ));
        assert!(matches!(&report.external_mcp, ExternalMcpUsage::StillInUse));
        assert!(!report.is_complete());
        assert!(report.message().contains("STDIO"));
        assert!(report.message().contains("已断开 App 连接"));

        drop(usage);
    }

    #[tokio::test]
    async fn disconnect_preserves_partial_managed_mcp_progress() {
        let (_directory, _repository, registry, service) = service();
        let profile = service
            .save_profile(save_request("partial-managed", None))
            .await
            .expect("save profile");
        let service_id = Uuid::new_v4();
        let successful_session = Uuid::new_v4();
        let failed_session = Uuid::new_v4();
        register_managed_session(&registry, service_id, successful_session, &profile).await;
        register_managed_session(&registry, service_id, failed_session, &profile).await;

        let disconnect_service = service.clone();
        let disconnect_task =
            tokio::spawn(async move { disconnect_service.disconnect("partial-managed").await });
        let successful_command =
            poll_managed_disconnect(&registry, service_id, successful_session).await;
        let failed_command = poll_managed_disconnect(&registry, service_id, failed_session).await;
        acknowledge_managed_disconnect(
            &registry,
            service_id,
            successful_session,
            successful_command,
            Ok(()),
        )
        .await;
        acknowledge_managed_disconnect(
            &registry,
            service_id,
            failed_session,
            failed_command,
            Err("driver refused to close"),
        )
        .await;

        let report = disconnect_task.await.expect("disconnect task");
        assert_eq!(
            &report.managed_mcp,
            &ManagedMcpDisconnect::Failed {
                requested: 2,
                completed: 1,
                error: "driver refused to close".to_string(),
            }
        );
        let message = report.message();
        assert!(message.contains("已断开 1 个 Streamable HTTP MCP 会话"));
        assert!(message.contains("已完成 1/2"));
        assert!(message.contains("driver refused to close"));
    }

    #[tokio::test]
    async fn database_loads_carry_the_session_generation_across_reconnects() {
        let (_directory, _repository, _registry, service) = service();
        service
            .save_profile(save_request("local", None))
            .await
            .expect("save profile");
        assert_eq!(
            service.connect("local").await.expect("connect"),
            ConnectionOutcome::Succeeded
        );
        let first = service
            .load_databases("local")
            .await
            .expect("first databases");
        assert_eq!(first.databases, vec!["main".to_string()]);

        let disconnected = service.disconnect("local").await;
        assert!(disconnected.local_session_disconnected);
        assert!(disconnected.is_complete());
        assert_eq!(
            service.connect("local").await.expect("reconnect"),
            ConnectionOutcome::Succeeded
        );
        let second = service
            .load_databases("local")
            .await
            .expect("second databases");

        assert_ne!(first.session_generation, second.session_generation);
        let snapshot = service.snapshot().await.expect("reconnected snapshot");
        assert_eq!(
            snapshot.profiles[0].session.generation,
            Some(second.session_generation)
        );
    }

    #[test]
    fn disconnect_message_combines_app_and_http_outcomes() {
        assert_eq!(disconnect_progress_message(false, 0), "连接当前未连接");
        assert_eq!(disconnect_progress_message(true, 0), "已断开 App 连接");
        assert_eq!(
            disconnect_progress_message(false, 2),
            "已断开 2 个 Streamable HTTP MCP 会话"
        );
        assert_eq!(
            disconnect_progress_message(true, 1),
            "已断开 App 连接及 1 个 Streamable HTTP MCP 会话"
        );
    }
}
