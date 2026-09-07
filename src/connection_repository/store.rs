use std::{path::PathBuf, sync::Arc, time::Duration};

use serde_json::json;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    SqlitePool,
};
use tokio::sync::{Mutex, OnceCell};
use uuid::Uuid;

use super::{
    default_database_path,
    format::{
        credential_binding, credential_scope, is_sqlite_busy_error, profile_select, row_to_profile,
        row_to_record,
    },
    schema::initialize_schema,
    ConnectionProfilesSnapshot, ConnectionRepositoryError, ConnectionRepositoryErrorCode,
    CredentialVerificationScope, SharedConnectionProfile, SharedConnectionRecord,
    SharedConnectionRepository,
};
use crate::{
    connection_usage::{
        ConnectionMutationGuard, ConnectionUsageError, ConnectionUsageLease, ConnectionUsageLocks,
    },
    credential_vault::{CredentialVaultHandle, SystemCredentialVault},
    db::ConnectionConfig,
};

impl SharedConnectionRepository {
    pub(crate) fn schema_cache_path(&self) -> PathBuf {
        self.database_path.with_extension("schema-cache.sqlite3")
    }
    pub fn new(database_path: PathBuf, vault: CredentialVaultHandle) -> Self {
        let usage_locks = ConnectionUsageLocks::for_repository(&database_path);
        Self {
            database_path: Arc::new(database_path),
            usage_locks,
            pool: Arc::new(OnceCell::new()),
            vault,
            cleanup_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn new_default() -> Result<Self, ConnectionRepositoryError> {
        Ok(Self::new(
            default_database_path()?,
            SystemCredentialVault::shared(),
        ))
    }

    pub fn new_default_strict() -> Result<Self, ConnectionRepositoryError> {
        Ok(Self::new(
            default_database_path()?,
            SystemCredentialVault::shared_strict(),
        ))
    }

    pub(super) async fn initialized_pool(&self) -> Result<&SqlitePool, ConnectionRepositoryError> {
        self.pool
            .get_or_try_init(|| async {
                if let Some(parent) = self.database_path.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|_| {
                        ConnectionRepositoryError::storage_unavailable(
                            "无法创建 Astesia 共享连接数据目录",
                        )
                    })?;
                }

                let options = SqliteConnectOptions::new()
                    .filename(self.database_path.as_ref())
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Full)
                    .busy_timeout(Duration::from_secs(5));
                let mut busy_retries = 0_u32;
                let pool = loop {
                    match SqlitePoolOptions::new()
                        .max_connections(4)
                        .acquire_timeout(Duration::from_secs(6))
                        .connect_with(options.clone())
                        .await
                    {
                        Ok(pool) => break pool,
                        Err(error) if is_sqlite_busy_error(&error) && busy_retries < 6 => {
                            busy_retries += 1;
                            // Entering WAL mode needs an exclusive lock before SQLite's busy
                            // handler applies, so concurrent first-open attempts retry outside it.
                            tokio::time::sleep(Duration::from_millis(10_u64 << busy_retries)).await;
                        }
                        Err(error) => {
                            return Err(ConnectionRepositoryError::from_sqlx(
                                error,
                                "打开共享连接仓库",
                            ));
                        }
                    }
                };
                initialize_schema(&pool).await?;
                Ok(pool)
            })
            .await
    }

    pub(super) async fn pool(&self) -> Result<&SqlitePool, ConnectionRepositoryError> {
        self.initialized_pool().await
    }

    pub(super) async fn retry_pending_credential_cleanup_on_demand(
        &self,
    ) -> Result<(), ConnectionRepositoryError> {
        let pool = self.initialized_pool().await?;
        self.retry_pending_credential_cleanup(pool).await;
        Ok(())
    }

    pub(crate) fn acquire_mcp_usage(
        &self,
        connection_id: &str,
    ) -> Result<ConnectionUsageLease, ConnectionRepositoryError> {
        self.usage_locks
            .try_acquire_usage(connection_id)
            .map_err(|error| match error {
                ConnectionUsageError::Contended => {
                    ConnectionRepositoryError::connection_usage_busy(connection_id)
                }
                ConnectionUsageError::Io(error) => {
                    ConnectionRepositoryError::usage_lock_unavailable(
                        connection_id,
                        "acquire_mcp_usage",
                        error,
                    )
                }
            })
    }

    pub(crate) fn is_connection_externally_in_use(
        &self,
        connection_id: &str,
    ) -> Result<bool, ConnectionRepositoryError> {
        self.usage_locks
            .is_in_use(connection_id)
            .map_err(|error| match error {
                ConnectionUsageError::Contended => {
                    ConnectionRepositoryError::connection_usage_probe_busy(connection_id)
                }
                ConnectionUsageError::Io(error) => {
                    ConnectionRepositoryError::usage_lock_unavailable(
                        connection_id,
                        "probe_mcp_usage",
                        error,
                    )
                }
            })
    }

    pub(super) fn acquire_mutation_guard(
        &self,
        connection_id: &str,
    ) -> Result<ConnectionMutationGuard, ConnectionRepositoryError> {
        self.usage_locks
            .try_acquire_mutation(connection_id)
            .map_err(|error| match error {
                ConnectionUsageError::Contended => {
                    ConnectionRepositoryError::connection_in_use(connection_id)
                }
                ConnectionUsageError::Io(error) => {
                    ConnectionRepositoryError::usage_lock_unavailable(
                        connection_id,
                        "mutate_connection_profile",
                        error,
                    )
                }
            })
    }

    pub async fn current_revision(&self) -> Result<i64, ConnectionRepositoryError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT revision FROM shared_connection_state WHERE singleton = 1",
        )
        .fetch_one(self.pool().await?)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "读取连接版本"))
    }

    pub(super) async fn repository_id(&self) -> Result<String, ConnectionRepositoryError> {
        let repository_id = sqlx::query_scalar::<_, String>(
            "SELECT value FROM shared_connection_meta WHERE key = 'repository_id'",
        )
        .fetch_optional(self.pool().await?)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "读取连接仓库标识"))?
        .ok_or_else(|| {
            ConnectionRepositoryError::new(
                ConnectionRepositoryErrorCode::StorageCorrupt,
                "共享连接仓库缺少 repository_id",
                "请从备份恢复仓库，或联系 Astesia 维护者；不要手动生成替代标识。",
            )
        })?;
        Uuid::parse_str(&repository_id)
            .map(|value| value.to_string())
            .map_err(|_| {
                ConnectionRepositoryError::new(
                    ConnectionRepositoryErrorCode::StorageCorrupt,
                    "共享连接仓库包含无效的 repository_id",
                    "请从备份恢复仓库，或联系 Astesia 维护者；不要手动生成替代标识。",
                )
            })
    }

    pub async fn list(&self) -> Result<Vec<SharedConnectionProfile>, ConnectionRepositoryError> {
        let query = profile_select("ORDER BY group_name COLLATE NOCASE, name COLLATE NOCASE, id");
        let rows = sqlx::query(&query)
            .fetch_all(self.pool().await?)
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "列出连接"))?;

        rows.iter().map(row_to_profile).collect()
    }

    pub async fn snapshot(&self) -> Result<ConnectionProfilesSnapshot, ConnectionRepositoryError> {
        let mut transaction = self
            .pool()
            .await?
            .begin()
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "读取连接快照"))?;
        let revision = sqlx::query_scalar::<_, i64>(
            "SELECT revision FROM shared_connection_state WHERE singleton = 1",
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "读取连接快照版本"))?;
        let query = profile_select("ORDER BY group_name COLLATE NOCASE, name COLLATE NOCASE, id");
        let rows = sqlx::query(&query)
            .fetch_all(&mut *transaction)
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "读取连接快照"))?;
        let profiles = rows
            .iter()
            .map(row_to_profile)
            .collect::<Result<Vec<_>, _>>()?;
        transaction
            .commit()
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "读取连接快照"))?;
        Ok(ConnectionProfilesSnapshot { revision, profiles })
    }

    pub async fn get(
        &self,
        connection_id: &str,
    ) -> Result<SharedConnectionProfile, ConnectionRepositoryError> {
        Ok(self.get_record(connection_id).await?.profile)
    }

    pub(crate) async fn get_record(
        &self,
        connection_id: &str,
    ) -> Result<SharedConnectionRecord, ConnectionRepositoryError> {
        let query = profile_select("WHERE id = ?");
        let row = sqlx::query(&query)
            .bind(connection_id)
            .fetch_optional(self.pool().await?)
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "读取连接"))?
            .ok_or_else(|| ConnectionRepositoryError::not_found(connection_id))?;
        row_to_record(&row)
    }

    pub async fn resolve_config(
        &self,
        connection_id: &str,
    ) -> Result<(ConnectionConfig, i64), ConnectionRepositoryError> {
        let record = self.get_record(connection_id).await?;
        let mut config = record.profile.public_config();
        if let Some(reference) = record.credential_ref {
            let binding = credential_binding(&config);
            config.password = self.vault.get(&reference, &binding).await?;
        }
        Ok((config, record.profile.revision))
    }

    pub async fn resolve_matching_config(
        &self,
        candidate: &ConnectionConfig,
    ) -> Result<ConnectionConfig, ConnectionRepositoryError> {
        let record = self.get_record(&candidate.id).await?;
        if !record.profile.credential_fingerprint_matches(candidate) {
            return Err(ConnectionRepositoryError::new(
                ConnectionRepositoryErrorCode::CredentialReentryRequired,
                "连接端点、账号或数据库已改变，不能复用旧密码",
                "请重新输入密码后再测试或保存；这可防止旧凭据被发送到不同端点。",
            )
            .with_details(json!({ "connection_id": candidate.id })));
        }
        let mut config = candidate.clone();
        if let Some(reference) = record.credential_ref {
            let binding = credential_binding(&config);
            config.password = self.vault.get(&reference, &binding).await?;
        }
        Ok(config)
    }

    pub async fn credential_verification_scope(
        &self,
    ) -> Result<CredentialVerificationScope, ConnectionRepositoryError> {
        let repository_id = self.repository_id().await?;
        let snapshot = self.snapshot().await?;
        let mut profiles = snapshot
            .profiles
            .into_iter()
            .filter(|profile| profile.mcp_enabled)
            .collect::<Vec<_>>();
        profiles.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(credential_scope(
            &repository_id,
            snapshot.revision,
            &profiles,
        ))
    }

    pub async fn verify_enabled_credentials(
        &self,
    ) -> Result<CredentialVerificationScope, ConnectionRepositoryError> {
        let repository_id = self.repository_id().await?;
        let snapshot = self.snapshot().await?;
        let mut profiles = snapshot
            .profiles
            .into_iter()
            .filter(|profile| profile.mcp_enabled)
            .collect::<Vec<_>>();
        profiles.sort_by(|left, right| left.id.cmp(&right.id));
        let mut connection_ids = profiles
            .iter()
            .filter(|profile| profile.has_credential)
            .map(|profile| profile.id.clone())
            .collect::<Vec<_>>();
        connection_ids.sort();
        let expected = credential_scope(&repository_id, snapshot.revision, &profiles);
        for connection_id in &connection_ids {
            let _config = self.resolve_config(connection_id).await?;
        }
        let actual = self.credential_verification_scope().await?;
        if actual != expected {
            return Err(ConnectionRepositoryError::verification_scope_changed(
                &expected, &actual,
            ));
        }
        Ok(expected)
    }

    async fn all_credential_scope(
        &self,
    ) -> Result<(CredentialVerificationScope, Vec<String>), ConnectionRepositoryError> {
        let repository_id = self.repository_id().await?;
        let snapshot = self.snapshot().await?;
        let mut profiles = snapshot
            .profiles
            .into_iter()
            .filter(|profile| profile.has_credential)
            .collect::<Vec<_>>();
        profiles.sort_by(|left, right| left.id.cmp(&right.id));
        let mut connection_ids = profiles
            .iter()
            .map(|profile| profile.id.clone())
            .collect::<Vec<_>>();
        connection_ids.sort();
        let scope = credential_scope(&repository_id, snapshot.revision, &profiles);
        Ok((scope, connection_ids))
    }

    /// Converts any credentials created by the unreleased per-item keychain
    /// format before the bundled sidecar starts. New encrypted-vault
    /// credentials are only decrypted and verified here.
    pub async fn migrate_all_credential_storage(&self) -> Result<usize, ConnectionRepositoryError> {
        let (expected, connection_ids) = self.all_credential_scope().await?;
        for connection_id in &connection_ids {
            let record = self.get_record(connection_id).await?;
            if let Some(reference) = record.credential_ref {
                let config = record.profile.public_config();
                let binding = credential_binding(&config);
                let _secret = self.vault.get(&reference, &binding).await?;
            }
        }
        let (actual, _) = self.all_credential_scope().await?;
        if actual != expected {
            return Err(ConnectionRepositoryError::verification_scope_changed(
                &expected, &actual,
            ));
        }
        Ok(expected.credential_count)
    }
}
