use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

use serde::Serialize;
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

use crate::connection_repository::{ConnectionRepositoryError, SharedConnectionRepository};
use crate::db::{ConnectionConfig, DatabaseDriver};
use crate::state::create_driver;

pub(super) type DriverHandle = Arc<Mutex<Box<dyn DatabaseDriver>>>;
const MAX_RETAINED_LIFECYCLE_LOCKS: usize = 4_096;

#[derive(Debug, Clone)]
pub(super) enum CatalogError {
    Repository(ConnectionRepositoryError),
    Message(String),
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Repository(error) => error.fmt(formatter),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for CatalogError {}

impl From<ConnectionRepositoryError> for CatalogError {
    fn from(error: ConnectionRepositoryError) -> Self {
        Self::Repository(error)
    }
}

impl From<String> for CatalogError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for CatalogError {
    fn from(message: &str) -> Self {
        Self::Message(message.to_string())
    }
}

impl From<CatalogError> for String {
    fn from(error: CatalogError) -> Self {
        error.to_string()
    }
}

#[derive(Clone)]
pub(super) struct ConnectionProfile {
    pub config: ConnectionConfig,
    pub password_env: Option<String>,
    pub credential_ref: Option<String>,
    pub revision: i64,
}

#[derive(Clone)]
struct ConnectedDriver {
    handle: DriverHandle,
    profile_revision: i64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(super) struct SavedQuery {
    pub id: String,
    pub name: String,
    pub connection_id: String,
    pub database: String,
    pub sql: String,
}

#[derive(Clone, Default)]
pub(super) struct Catalog {
    profiles: Arc<RwLock<HashMap<String, ConnectionProfile>>>,
    drivers: Arc<RwLock<HashMap<String, ConnectedDriver>>>,
    queries: Arc<RwLock<HashMap<String, SavedQuery>>>,
    update_approvals: Arc<RwLock<HashSet<(String, String)>>>,
    credential_approvals: Arc<RwLock<HashSet<(String, String)>>>,
    lifecycle_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
    repository: Option<SharedConnectionRepository>,
}

impl Catalog {
    pub fn with_repository(repository: SharedConnectionRepository) -> Self {
        Self {
            repository: Some(repository),
            ..Self::default()
        }
    }

    pub const fn uses_shared_repository(&self) -> bool {
        self.repository.is_some()
    }

    async fn resolved_config(
        &self,
        profile: &ConnectionProfile,
    ) -> Result<ConnectionConfig, CatalogError> {
        if let Some(variable) = profile.password_env.as_deref() {
            let mut config = profile.config.clone();
            config.password = std::env::var(variable).map_err(|_| {
                CatalogError::Message(format!("环境变量 {variable} 未设置，无法读取数据库凭据"))
            })?;
            return Ok(config);
        }
        if profile.credential_ref.is_some() {
            let repository = self.repository.as_ref().ok_or_else(|| {
                CatalogError::Message("连接引用了系统凭据，但当前 MCP 会话没有共享仓库".to_string())
            })?;
            return repository
                .resolve_config(&profile.config.id)
                .await
                .map(|(config, _)| config)
                .map_err(CatalogError::Repository);
        }
        Ok(profile.config.clone())
    }

    async fn shared_profile(&self, connection_id: &str) -> Result<ConnectionProfile, CatalogError> {
        let repository = self
            .repository
            .as_ref()
            .ok_or_else(|| CatalogError::Message("当前 MCP 会话没有共享连接仓库".to_string()))?;
        let record = repository
            .get_record(connection_id)
            .await
            .map_err(CatalogError::Repository)?;
        if !record.profile.mcp_enabled {
            return Err(CatalogError::Message(format!(
                "连接 {connection_id} 未允许 MCP 使用"
            )));
        }
        Ok(ConnectionProfile {
            config: record.profile.public_config(),
            password_env: None,
            credential_ref: record.credential_ref,
            revision: record.profile.revision,
        })
    }

