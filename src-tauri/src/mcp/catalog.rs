use std::collections::{HashMap, HashSet};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Weak,
};

use serde::Serialize;
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

use crate::connection_repository::{ConnectionRepositoryError, SharedConnectionRepository};
use crate::connection_usage::ConnectionUsageLease;
use crate::db::{ConnectionConfig, DatabaseDriver};
use crate::state::create_driver;

pub(super) type DriverHandle = Arc<Mutex<Box<dyn DatabaseDriver>>>;
pub(super) type ConnectionGeneration = u64;
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

#[derive(Clone, Debug)]
pub(super) struct ConnectionProfile {
    pub config: ConnectionConfig,
    pub has_credential: bool,
    pub revision: i64,
}

#[derive(Clone)]
struct ConnectedDriver {
    handle: DriverHandle,
    _usage_lease: Arc<ConnectionUsageLease>,
    profile_revision: i64,
    generation: ConnectionGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ConnectOutcome {
    pub opened_now: bool,
    pub generation: ConnectionGeneration,
}

#[derive(Debug)]
pub(super) struct DisconnectOutcome {
    pub generation: Option<ConnectionGeneration>,
    pub result: Result<bool, CatalogError>,
}

pub(super) struct PreparedConnectionTest {
    config: ConnectionConfig,
    _usage_lease: ConnectionUsageLease,
}

impl PreparedConnectionTest {
    pub async fn run(self) -> Result<(), CatalogError> {
        create_driver(&self.config)
            .test_connection()
            .await
            .map(|_| ())
            .map_err(|error| CatalogError::Message(format!("测试连接失败: {error}")))
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(super) struct SavedQuery {
    pub id: String,
    pub name: String,
    pub connection_id: String,
    pub database: String,
    pub sql: String,
}

#[derive(Clone)]
pub(super) struct Catalog {
    drivers: Arc<RwLock<HashMap<String, ConnectedDriver>>>,
    queries: Arc<RwLock<HashMap<String, SavedQuery>>>,
    update_approvals: Arc<RwLock<HashSet<(String, String)>>>,
    lifecycle_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
    next_generation: Arc<AtomicU64>,
    repository: SharedConnectionRepository,
}

impl Catalog {
    pub fn with_repository(repository: SharedConnectionRepository) -> Self {
        Self {
            drivers: Arc::default(),
            queries: Arc::default(),
            update_approvals: Arc::default(),
            lifecycle_locks: Arc::default(),
            next_generation: Arc::new(AtomicU64::new(1)),
            repository,
        }
    }

    fn allocate_generation(&self) -> ConnectionGeneration {
        loop {
            let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
            if generation != 0 {
                return generation;
            }
        }
    }

    async fn resolved_config(
        &self,
        profile: &ConnectionProfile,
    ) -> Result<ConnectionConfig, CatalogError> {
        let (config, revision) = self
            .repository
            .resolve_config(&profile.config.id)
            .await
            .map_err(CatalogError::Repository)?;
        if revision != profile.revision {
            return Err(CatalogError::Message(format!(
                "连接 {} 在读取凭据期间已被修改，请重试（错误码：driver_stale）",
                profile.config.id
            )));
        }
        Ok(config)
    }

    async fn shared_profile(&self, connection_id: &str) -> Result<ConnectionProfile, CatalogError> {
        let record = self
            .repository
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
            has_credential: record.profile.has_credential,
            revision: record.profile.revision,
        })
    }

