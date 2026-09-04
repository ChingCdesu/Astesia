#[cfg(test)]
mod tests;

use std::collections::HashMap;

use crate::connection_repository::{
    ConnectionProfilesSnapshot, ConnectionRepositoryError, ConnectionRepositoryErrorCode,
    DeleteConnectionResult, SaveConnectionRequest, SharedConnectionProfile,
    SharedConnectionRepository,
};
use crate::connection_runtime::{
    ConnectionIntentGeneration, ConnectionRuntime, DriverHandle, ReplacingConnectError,
};
use crate::db::{create_driver, ConnectionConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionOutcome {
    Succeeded,
    Rejected(String),
}

#[derive(Clone)]
pub(super) struct ConnectionManager {
    runtime: ConnectionRuntime<()>,
    repository: SharedConnectionRepository,
}

impl ConnectionManager {
    pub(super) fn new(repository: SharedConnectionRepository) -> Self {
        Self {
            runtime: ConnectionRuntime::new(),
            repository,
        }
    }

    pub async fn test_connection(
        &self,
        config: ConnectionConfig,
    ) -> Result<ConnectionOutcome, String> {
        let config = if config.password.is_empty() {
            match self.repository.get(&config.id).await {
                Ok(_) => self
                    .repository
                    .resolve_matching_config(&config)
                    .await
                    .map_err(|error| error.to_string())?,
                Err(error) if error.code == ConnectionRepositoryErrorCode::ProfileNotFound => {
                    config
                }
                Err(error) => return Err(error.to_string()),
            }
        } else {
            config
        };

        match create_driver(&config).test_connection().await {
            Ok(_) => Ok(ConnectionOutcome::Succeeded),
            Err(error) => Ok(ConnectionOutcome::Rejected(format!("连接失败: {error}"))),
        }
    }

    pub async fn connect(&self, connection_id: &str) -> Result<ConnectionOutcome, String> {
        let generation = self.begin_connect_intent(connection_id).await;
        let (config, revision) = self
            .repository
            .resolve_config(connection_id)
            .await
            .map_err(|error| error.to_string())?;
        let resolved_connection_id = config.id.clone();
        let repository = self.repository.clone();
        let verification_id = resolved_connection_id.clone();
        match self
            .runtime
            .connect_replacing(
                resolved_connection_id,
                generation,
                config,
                revision,
                (),
                move || async move {
                    repository
                        .get(&verification_id)
                        .await
                        .map(|profile| profile.revision)
                },
            )
            .await
        {
            Ok(()) => Ok(ConnectionOutcome::Succeeded),
            Err(ReplacingConnectError::Connect(error)) => {
                Ok(ConnectionOutcome::Rejected(format!("连接失败: {error}")))
            }
            Err(ReplacingConnectError::RevisionChanged) => Ok(ConnectionOutcome::Rejected(
                "连接配置在建立连接期间已被修改，请刷新后重试".to_string(),
            )),
            Err(ReplacingConnectError::Verification(error)) => Err(error.to_string()),
            Err(ReplacingConnectError::Superseded) => Ok(ConnectionOutcome::Rejected(
                "连接在建立完成前已被断开或替换".to_string(),
            )),
        }
    }

    async fn begin_connect_intent(&self, connection_id: &str) -> ConnectionIntentGeneration {
        self.runtime.begin_replacing_intent(connection_id).await
    }

    pub(super) async fn snapshot_with_session_generations(
        &self,
    ) -> Result<(ConnectionProfilesSnapshot, HashMap<String, u64>), ConnectionRepositoryError> {
        let lifecycle_guard = self.runtime.lock_global_lifecycle().await;
        let snapshot = self.repository.snapshot().await?;
        let revisions = snapshot
            .profiles
            .iter()
            .map(|profile| (profile.id.clone(), profile.revision))
            .collect();
        let session_generations = self
            .runtime
            .reconcile_revisions_under_global(lifecycle_guard, revisions)
            .await;
        Ok((snapshot, session_generations))
    }

    pub async fn save_profile(
        &self,
        request: SaveConnectionRequest,
    ) -> Result<SharedConnectionProfile, ConnectionRepositoryError> {
        let lifecycle_guard = self.runtime.lock_global_lifecycle().await;
        let profile = self.repository.save(request).await?;
        self.runtime
            .disconnect_replacing_under_global(lifecycle_guard, &profile.id)
            .await;
        Ok(profile)
    }

    pub async fn delete_profile(
        &self,
        connection_id: &str,
        expected_revision: i64,
    ) -> Result<DeleteConnectionResult, ConnectionRepositoryError> {
        let lifecycle_guard = self.runtime.lock_global_lifecycle().await;
        let result = self
            .repository
            .delete(connection_id, expected_revision)
            .await?;
        self.runtime
            .disconnect_replacing_under_global(lifecycle_guard, connection_id)
            .await;
        Ok(result)
    }

    pub async fn disconnect_local(&self, connection_id: &str) -> bool {
        let lifecycle_guard = self.runtime.lock_global_lifecycle().await;
        self.runtime
            .disconnect_replacing_under_global(lifecycle_guard, connection_id)
            .await
    }

    pub(super) async fn driver(&self, connection_id: &str) -> Result<DriverHandle, String> {
        self.runtime
            .driver(connection_id)
            .await
            .ok_or_else(|| "连接不存在".to_string())
    }

    pub(super) async fn driver_pair(
        &self,
        source_connection_id: &str,
        target_connection_id: &str,
    ) -> Result<(DriverHandle, DriverHandle), String> {
        let (source, target) = self
            .runtime
            .driver_pair(source_connection_id, target_connection_id)
            .await;
        let source = source.ok_or_else(|| "源连接不存在".to_string())?;
        let target = target.ok_or_else(|| "目标连接不存在".to_string())?;
        Ok((source, target))
    }

    pub(super) async fn load_databases(
        &self,
        connection_id: &str,
    ) -> Result<(u64, Vec<String>), String> {
        let (driver_handle, session_generation) = self.driver_session(connection_id).await?;
        let driver = driver_handle.lock_active().await?;
        let databases = driver
            .get_databases()
            .await
            .map_err(|error| format!("获取数据库列表失败: {error}"))?;
        Ok((session_generation, databases))
    }

    pub(super) async fn driver_session(
        &self,
        connection_id: &str,
    ) -> Result<(DriverHandle, u64), String> {
        self.runtime
            .driver_session(connection_id)
            .await
            .ok_or_else(|| "连接不存在".to_string())
    }

    pub(super) fn is_connection_externally_in_use(
        &self,
        connection_id: &str,
    ) -> Result<bool, ConnectionRepositoryError> {
        self.repository
            .is_connection_externally_in_use(connection_id)
    }
}