    fn credential_approval_scope(profile: &ConnectionProfile) -> Option<(String, String)> {
        let password_env = profile.password_env.as_deref()?;
        let scope = serde_json::to_string(&(
            password_env,
            &profile.config.db_type,
            &profile.config.host,
            profile.config.port,
            &profile.config.username,
            &profile.config.database,
        ))
        .expect("connection credential approval scope must be serializable");
        Some((profile.config.id.clone(), scope))
    }

    fn update_approval_scope(profile: &ConnectionProfile, database: &str) -> (String, String) {
        let scope = serde_json::to_string(&(
            profile.revision,
            &profile.config.db_type,
            &profile.config.host,
            profile.config.port,
            &profile.config.username,
            &profile.config.database,
            &profile.password_env,
            &profile.credential_ref,
            database,
        ))
        .expect("connection update approval scope must be serializable");
        (profile.config.id.clone(), scope)
    }

    pub async fn lock_connection_lifecycle(&self, connection_id: &str) -> OwnedMutexGuard<()> {
        let lifecycle = {
            let mut lifecycles = self.lifecycle_locks.lock().await;
            if lifecycles.len() >= MAX_RETAINED_LIFECYCLE_LOCKS {
                lifecycles.retain(|_, lifecycle| lifecycle.strong_count() > 0);
            }
            if let Some(lifecycle) = lifecycles.get(connection_id).and_then(Weak::upgrade) {
                lifecycle
            } else {
                let lifecycle = Arc::new(Mutex::new(()));
                lifecycles.insert(connection_id.to_string(), Arc::downgrade(&lifecycle));
                lifecycle
            }
        };
        lifecycle.lock_owned().await
    }

    pub async fn insert_profile(&self, profile: ConnectionProfile) -> Result<(), CatalogError> {
        if let Some(repository) = self.repository.as_ref() {
            let mut config = profile.config;
            if let Some(variable) = profile.password_env.as_deref() {
                config.password = std::env::var(variable).map_err(|_| {
                    CatalogError::Message(format!("环境变量 {variable} 未设置，无法读取数据库凭据"))
                })?;
            }
            repository
                .create(config, true)
                .await
                .map(|_| ())
                .map_err(CatalogError::Repository)?;
            return Ok(());
        }

        let id = profile.config.id.clone();
        let mut profiles = self.profiles.write().await;
        if profiles.contains_key(&id) {
            return Err(CatalogError::Message(format!("连接 {id} 已存在")));
        }
        profiles.insert(id, profile);
        Ok(())
    }

    pub async fn profiles(&self) -> Result<Vec<ConnectionProfile>, CatalogError> {
        if let Some(repository) = self.repository.as_ref() {
            return repository
                .list()
                .await
                .map(|profiles| {
                    profiles
                        .into_iter()
                        .filter(|profile| profile.mcp_enabled)
                        .map(|profile| ConnectionProfile {
                            config: profile.public_config(),
                            password_env: None,
                            credential_ref: profile
                                .has_credential
                                .then(|| "system-vault".to_string()),
                            revision: profile.revision,
                        })
                        .collect()
                })
                .map_err(CatalogError::Repository);
        }

        let mut profiles = self
            .profiles
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        profiles.sort_by(|a, b| a.config.name.cmp(&b.config.name));
        Ok(profiles)
    }

    pub async fn profile(&self, connection_id: &str) -> Result<ConnectionProfile, CatalogError> {
        if self.repository.is_some() {
            return self.shared_profile(connection_id).await;
        }
        self.profiles
            .read()
            .await
            .get(connection_id)
            .cloned()
            .ok_or_else(|| CatalogError::Message(format!("连接 {connection_id} 不存在")))
    }

    pub async fn is_connected(&self, connection_id: &str) -> bool {
        self.current_driver(connection_id).await.is_ok()
    }