    fn update_approval_scope(profile: &ConnectionProfile, database: &str) -> (String, String) {
        let scope = serde_json::to_string(&(
            profile.revision,
            &profile.config.db_type,
            &profile.config.host,
            profile.config.port,
            &profile.config.username,
            &profile.config.database,
            profile.has_credential,
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

    pub async fn profiles(&self) -> Result<Vec<ConnectionProfile>, CatalogError> {
        self.repository
            .list()
            .await
            .map(|profiles| {
                profiles
                    .into_iter()
                    .filter(|profile| profile.mcp_enabled)
                    .map(|profile| ConnectionProfile {
                        config: profile.public_config(),
                        has_credential: profile.has_credential,
                        revision: profile.revision,
                    })
                    .collect()
            })
            .map_err(CatalogError::Repository)
    }

    pub async fn profile(&self, connection_id: &str) -> Result<ConnectionProfile, CatalogError> {
        self.shared_profile(connection_id).await
    }

    pub async fn connected_generation(&self, connection_id: &str) -> Option<ConnectionGeneration> {
        self.drivers
            .read()
            .await
            .get(connection_id)
            .map(|connected| connected.generation)
    }

    async fn current_connection(
        &self,
        connection_id: &str,
    ) -> Result<ConnectedDriver, CatalogError> {
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

        match self.profile(connection_id).await {
            Ok(profile) if profile.revision == connected.profile_revision => Ok(connected),
            profile_result => {
                let reason = match profile_result {
                    Ok(_) => "连接配置已被其他 Astesia 进程修改".to_string(),
                    Err(error) => format!("无法验证最新连接配置：{error}"),
                };
                Err(CatalogError::Message(format!(
                    "{reason}；为避免连接占用状态与 Astesia App 失步，旧连接仍保留为占用但已拒绝继续使用，请先调用 disconnect_connection 后重新连接（错误码：driver_stale）"
                )))
            }
        }
    }

    pub async fn test_connection(&self, connection_id: &str) -> Result<(), CatalogError> {
        let _usage_lease = self.repository.acquire_mcp_usage(connection_id)?;
        let profile = self.profile(connection_id).await?;
        let config = self.resolved_config(&profile).await?;
        create_driver(&config)
            .test_connection()
            .await
            .map(|_| ())
            .map_err(|error| CatalogError::Message(format!("测试连接失败: {error}")))
    }

    pub async fn prepare_connection_test(
        &self,
        connection_id: &str,
        expected_profile_revision: i64,
    ) -> Result<PreparedConnectionTest, CatalogError> {
        // Acquire the cross-process lease before re-reading the profile. The
        // App registry has already approved expected_profile_revision, so a
        // newer revision must fail closed instead of testing different data.
        let usage_lease = self.repository.acquire_mcp_usage(connection_id)?;
        let profile = self.profile(connection_id).await?;
        if profile.revision != expected_profile_revision {
            return Err(CatalogError::Message(format!(
                "连接 {connection_id} 已从 revision {expected_profile_revision} 更新为 {}，拒绝使用 App 已批准的旧资料（错误码：driver_stale）",
                profile.revision
            )));
        }
        let config = self.resolved_config(&profile).await?;
        Ok(PreparedConnectionTest {
            config,
            _usage_lease: usage_lease,
        })
    }

    pub async fn connect(&self, connection_id: &str) -> Result<ConnectOutcome, CatalogError> {
        if self.connected_generation(connection_id).await.is_some() {
            let connected = self.current_connection(connection_id).await?;
            return Ok(ConnectOutcome {
                opened_now: false,
                generation: connected.generation,
            });
        }
        let generation = self.allocate_generation();
        self.connect_internal(connection_id, generation, None).await
    }

    pub async fn connect_with_generation(
        &self,
        connection_id: &str,
        generation: ConnectionGeneration,
        expected_profile_revision: i64,
    ) -> Result<ConnectOutcome, CatalogError> {
        self.connect_internal(connection_id, generation, Some(expected_profile_revision))
            .await
    }

    async fn connect_internal(
        &self,
        connection_id: &str,
        generation: ConnectionGeneration,
        expected_profile_revision: Option<i64>,
    ) -> Result<ConnectOutcome, CatalogError> {
        if generation == 0 {
            return Err(CatalogError::Message(
                "连接 generation 必须大于 0".to_string(),
            ));
        }
        let usage_lease = Arc::new(self.repository.acquire_mcp_usage(connection_id)?);
        if self.connected_generation(connection_id).await.is_some() {
            let connected = self.current_connection(connection_id).await?;
            if expected_profile_revision
                .is_some_and(|expected| expected != connected.profile_revision)
            {
                return Err(CatalogError::Message(format!(
                    "连接 {connection_id} 的 App 授权 revision 与现有驱动不一致，拒绝复用（错误码：driver_stale）"
                )));
            }
            if connected.generation == generation {
                return Ok(ConnectOutcome {
                    opened_now: false,
                    generation,
                });
            }
            return Err(CatalogError::Message(format!(
                "连接 {connection_id} 已被 generation {} 占用，不能切换到 generation {generation}",
                connected.generation
            )));
        }

        let profile = self.profile(connection_id).await?;
        if let Some(expected_revision) = expected_profile_revision {
            if profile.revision != expected_revision {
                return Err(CatalogError::Message(format!(
                    "连接 {connection_id} 已从 revision {expected_revision} 更新为 {}，拒绝使用 App 已批准的旧资料（错误码：driver_stale）",
                    profile.revision
                )));
            }
        }
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
        if let Some(existing) = drivers.get(connection_id) {
            let existing_generation = existing.generation;
            drop(drivers);
            let _ = driver.disconnect().await;
            if existing_generation == generation {
                return Ok(ConnectOutcome {
                    opened_now: false,
                    generation,
                });
            }
            return Err(CatalogError::Message(format!(
                "连接 {connection_id} 已被 generation {existing_generation} 占用"
            )));
        }
        drivers.insert(
            connection_id.to_string(),
            ConnectedDriver {
                handle: Arc::new(Mutex::new(driver)),
                _usage_lease: usage_lease,
                profile_revision: latest.revision,
                generation,
            },
        );
        Ok(ConnectOutcome {
            opened_now: true,
            generation,
        })
    }

    async fn clear_update_approvals(&self, connection_id: &str) {
        self.update_approvals
            .write()
            .await
            .retain(|(approved_connection, _)| approved_connection != connection_id);
    }

    async fn close_driver(
        &self,
        connection_id: &str,
        driver: ConnectedDriver,
    ) -> Result<bool, CatalogError> {
        self.clear_update_approvals(connection_id).await;
        driver
            .handle
            .lock()
            .await
            .disconnect()
            .await
            .map_err(|error| CatalogError::Message(format!("断开连接失败: {error}")))?;
        Ok(true)
    }

    pub async fn disconnect(&self, connection_id: &str) -> DisconnectOutcome {
        let driver = self.drivers.write().await.remove(connection_id);
        match driver {
            Some(driver) => {
                let generation = driver.generation;
                DisconnectOutcome {
                    generation: Some(generation),
                    result: self.close_driver(connection_id, driver).await,
                }
            }
            None => {
                self.clear_update_approvals(connection_id).await;
                DisconnectOutcome {
                    generation: None,
                    result: Ok(false),
                }
            }
        }
    }

    pub(crate) async fn disconnect_if_generation_under_lifecycle(
        &self,
        connection_id: &str,
        generation: ConnectionGeneration,
    ) -> Result<bool, CatalogError> {
        let driver = {
            let mut drivers = self.drivers.write().await;
            let matches = drivers
                .get(connection_id)
                .is_some_and(|driver| driver.generation == generation);
            matches.then(|| drivers.remove(connection_id)).flatten()
        };
        match driver {
            Some(driver) => self.close_driver(connection_id, driver).await,
            None => Ok(false),
        }
    }

    pub async fn driver(&self, connection_id: &str) -> Result<DriverHandle, CatalogError> {
        self.current_connection(connection_id)
            .await
            .map(|connected| connected.handle)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        connection_repository::{ConnectionRepositoryErrorCode, SaveConnectionRequest},
        credential_vault::{
            test_support::MemoryCredentialVault, CredentialVault, CredentialVaultError,
            CredentialVaultErrorCode,
        },
        db::DbType,
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

    fn test_catalog() -> (tempfile::TempDir, Catalog) {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        );
        let catalog = Catalog::with_repository(repository);
        (directory, catalog)
    }

    fn credential_profile() -> ConnectionProfile {
        ConnectionProfile {
            config: ConnectionConfig {
                id: "connection-a".into(),
                name: "Analytics".into(),
                db_type: DbType::PostgreSQL,
                host: "db.internal".into(),
                port: 5432,
                username: "reader".into(),
                password: String::new(),
                database: Some("analytics".into()),
                color: None,
            },
            has_credential: true,
            revision: 1,
        }
    }

    #[tokio::test]
    async fn update_approval_is_scoped_to_exact_profile_and_database() {
        let (_directory, catalog) = test_catalog();
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
            .result
            .expect("disconnect");
        assert!(!catalog.updates_are_approved(&profile, "database-a").await);
    }

    #[tokio::test]
    async fn shared_repository_errors_remain_typed() {
        let (_directory, catalog) = test_catalog();
        let error = catalog
            .profile("missing-connection")
            .await
            .expect_err("missing");
        match error {
            CatalogError::Repository(error) => {
                assert_eq!(error.code, ConnectionRepositoryErrorCode::ProfileNotFound);
                assert_eq!(error.details["connection_id"], "missing-connection");
            }
            CatalogError::Message(message) => panic!("flattened repository error: {message}"),
        }
    }

    #[tokio::test]
    async fn shared_profiles_expose_public_config_without_passwords() {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        );
        let mut config = credential_profile().config;
        config.password = "vault-secret".into();
        repository.create(config, true).await.expect("create");
        let catalog = Catalog::with_repository(repository);
        let profiles = catalog.profiles().await.expect("list");
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].has_credential);
        assert!(profiles[0].config.password.is_empty());
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
        repository.create(config, true).await.expect("create");
        let catalog = Catalog::with_repository(repository);
        let error = catalog
            .test_connection("connection-a")
            .await
            .expect_err("migration");
        match error {
            CatalogError::Repository(error) => {
                assert_eq!(
                    error.code,
                    ConnectionRepositoryErrorCode::CredentialMigrationRequired
                );
                assert!(error.message.contains("尚未完成安全迁移"));
                assert!(error.remediation.contains("Astesia App"));
            }
            CatalogError::Message(message) => panic!("flattened credential error: {message}"),
        }
    }

    #[tokio::test]
    async fn prepared_test_rejects_a_profile_changed_after_app_approval() {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        );
        let mut config = credential_profile().config;
        config.db_type = DbType::SQLite;
        config.host = ":memory:".to_string();
        config.port = 0;
        config.username.clear();
        config.database = None;
        let created = repository
            .create(config.clone(), true)
            .await
            .expect("create");

        let mut updated = config;
        updated.name = "Changed after App approval".into();
        let replacement = repository
            .save(SaveConnectionRequest {
                config: updated,
                expected_revision: Some(created.revision),
                mcp_enabled: true,
            })
            .await
            .expect("mutate before test lease");
        assert_ne!(replacement.revision, created.revision);

        let catalog = Catalog::with_repository(repository.clone());
        let error = match catalog
            .prepare_connection_test("connection-a", created.revision)
            .await
        {
            Ok(_) => panic!("stale App approval must not prepare a database test"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("driver_stale"));
        assert!(
            !repository
                .is_connection_externally_in_use("connection-a")
                .expect("failed prepare must release lease"),
            "revision mismatch must not leak the prepared test lease"
        );
    }

    #[tokio::test]
    async fn prepared_test_holds_the_usage_lease_until_it_is_dropped() {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        );
        let mut config = credential_profile().config;
        config.db_type = DbType::SQLite;
        config.host = ":memory:".to_string();
        config.port = 0;
        config.username.clear();
        config.database = None;
        let created = repository
            .create(config.clone(), true)
            .await
            .expect("create");
        let catalog = Catalog::with_repository(repository.clone());