    async fn current_driver(&self, connection_id: &str) -> Result<DriverHandle, CatalogError> {
        let connected = self
            .drivers
            .read()
            .await
            .get(connection_id)
            .cloned()
            .ok_or_else(|| {
                CatalogError::Message(format!(
                    "连接 {connection_id} 尚未访问，请先调用 connect_connection"
                ))
            })?;
        if self.repository.is_none() {
            return Ok(connected.handle);
        }

        match self.profile(connection_id).await {
            Ok(profile) if profile.revision == connected.profile_revision => Ok(connected.handle),
            profile_result => {
                let removed = {
                    let mut drivers = self.drivers.write().await;
                    let still_current = drivers.get(connection_id).is_some_and(|current| {
                        current.profile_revision == connected.profile_revision
                            && Arc::ptr_eq(&current.handle, &connected.handle)
                    });
                    still_current
                        .then(|| drivers.remove(connection_id))
                        .flatten()
                };
                if let Some(driver) = removed {
                    let _ = driver.handle.lock().await.disconnect().await;
                }
                let reason = match profile_result {
                    Ok(_) => "连接配置已被其他 Astesia 进程修改".to_string(),
                    Err(error) => format!("无法验证最新连接配置：{error}"),
                };
                Err(CatalogError::Message(format!(
                    "{reason}；旧连接已断开，请重新调用 connect_connection（错误码：driver_stale）"
                )))
            }
        }
    }

    pub async fn test_connection(&self, connection_id: &str) -> Result<(), CatalogError> {
        let profile = self.profile(connection_id).await?;
        let config = self.resolved_config(&profile).await?;
        create_driver(&config)
            .test_connection()
            .await
            .map(|_| ())
            .map_err(|error| CatalogError::Message(format!("测试连接失败: {error}")))
    }

    pub async fn connect(&self, connection_id: &str) -> Result<bool, CatalogError> {
        if self.is_connected(connection_id).await {
            return Ok(false);
        }

        let profile = self.profile(connection_id).await?;
        let connected_revision = profile.revision;
        let config = self.resolved_config(&profile).await?;
        let mut driver = create_driver(&config);
        driver
            .connect()
            .await
            .map_err(|error| format!("连接失败: {error}"))?;

        let latest = match self.profile(connection_id).await {
            Ok(profile) if profile.revision == connected_revision => profile,
            Ok(_) => {
                let _ = driver.disconnect().await;
                return Err(CatalogError::Message(format!(
                    "连接 {connection_id} 在建立过程中已被修改，请重新连接（错误码：driver_stale）"
                )));
            }
            Err(error) => {
                let _ = driver.disconnect().await;
                return Err(CatalogError::Message(format!(
                    "连接 {connection_id} 在建立过程中不可用：{error}（错误码：driver_stale）"
                )));
            }
        };
        let mut drivers = self.drivers.write().await;
        if drivers.contains_key(connection_id) {
            drop(drivers);
            let _ = driver.disconnect().await;
            return Ok(false);
        }
        drivers.insert(
            connection_id.to_string(),
            ConnectedDriver {
                handle: Arc::new(Mutex::new(driver)),
                profile_revision: latest.revision,
            },
        );
        Ok(true)
    }

    pub async fn disconnect(&self, connection_id: &str) -> Result<bool, CatalogError> {
        let driver = self.drivers.write().await.remove(connection_id);
        self.update_approvals
            .write()
            .await
            .retain(|(approved_connection, _)| approved_connection != connection_id);
        match driver {
            Some(driver) => {
                driver
                    .handle
                    .lock()
                    .await
                    .disconnect()
                    .await
                    .map_err(|error| CatalogError::Message(format!("断开连接失败: {error}")))?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub async fn remove_profile(
        &self,
        connection_id: &str,
        expected_revision: i64,
    ) -> Result<(), CatalogError> {
        let profile = self.profile(connection_id).await?;
        if profile.revision != expected_revision {
            return Err(CatalogError::Message(format!(
                "连接 {connection_id} 在确认期间已被修改，未删除（错误码：profile_conflict）"
            )));
        }
        self.disconnect(connection_id).await?;
        self.credential_approvals
            .write()
            .await
            .retain(|(approved_connection, _)| approved_connection != connection_id);
        if let Some(repository) = self.repository.as_ref() {
            return repository
                .delete(connection_id, expected_revision)
                .await
                .map(|_| ())
                .map_err(CatalogError::Repository);
        }
        let mut profiles = self.profiles.write().await;
        let current = profiles
            .get(connection_id)
            .ok_or_else(|| CatalogError::Message(format!("连接 {connection_id} 不存在")))?;
        if current.revision != expected_revision {
            return Err(CatalogError::Message(format!(
                "连接 {connection_id} 在确认期间已被修改，未删除（错误码：profile_conflict）"
            )));
        }
        profiles.remove(connection_id);
        Ok(())
    }

    pub async fn driver(&self, connection_id: &str) -> Result<DriverHandle, CatalogError> {
        self.current_driver(connection_id).await
    }

    pub async fn insert_query(&self, query: SavedQuery) -> Result<(), CatalogError> {
        self.profile(&query.connection_id).await?;
        let mut queries = self.queries.write().await;
        if queries.contains_key(&query.id) {
            return Err(CatalogError::Message(format!("查询 {} 已存在", query.id)));
        }
        queries.insert(query.id.clone(), query);
        Ok(())
    }

    pub async fn queries(&self) -> Vec<SavedQuery> {
        let mut queries = self
            .queries
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        queries.sort_by(|a, b| a.name.cmp(&b.name));
        queries
    }

    pub async fn query(&self, query_id: &str) -> Result<SavedQuery, CatalogError> {
        self.queries
            .read()
            .await
            .get(query_id)
            .cloned()
            .ok_or_else(|| CatalogError::Message(format!("查询 {query_id} 不存在")))
    }

    pub async fn remove_query_if_unchanged(
        &self,
        expected: &SavedQuery,
    ) -> Result<SavedQuery, CatalogError> {
        let mut queries = self.queries.write().await;
        let current = queries
            .get(&expected.id)
            .ok_or_else(|| CatalogError::Message(format!("查询 {} 不存在", expected.id)))?;
        if current != expected {
            return Err(CatalogError::Message(format!(
                "查询 {} 在确认期间已被替换，未删除（错误码：query_conflict）",
                expected.id
            )));
        }
        queries
            .remove(&expected.id)
            .ok_or_else(|| CatalogError::Message(format!("查询 {} 不存在", expected.id)))
    }

    pub async fn query_count_for_connection(&self, connection_id: &str) -> usize {
        self.queries
            .read()
            .await
            .values()
            .filter(|query| query.connection_id == connection_id)
            .count()
    }

    pub async fn remove_queries_for_connection(&self, connection_id: &str) -> usize {
        let mut queries = self.queries.write().await;
        let before = queries.len();
        queries.retain(|_, query| query.connection_id != connection_id);
        before - queries.len()
    }

    pub async fn updates_are_approved(&self, profile: &ConnectionProfile, database: &str) -> bool {
        self.update_approvals
            .read()
            .await
            .contains(&Self::update_approval_scope(profile, database))
    }

    pub async fn approve_updates(&self, profile: &ConnectionProfile, database: &str) {
        self.update_approvals
            .write()
            .await
            .insert(Self::update_approval_scope(profile, database));
    }

    pub async fn credential_use_is_approved(&self, profile: &ConnectionProfile) -> bool {
        let Some(scope) = Self::credential_approval_scope(profile) else {
            return true;
        };
        self.credential_approvals.read().await.contains(&scope)
    }

    pub async fn approve_credential_use(&self, profile: &ConnectionProfile) {
        if let Some(scope) = Self::credential_approval_scope(profile) {
            self.credential_approvals.write().await.insert(scope);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        connection_repository::ConnectionRepositoryErrorCode,
        credential_vault::{
            test_support::MemoryCredentialVault, CredentialVault, CredentialVaultError,
            CredentialVaultErrorCode,
        },
    };

    struct MigrationRequiredVault;

    #[async_trait::async_trait]
    impl CredentialVault for MigrationRequiredVault {
        async fn put(
            &self,
            _binding: &[u8],
            _secret: &str,
        ) -> Result<String, CredentialVaultError> {
            Ok("legacy-reference".to_string())
        }

        async fn get(
            &self,
            _reference: &str,
            _binding: &[u8],
        ) -> Result<String, CredentialVaultError> {
            Err(CredentialVaultError {
                code: CredentialVaultErrorCode::MigrationRequired,
                message: "旧版凭据尚未完成安全迁移".to_string(),
                remediation: "请先打开 Astesia App 完成迁移。".to_string(),
            })
        }

        async fn delete(&self, _reference: &str) -> Result<(), CredentialVaultError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn update_approval_is_scoped_to_exact_profile_and_database() {
        let catalog = Catalog::default();
        let profile = credential_profile();
        catalog.approve_updates(&profile, "database-a").await;

        assert!(catalog.updates_are_approved(&profile, "database-a").await);
        assert!(!catalog.updates_are_approved(&profile, "database-b").await);

        let mut different_connection = profile.clone();
        different_connection.config.id = "connection-b".into();
        assert!(
            !catalog
                .updates_are_approved(&different_connection, "database-a")
                .await
        );

        let mut different_endpoint = profile.clone();
        different_endpoint.config.host = "replacement.internal".into();
        assert!(
            !catalog
                .updates_are_approved(&different_endpoint, "database-a")
                .await
        );

        let mut different_revision = profile.clone();
        different_revision.revision += 1;
        assert!(
            !catalog
                .updates_are_approved(&different_revision, "database-a")
                .await
        );

        catalog
            .disconnect("connection-a")
            .await
            .expect("disconnect without a live driver");
        assert!(!catalog.updates_are_approved(&profile, "database-a").await);
    }

    fn credential_profile() -> ConnectionProfile {
        ConnectionProfile {
            config: ConnectionConfig {
                id: "connection-a".into(),
                name: "Analytics".into(),
                db_type: crate::db::DbType::PostgreSQL,
                host: "db.internal".into(),
                port: 5432,
                username: "reader".into(),
                password: String::new(),
                database: Some("analytics".into()),
                color: None,
            },
            password_env: Some("ASTESIA_DB_PASSWORD_ANALYTICS".into()),
            credential_ref: None,
            revision: 0,
        }
    }

    #[tokio::test]
    async fn shared_repository_errors_remain_typed() {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        );
        let catalog = Catalog::with_repository(repository);

        let error = match catalog.profile("missing-connection").await {
            Ok(_) => panic!("missing profile unexpectedly resolved"),
            Err(error) => error,
        };

        match error {
            CatalogError::Repository(error) => {
                assert_eq!(error.code, ConnectionRepositoryErrorCode::ProfileNotFound);
                assert_eq!(error.details["connection_id"], "missing-connection");
            }
            CatalogError::Message(message) => {
                panic!("repository error was flattened into a string: {message}")
            }
        }
    }

    #[tokio::test]
    async fn credential_migration_error_remains_typed_during_connection_test() {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            Arc::new(MigrationRequiredVault),
        );
        let mut config = credential_profile().config;
        config.password = "legacy-password".to_string();
        repository
            .create(config, true)
            .await
            .expect("create shared profile");
        let catalog = Catalog::with_repository(repository);

        let error = match catalog.test_connection("connection-a").await {
            Ok(()) => panic!("legacy credential unexpectedly resolved"),
            Err(error) => error,
        };

        match error {
            CatalogError::Repository(error) => {
                assert_eq!(
                    error.code,
                    ConnectionRepositoryErrorCode::CredentialMigrationRequired
                );
                assert!(error.message.contains("尚未完成安全迁移"));
                assert!(error.remediation.contains("Astesia App"));
            }
            CatalogError::Message(message) => {
                panic!("credential migration error was flattened: {message}")
            }
        }
    }

    #[tokio::test]
    async fn credential_approval_is_scoped_to_the_exact_profile_and_session() {
        let catalog = Catalog::default();
        let profile = credential_profile();

        assert!(!catalog.credential_use_is_approved(&profile).await);
        catalog.approve_credential_use(&profile).await;
        assert!(catalog.credential_use_is_approved(&profile).await);

        let mut different_endpoint = profile.clone();
        different_endpoint.config.host = "attacker.invalid".into();
        assert!(
            !catalog
                .credential_use_is_approved(&different_endpoint)
                .await
        );

        let mut different_credential = profile.clone();
        different_credential.password_env = Some("ASTESIA_DB_PASSWORD_OTHER".into());
        assert!(
            !catalog
                .credential_use_is_approved(&different_credential)
                .await
        );

        let fresh_session = Catalog::default();
        assert!(!fresh_session.credential_use_is_approved(&profile).await);
    }

    #[tokio::test]
    async fn removing_a_profile_revokes_its_credential_approval() {
        let catalog = Catalog::default();
        let profile = credential_profile();
        catalog
            .insert_profile(profile.clone())
            .await
            .expect("insert profile");
        catalog.approve_credential_use(&profile).await;

        catalog
            .remove_profile(&profile.config.id, profile.revision)
            .await
            .expect("remove profile");
        catalog
            .insert_profile(profile.clone())
            .await
            .expect("reinsert profile");

        assert!(!catalog.credential_use_is_approved(&profile).await);
    }

    #[tokio::test]
    async fn profile_delete_rejects_a_revision_changed_during_confirmation() {
        let catalog = Catalog::default();
        let profile = credential_profile();
        catalog
            .insert_profile(profile.clone())
            .await
            .expect("insert profile");
        let mut replacement = profile.clone();
        replacement.revision = 1;
        replacement.config.host = "replacement.internal".into();
        catalog
            .profiles
            .write()
            .await
            .insert(profile.config.id.clone(), replacement.clone());

        let error = catalog
            .remove_profile(&profile.config.id, profile.revision)
            .await
            .expect_err("stale confirmation must not delete replacement");

        assert!(error.to_string().contains("profile_conflict"));
        assert_eq!(
            catalog
                .profile(&profile.config.id)
                .await
                .expect("replacement remains")
                .config
                .host,
            replacement.config.host
        );
    }

    #[tokio::test]
    async fn query_delete_rejects_a_definition_replaced_during_confirmation() {
        let catalog = Catalog::default();
        let expected = SavedQuery {
            id: "query-a".into(),
            name: "Old query".into(),
            connection_id: "connection-a".into(),
            database: "analytics".into(),
            sql: "SELECT 1".into(),
        };
        catalog
            .queries
            .write()
            .await
            .insert(expected.id.clone(), expected.clone());
        let mut replacement = expected.clone();
        replacement.sql = "DELETE FROM users".into();
        catalog
            .queries
            .write()
            .await
            .insert(expected.id.clone(), replacement.clone());

        let error = catalog
            .remove_query_if_unchanged(&expected)
            .await
            .expect_err("stale confirmation must not delete replacement");

        assert!(error.to_string().contains("query_conflict"));
        assert_eq!(
            catalog
                .query(&expected.id)
                .await
                .expect("replacement remains"),
            replacement
        );
    }

    #[tokio::test]
    async fn connection_lifecycle_operations_are_serialized() {
        let catalog = Catalog::default();
        let first = catalog.lock_connection_lifecycle("connection-a").await;
        let waiting_catalog = catalog.clone();
        let waiting = tokio::spawn(async move {
            let _guard = waiting_catalog
                .lock_connection_lifecycle("connection-a")
                .await;
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());

        drop(first);
        tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
            .await
            .expect("second lifecycle operation acquires after the first")
            .expect("lifecycle task");
    }
}