        let prepared = catalog
            .prepare_connection_test("connection-a", created.revision)
            .await
            .expect("prepare");
        assert!(repository
            .is_connection_externally_in_use("connection-a")
            .expect("prepared lease"));

        let mut updated = config.clone();
        updated.name = "Blocked while test is prepared".into();
        let blocked = repository
            .save(SaveConnectionRequest {
                config: updated.clone(),
                expected_revision: Some(created.revision),
                mcp_enabled: true,
            })
            .await
            .expect_err("prepared test must block profile mutation");
        assert_eq!(blocked.code, ConnectionRepositoryErrorCode::ConnectionInUse);

        drop(prepared);
        let saved = repository
            .save(SaveConnectionRequest {
                config: updated,
                expected_revision: Some(created.revision),
                mcp_enabled: true,
            })
            .await
            .expect("mutation after prepared test drops");
        assert!(saved.revision > created.revision);
    }

    #[tokio::test]
    async fn generation_safe_disconnect_ignores_stale_commands() {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        );
        let mut config = credential_profile().config;
        config.db_type = DbType::SQLite;
        config.host = ":memory:".to_string();
        config.port = 0;
        config.username.clear();
        config.database = None;
        let created = repository.create(config, true).await.expect("create");
        let catalog = Catalog::with_repository(repository);

        let connected = catalog
            .connect_with_generation("connection-a", 41, created.revision)
            .await
            .expect("connect");
        assert_eq!(connected.generation, 41);
        assert!(connected.opened_now);
        {
            let _lifecycle = catalog.lock_connection_lifecycle("connection-a").await;
            assert!(!catalog
                .disconnect_if_generation_under_lifecycle("connection-a", 40)
                .await
                .expect("stale"));
        }
        assert_eq!(catalog.connected_generation("connection-a").await, Some(41));
        {
            let _lifecycle = catalog.lock_connection_lifecycle("connection-a").await;
            assert!(catalog
                .disconnect_if_generation_under_lifecycle("connection-a", 41)
                .await
                .expect("current"));
        }
        assert_eq!(catalog.connected_generation("connection-a").await, None);
    }

    #[tokio::test]
    async fn stale_profile_validation_preserves_generation_until_explicit_disconnect() {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        );
        let mut config = credential_profile().config;
        config.db_type = DbType::SQLite;
        config.host = ":memory:".to_string();
        config.port = 0;
        config.username.clear();
        config.database = None;
        let created = repository
            .create(config.clone(), true)
            .await
            .expect("create");
        let catalog = Catalog::with_repository(repository.clone());

        catalog
            .connect_with_generation("connection-a", 73, created.revision)
            .await
            .expect("connect");
        let mut updated = config;
        updated.name = "Updated connection".into();
        let blocked = repository
            .save(SaveConnectionRequest {
                config: updated.clone(),
                expected_revision: Some(created.revision),
                mcp_enabled: true,
            })
            .await
            .expect_err("connected MCP profile must be immutable");
        assert_eq!(blocked.code, ConnectionRepositoryErrorCode::ConnectionInUse);
        assert_eq!(
            catalog.connected_generation("connection-a").await,
            Some(73),
            "rejected profile mutation must preserve the App-owned generation"
        );
        assert!(catalog.driver("connection-a").await.is_ok());

        // Simulate an already-running pre-v3 peer or manual database edit that
        // ignores the lease protocol. Catalog must still fail closed without
        // silently dropping the generation owned by the App sync registry.
        let bypass_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(directory.path().join("connections.sqlite3")),
            )
            .await
            .expect("open repository outside the lease protocol");
        sqlx::query("UPDATE shared_connections SET name = ?, revision = ? WHERE id = ?")
            .bind(&updated.name)
            .bind(created.revision + 1)
            .bind("connection-a")
            .execute(&bypass_pool)
            .await
            .expect("simulate legacy profile update");
        bypass_pool.close().await;

        let error = match catalog.driver("connection-a").await {
            Ok(_) => panic!("stale driver must not remain usable"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("driver_stale"));
        assert_eq!(
            catalog.connected_generation("connection-a").await,
            Some(73),
            "stale validation must not silently lose the generation that the App owns"
        );

        let disconnected = catalog.disconnect("connection-a").await;
        assert_eq!(disconnected.generation, Some(73));
        assert!(disconnected.result.expect("disconnect stale driver"));
        assert_eq!(catalog.connected_generation("connection-a").await, None);
    }

    #[tokio::test]
    async fn app_approved_revision_is_rechecked_after_usage_lease_is_acquired() {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        );
        let mut config = credential_profile().config;
        config.db_type = DbType::SQLite;
        config.host = ":memory:".to_string();
        config.port = 0;
        config.username.clear();
        config.database = None;
        let created = repository.create(config, true).await.expect("create");
        let catalog = Catalog::with_repository(repository.clone());

        let error = catalog
            .connect_with_generation("connection-a", 91, created.revision + 1)
            .await
            .expect_err("stale App approval must fail");
        assert!(error.to_string().contains("driver_stale"));
        assert_eq!(catalog.connected_generation("connection-a").await, None);
        assert!(
            !repository
                .is_connection_externally_in_use("connection-a")
                .expect("failed connection must release usage lease"),
            "revision rejection must not leak the shared lease"
        );

        catalog
            .connect_with_generation("connection-a", 92, created.revision)
            .await
            .expect("current App approval");
        assert!(repository
            .is_connection_externally_in_use("connection-a")
            .expect("connected usage"));
        let disconnected = catalog.disconnect("connection-a").await;
        assert_eq!(disconnected.generation, Some(92));
        assert!(disconnected.result.expect("disconnect"));
        assert!(!repository
            .is_connection_externally_in_use("connection-a")
            .expect("released usage"));
    }

    #[tokio::test]
    async fn query_delete_rejects_a_definition_replaced_during_confirmation() {
        let (_directory, catalog) = test_catalog();
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
            .expect_err("conflict");
        assert!(error.to_string().contains("query_conflict"));
        assert_eq!(
            catalog.query(&expected.id).await.expect("replacement"),
            replacement
        );
    }

    #[tokio::test]
    async fn connection_lifecycle_operations_are_serialized() {
        let (_directory, catalog) = test_catalog();
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
            .expect("acquire timeout")
            .expect("lifecycle task");
    }
}
