use std::{collections::HashSet, fmt, path::PathBuf, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{
    sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
    },
    Row, SqlitePool,
};
use tokio::sync::{Mutex, OnceCell};
use uuid::Uuid;

use crate::{
    connection_usage::{
        ConnectionMutationGuard, ConnectionUsageError, ConnectionUsageLease, ConnectionUsageLocks,
    },
    credential_vault::{
        CredentialVaultError, CredentialVaultErrorCode, CredentialVaultHandle,
        SystemCredentialVault,
    },
    db::{ConnectionConfig, DbType},
};

const APP_IDENTIFIER: &str = "com.astesia.app";
const DATABASE_FILENAME: &str = "connections.sqlite3";
const SCHEMA_VERSION: i64 = 4;
// A staged credential may still be committed by another process after an
// arbitrarily long OS prompt or suspension. Never expire it by wall-clock
// time; only an explicit failed operation may mark it ready for cleanup.
const CREDENTIAL_STAGING_SENTINEL: i64 = i64::MAX;
const MAX_CONNECTION_ID_CHARS: usize = 256;
const MAX_NAME_CHARS: usize = 512;
const MAX_ENDPOINT_CHARS: usize = 4_096;
const MAX_USERNAME_CHARS: usize = 1_024;
const MAX_PASSWORD_BYTES: usize = 65_536;
const MAX_GROUP_NAME_CHARS: usize = 128;
const MAX_TAGS: usize = 20;
const MAX_TAG_CHARS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SharedConnectionProfile {
    pub id: String,
    pub name: String,
    pub db_type: DbType,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub database: Option<String>,
    pub color: Option<String>,
    pub group_name: Option<String>,
    pub tags: Vec<String>,
    pub has_credential: bool,
    pub revision: i64,
    pub mcp_enabled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConnectionProfilesSnapshot {
    pub revision: i64,
    pub profiles: Vec<SharedConnectionProfile>,
}

impl SharedConnectionProfile {
    pub fn public_config(&self) -> ConnectionConfig {
        ConnectionConfig {
            id: self.id.clone(),
            name: self.name.clone(),
            db_type: self.db_type.clone(),
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            password: String::new(),
            database: self.database.clone(),
            color: self.color.clone(),
        }
    }

    fn credential_fingerprint_matches(&self, config: &ConnectionConfig) -> bool {
        self.db_type == config.db_type
            && self.host == config.host
            && self.port == config.port
            && self.username == config.username
            && self.database == config.database
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SharedConnectionRecord {
    pub profile: SharedConnectionProfile,
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveConnectionRequest {
    pub config: ConnectionConfig,
    pub expected_revision: Option<i64>,
    #[serde(default = "default_mcp_enabled")]
    pub mcp_enabled: bool,
    #[serde(default)]
    pub group_name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

const fn default_mcp_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct LegacyMigrationResult {
    pub imported: usize,
    pub skipped: usize,
    pub already_complete: bool,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteConnectionResult {
    pub deleted: bool,
    pub revision: i64,
    pub credential_cleanup_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionRepositoryErrorCode {
    InvalidProfile,
    MigrationIncomplete,
    ProfileNotFound,
    ProfileConflict,
    ConnectionInUse,
    CredentialReentryRequired,
    CredentialMissing,
    CredentialMigrationRequired,
    CredentialStoreUnavailable,
    CredentialAccessDenied,
    CredentialCorrupt,
    CredentialInvalid,
    StorageBusy,
    StorageUnavailable,
    StorageCorrupt,
}

impl ConnectionRepositoryErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidProfile => "invalid_profile",
            Self::MigrationIncomplete => "migration_incomplete",
            Self::ProfileNotFound => "profile_not_found",
            Self::ProfileConflict => "profile_conflict",
            Self::ConnectionInUse => "connection_in_use",
            Self::CredentialReentryRequired => "credential_reentry_required",
            Self::CredentialMissing => "credential_missing",
            Self::CredentialMigrationRequired => "credential_migration_required",
            Self::CredentialStoreUnavailable => "credential_store_unavailable",
            Self::CredentialAccessDenied => "credential_access_denied",
            Self::CredentialCorrupt => "credential_corrupt",
            Self::CredentialInvalid => "credential_invalid",
            Self::StorageBusy => "storage_busy",
            Self::StorageUnavailable => "storage_unavailable",
            Self::StorageCorrupt => "storage_corrupt",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionRepositoryError {
    pub code: ConnectionRepositoryErrorCode,
    pub message: String,
    pub remediation: String,
    pub retryable: bool,
    pub details: Value,
}

impl ConnectionRepositoryError {
    fn new(
        code: ConnectionRepositoryErrorCode,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            remediation: remediation.into(),
            retryable: matches!(
                code,
                ConnectionRepositoryErrorCode::StorageBusy
                    | ConnectionRepositoryErrorCode::ConnectionInUse
            ),
            details: json!({}),
        }
    }

    fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new(
            ConnectionRepositoryErrorCode::InvalidProfile,
            message,
            "请修正连接配置后重试。",
        )
    }

    fn not_found(connection_id: &str) -> Self {
        Self::new(
            ConnectionRepositoryErrorCode::ProfileNotFound,
            format!("连接 {connection_id} 不存在"),
            "请先调用 list_connections 刷新连接列表。",
        )
        .with_details(json!({ "connection_id": connection_id }))
    }

    fn migration_incomplete(missing_ids: Vec<String>, conflicting_ids: Vec<String>) -> Self {
        Self::new(
            ConnectionRepositoryErrorCode::MigrationIncomplete,
            "旧连接迁移未覆盖全部连接，Astesia 未确认删除旧数据",
            "请检查缺失或冲突的连接；修正后重新迁移，旧版 localStorage 数据应继续保留。",
        )
        .with_details(json!({
            "missing_ids": missing_ids,
            "conflicting_ids": conflicting_ids,
        }))
    }

    fn conflict(connection_id: &str, expected: i64, actual: Option<i64>) -> Self {
        Self::new(
            ConnectionRepositoryErrorCode::ProfileConflict,
            format!("连接 {connection_id} 已被其他 Astesia 进程修改"),
            "请刷新连接列表，并基于最新 revision 重新提交修改。",
        )
        .with_details(json!({
            "connection_id": connection_id,
            "expected_revision": expected,
            "actual_revision": actual,
        }))
    }

    pub(crate) fn connection_in_use(connection_id: &str) -> Self {
        Self::new(
            ConnectionRepositoryErrorCode::ConnectionInUse,
            format!("连接 {connection_id} 正被 MCP 使用，不能修改或删除"),
            "请先在对应 MCP 客户端断开该连接；Streamable HTTP 也可由 Astesia App 强制断开，然后重试。",
        )
        .with_details(json!({
            "connection_id": connection_id,
            "transport": "mcp",
            "scope": "cross_process",
        }))
    }

    fn connection_usage_busy(connection_id: &str) -> Self {
        Self::new(
            ConnectionRepositoryErrorCode::StorageBusy,
            format!("连接 {connection_id} 的资料正在被修改，MCP 暂时不能使用"),
            "请稍后重新调用 connect_connection 或 test_connection。",
        )
        .with_details(json!({
            "connection_id": connection_id,
            "operation": "acquire_mcp_usage",
        }))
    }

    fn connection_usage_probe_busy(connection_id: &str) -> Self {
        Self::new(
            ConnectionRepositoryErrorCode::StorageBusy,
            format!("连接 {connection_id} 的资料正在被其他进程修改，暂时无法确认 MCP 占用"),
            "请等待资料修改完成后重试断开操作。",
        )
        .with_details(json!({
            "connection_id": connection_id,
            "operation": "probe_mcp_usage",
        }))
    }

    fn usage_lock_unavailable(connection_id: &str, operation: &str, error: std::io::Error) -> Self {
        Self::storage_unavailable(format!(
            "无法为连接 {connection_id} 打开跨进程占用锁：{error}"
        ))
        .with_details(json!({
            "connection_id": connection_id,
            "operation": operation,
        }))
    }

    fn storage_unavailable(message: impl Into<String>) -> Self {
        Self::new(
            ConnectionRepositoryErrorCode::StorageUnavailable,
            message,
            "请检查当前用户的数据目录是否可写，然后重试。",
        )
    }

    pub(crate) fn credential_verifier_unavailable(message: impl Into<String>) -> Self {
        let mut error = Self::new(
            ConnectionRepositoryErrorCode::CredentialStoreUnavailable,
            message,
            "请确认打包的 astesia-mcp 可执行文件存在并可运行，然后重试迁移。macOS 用户还应在图形登录会话中通过 Touch ID、Apple Watch 或本机密码完成系统验证。",
        );
        error.retryable = true;
        error
    }

    pub(crate) fn migration_revision_changed(
        migration_revision: i64,
        actual_revision: i64,
        stage: &str,
    ) -> Self {
        let mut error = Self::new(
            ConnectionRepositoryErrorCode::MigrationIncomplete,
            format!("连接仓库在{stage}期间发生变化"),
            "请刷新连接列表并重新执行迁移；旧版 localStorage 数据应继续保留。",
        )
        .with_details(json!({
            "migration_revision": migration_revision,
            "actual_revision": actual_revision,
            "stage": stage,
        }));
        error.retryable = true;
        error
    }

    pub(crate) fn verification_scope_changed(
        expected: &CredentialVerificationScope,
        actual: &CredentialVerificationScope,
    ) -> Self {
        let mut error = Self::new(
            ConnectionRepositoryErrorCode::MigrationIncomplete,
            "连接仓库在凭据迁移或验证期间发生变化",
            "请刷新连接列表并重新执行迁移；旧版 localStorage 数据应继续保留。",
        )
        .with_details(json!({
            "expected_scope": expected,
            "actual_scope": actual,
        }));
        error.retryable = true;
        error
    }

    fn from_sqlx(error: sqlx::Error, operation: &str) -> Self {
        if let sqlx::Error::Database(database_error) = &error {
            if database_error
                .code()
                .is_some_and(|code| code == "5" || code == "6")
            {
                return Self::new(
                    ConnectionRepositoryErrorCode::StorageBusy,
                    format!("无法{operation}：连接仓库正被其他 Astesia 进程占用"),
                    "请稍后重试；不要同时重复提交相同修改。",
                );
            }
        }

        let code = match error {
            sqlx::Error::Decode(_) | sqlx::Error::ColumnDecode { .. } => {
                ConnectionRepositoryErrorCode::StorageCorrupt
            }
            _ => ConnectionRepositoryErrorCode::StorageUnavailable,
        };
        Self::new(
            code,
            format!("无法{operation}：共享连接仓库不可用或数据格式无效"),
            "请检查 Astesia 用户数据目录和文件权限；不要删除仓库文件，除非已备份。",
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CredentialVerificationScope {
    pub repository_id: String,
    pub repository_revision: i64,
    pub profile_count: usize,
    pub credential_count: usize,
    pub profile_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialVerificationReport {
    pub ok: bool,
    pub verified: usize,
    pub scope: Option<CredentialVerificationScope>,
    pub error: Option<ConnectionRepositoryError>,
}

impl CredentialVerificationReport {
    pub fn success(scope: CredentialVerificationScope) -> Self {
        Self {
            ok: true,
            verified: scope.credential_count,
            scope: Some(scope),
            error: None,
        }
    }

    pub fn failure(error: ConnectionRepositoryError) -> Self {
        Self {
            ok: false,
            verified: 0,
            scope: None,
            error: Some(error),
        }
    }
}

impl From<CredentialVaultError> for ConnectionRepositoryError {
    fn from(error: CredentialVaultError) -> Self {
        let code = match error.code {
            CredentialVaultErrorCode::Missing => ConnectionRepositoryErrorCode::CredentialMissing,
            CredentialVaultErrorCode::MigrationRequired => {
                ConnectionRepositoryErrorCode::CredentialMigrationRequired
            }
            CredentialVaultErrorCode::StoreUnavailable => {
                ConnectionRepositoryErrorCode::CredentialStoreUnavailable
            }
            CredentialVaultErrorCode::AccessDenied => {
                ConnectionRepositoryErrorCode::CredentialAccessDenied
            }
            CredentialVaultErrorCode::Corrupt => ConnectionRepositoryErrorCode::CredentialCorrupt,
            CredentialVaultErrorCode::Invalid => ConnectionRepositoryErrorCode::CredentialInvalid,
        };
        Self {
            code,
            message: error.message,
            remediation: error.remediation,
            retryable: false,
            details: json!({}),
        }
    }
}

impl fmt::Display for ConnectionRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}（错误码：{}）。{}",
            self.message,
            self.code.as_str(),
            self.remediation
        )
    }
}

impl std::error::Error for ConnectionRepositoryError {}

#[derive(Clone)]
pub struct SharedConnectionRepository {
    database_path: Arc<PathBuf>,
    usage_locks: ConnectionUsageLocks,
    pool: Arc<OnceCell<SqlitePool>>,
    vault: CredentialVaultHandle,
    cleanup_lock: Arc<Mutex<()>>,
}

impl SharedConnectionRepository {
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

    async fn initialized_pool(&self) -> Result<&SqlitePool, ConnectionRepositoryError> {
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
                let pool = SqlitePoolOptions::new()
                    .max_connections(4)
                    .acquire_timeout(Duration::from_secs(6))
                    .connect_with(options)
                    .await
                    .map_err(|error| {
                        ConnectionRepositoryError::from_sqlx(error, "打开共享连接仓库")
                    })?;
                initialize_schema(&pool).await?;
                Ok(pool)
            })
            .await
    }

    async fn pool(&self) -> Result<&SqlitePool, ConnectionRepositoryError> {
        self.initialized_pool().await
    }

    async fn retry_pending_credential_cleanup_on_demand(
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

    fn acquire_mutation_guard(
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

    async fn repository_id(&self) -> Result<String, ConnectionRepositoryError> {
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
        let rows = sqlx::query(
            "SELECT id, name, db_type, host, port, username, database_name, color, \
                    credential_ref, revision, mcp_enabled, group_name, tags_json \
             FROM shared_connections \
             ORDER BY group_name COLLATE NOCASE, name COLLATE NOCASE, id",
        )
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
        let rows = sqlx::query(
            "SELECT id, name, db_type, host, port, username, database_name, color, \
                    credential_ref, revision, mcp_enabled, group_name, tags_json \
             FROM shared_connections \
             ORDER BY group_name COLLATE NOCASE, name COLLATE NOCASE, id",
        )
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
        let row = sqlx::query(
            "SELECT id, name, db_type, host, port, username, database_name, color, \
                    credential_ref, revision, mcp_enabled, group_name, tags_json \
             FROM shared_connections WHERE id = ?",
        )
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

    pub async fn save(
        &self,
        request: SaveConnectionRequest,
    ) -> Result<SharedConnectionProfile, ConnectionRepositoryError> {
        validate_config(&request.config)?;
        let group_name = normalize_group_name(request.group_name)?;
        let tags = normalize_tags(request.tags)?;
        let connection_id = request.config.id.clone();
        let _mutation_guard = self.acquire_mutation_guard(&connection_id)?;
        match request.expected_revision {
            Some(expected_revision) => {
                self.update(
                    request.config,
                    expected_revision,
                    request.mcp_enabled,
                    group_name,
                    tags,
                )
                .await
            }
            None if group_name.is_none() && tags.is_empty() => {
                self.create(request.config, request.mcp_enabled).await
            }
            None => {
                self.create_with_metadata(request.config, request.mcp_enabled, group_name, tags)
                    .await
            }
        }
    }

    pub async fn create(
        &self,
        config: ConnectionConfig,
        mcp_enabled: bool,
    ) -> Result<SharedConnectionProfile, ConnectionRepositoryError> {
        self.create_with_metadata(config, mcp_enabled, None, Vec::new())
            .await
    }

    async fn create_with_metadata(
        &self,
        mut config: ConnectionConfig,
        mcp_enabled: bool,
        group_name: Option<String>,
        tags: Vec<String>,
    ) -> Result<SharedConnectionProfile, ConnectionRepositoryError> {
        validate_config(&config)?;
        self.retry_pending_credential_cleanup_on_demand().await?;
        self.pool().await?;
        let secret = std::mem::take(&mut config.password);
        let credential_ref = if secret.is_empty() {
            None
        } else {
            let binding = credential_binding(&config);
            let reference = self.vault.put(&binding, &secret).await?;
            if let Err(error) = self.stage_credential_cleanup(&reference).await {
                self.schedule_credential_cleanup(&reference).await;
                return Err(error);
            }
            Some(reference)
        };

        let result = self
            .create_with_reference(
                &config,
                credential_ref.as_deref(),
                mcp_enabled,
                group_name.as_deref(),
                &tags,
            )
            .await;
        if result.is_err() {
            if let Some(reference) = credential_ref.as_deref() {
                self.schedule_credential_cleanup(reference).await;
            }
        }
        result
    }

    async fn create_with_reference(
        &self,
        config: &ConnectionConfig,
        credential_ref: Option<&str>,
        mcp_enabled: bool,
        group_name: Option<&str>,
        tags: &[String],
    ) -> Result<SharedConnectionProfile, ConnectionRepositoryError> {
        let mut transaction = self
            .initialized_pool()
            .await?
            .begin()
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "创建连接"))?;
        let revision = next_revision(&mut transaction).await?;
        let result = sqlx::query(
            "INSERT INTO shared_connections \
             (id, name, db_type, host, port, username, database_name, color, \
              credential_ref, revision, mcp_enabled, group_name, tags_json) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&config.id)
        .bind(&config.name)
        .bind(db_type_to_str(&config.db_type))
        .bind(&config.host)
        .bind(i64::from(config.port))
        .bind(&config.username)
        .bind(&config.database)
        .bind(&config.color)
        .bind(credential_ref)
        .bind(revision)
        .bind(mcp_enabled)
        .bind(group_name)
        .bind(serialize_tags(tags)?)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = result {
            transaction.rollback().await.ok();
            if is_unique_violation(&error) {
                let actual = self
                    .get(&config.id)
                    .await
                    .ok()
                    .map(|profile| profile.revision);
                return Err(ConnectionRepositoryError::conflict(&config.id, 0, actual));
            }
            return Err(ConnectionRepositoryError::from_sqlx(error, "创建连接"));
        }
        if let Some(reference) = credential_ref {
            remove_pending_credential_cleanup(&mut transaction, reference).await?;
        }
        transaction
            .commit()
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "创建连接"))?;
        Ok(profile_from_config(
            config,
            credential_ref.is_some(),
            revision,
            mcp_enabled,
            group_name.map(str::to_string),
            tags.to_vec(),
        ))
    }

    async fn update(
        &self,
        mut config: ConnectionConfig,
        expected_revision: i64,
        mcp_enabled: bool,
        group_name: Option<String>,
        tags: Vec<String>,
    ) -> Result<SharedConnectionProfile, ConnectionRepositoryError> {
        self.retry_pending_credential_cleanup_on_demand().await?;
        let current = self.get_record(&config.id).await?;
        if current.profile.revision != expected_revision {
            return Err(ConnectionRepositoryError::conflict(
                &config.id,
                expected_revision,
                Some(current.profile.revision),
            ));
        }

        let replacement_secret = std::mem::take(&mut config.password);
        if replacement_secret.is_empty()
            && current.credential_ref.is_some()
            && !current.profile.credential_fingerprint_matches(&config)
        {
            return Err(ConnectionRepositoryError::new(
                ConnectionRepositoryErrorCode::CredentialReentryRequired,
                "连接端点、账号或数据库已改变，必须重新输入密码",
                "请重新输入密码后保存；旧密码不会自动发送到新的连接目标。",
            )
            .with_details(json!({ "connection_id": config.id })));
        }

        let new_credential_ref = if replacement_secret.is_empty() {
            current.credential_ref.clone()
        } else {
            let binding = credential_binding(&config);
            let reference = self.vault.put(&binding, &replacement_secret).await?;
            if let Err(error) = self.stage_credential_cleanup(&reference).await {
                self.schedule_credential_cleanup(&reference).await;
                return Err(error);
            }
            Some(reference)
        };
        let wrote_new_credential =
            !replacement_secret.is_empty() && new_credential_ref != current.credential_ref;

        let result = self
            .update_with_reference(
                &config,
                expected_revision,
                new_credential_ref.as_deref(),
                mcp_enabled,
                current.credential_ref.as_deref(),
                group_name.as_deref(),
                &tags,
            )
            .await;
        if result.is_err() && wrote_new_credential {
            if let Some(reference) = new_credential_ref.as_deref() {
                self.schedule_credential_cleanup(reference).await;
            }
        }
        if result.is_ok() && wrote_new_credential {
            if let Some(reference) = current.credential_ref.as_deref() {
                self.cleanup_credential(reference).await;
            }
        }
        result
    }

    async fn update_with_reference(
        &self,
        config: &ConnectionConfig,
        expected_revision: i64,
        credential_ref: Option<&str>,
        mcp_enabled: bool,
        old_credential_ref: Option<&str>,
        group_name: Option<&str>,
        tags: &[String],
    ) -> Result<SharedConnectionProfile, ConnectionRepositoryError> {
        let mut transaction = self
            .initialized_pool()
            .await?
            .begin()
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "更新连接"))?;
        let revision = next_revision(&mut transaction).await?;
        let changed = sqlx::query(
            "UPDATE shared_connections SET \
                name = ?, db_type = ?, host = ?, port = ?, username = ?, \
                database_name = ?, color = ?, credential_ref = ?, revision = ?, mcp_enabled = ?, \
                group_name = ?, tags_json = ? \
             WHERE id = ? AND revision = ?",
        )
        .bind(&config.name)
        .bind(db_type_to_str(&config.db_type))
        .bind(&config.host)
        .bind(i64::from(config.port))
        .bind(&config.username)
        .bind(&config.database)
        .bind(&config.color)
        .bind(credential_ref)
        .bind(revision)
        .bind(mcp_enabled)
        .bind(group_name)
        .bind(serialize_tags(tags)?)
        .bind(&config.id)
        .bind(expected_revision)
        .execute(&mut *transaction)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "更新连接"))?;
        if changed.rows_affected() != 1 {
            transaction.rollback().await.ok();
            let actual = self
                .get(&config.id)
                .await
                .ok()
                .map(|profile| profile.revision);
            return Err(ConnectionRepositoryError::conflict(
                &config.id,
                expected_revision,
                actual,
            ));
        }
        if old_credential_ref.is_some() && old_credential_ref != credential_ref {
            sqlx::query(
                "INSERT OR IGNORE INTO pending_credential_cleanup (credential_ref) VALUES (?)",
            )
            .bind(old_credential_ref)
            .execute(&mut *transaction)
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "记录旧凭据清理任务"))?;
        }
        if let Some(reference) = credential_ref {
            remove_pending_credential_cleanup(&mut transaction, reference).await?;
        }
        transaction
            .commit()
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "更新连接"))?;
        Ok(profile_from_config(
            config,
            credential_ref.is_some(),
            revision,
            mcp_enabled,
            group_name.map(str::to_string),
            tags.to_vec(),
        ))
    }

    pub async fn delete(
        &self,
        connection_id: &str,
        expected_revision: i64,
    ) -> Result<DeleteConnectionResult, ConnectionRepositoryError> {
        let _mutation_guard = self.acquire_mutation_guard(connection_id)?;
        self.retry_pending_credential_cleanup_on_demand().await?;
        let mut transaction = self
            .pool()
            .await?
            .begin()
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "删除连接"))?;
        let revision = next_revision(&mut transaction).await?;
        let credential_ref = sqlx::query_scalar::<_, Option<String>>(
            "DELETE FROM shared_connections \
             WHERE id = ? AND revision = ? \
             RETURNING credential_ref",
        )
        .bind(connection_id)
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "删除连接"))?;
        let Some(credential_ref) = credential_ref else {
            transaction.rollback().await.ok();
            let actual = self
                .get(connection_id)
                .await
                .ok()
                .map(|profile| profile.revision);
            return Err(ConnectionRepositoryError::conflict(
                connection_id,
                expected_revision,
                actual,
            ));
        };
        if let Some(reference) = credential_ref.as_deref() {
            sqlx::query(
                "INSERT OR IGNORE INTO pending_credential_cleanup (credential_ref) VALUES (?)",
            )
            .bind(reference)
            .execute(&mut *transaction)
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "记录凭据清理任务"))?;
        }
        transaction
            .commit()
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "删除连接"))?;

        let credential_cleanup_pending = if let Some(reference) = credential_ref.as_deref() {
            !self.cleanup_credential(reference).await
        } else {
            false
        };
        Ok(DeleteConnectionResult {
            deleted: true,
            revision,
            credential_cleanup_pending,
        })
    }

    pub async fn migration_complete(&self) -> Result<bool, ConnectionRepositoryError> {
        let value = sqlx::query_scalar::<_, String>(
            "SELECT value FROM shared_connection_meta WHERE key = 'legacy_migration_complete'",
        )
        .fetch_optional(self.pool().await?)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "读取迁移状态"))?;
        Ok(value.as_deref() == Some("1"))
    }

    pub async fn migrate_legacy(
        &self,
        mut configs: Vec<ConnectionConfig>,
    ) -> Result<LegacyMigrationResult, ConnectionRepositoryError> {
        validate_unique_configs(&configs)?;
        for config in &configs {
            validate_config(config)?;
        }
        self.retry_pending_credential_cleanup_on_demand().await?;
        if self.migration_complete().await? {
            return self.verify_completed_legacy_migration(&configs).await;
        }

        let mut verified = Vec::new();
        let mut missing_ids = Vec::new();
        let mut conflicting_ids = Vec::new();
        for config in &configs {
            match self.get_record(&config.id).await {
                Ok(record) => {
                    if self.legacy_config_matches(config, &record).await? {
                        verified.push((config.id.clone(), record.profile.revision));
                    } else {
                        conflicting_ids.push(config.id.clone());
                    }
                }
                Err(error) if error.code == ConnectionRepositoryErrorCode::ProfileNotFound => {
                    missing_ids.push(config.id.clone());
                }
                Err(error) => return Err(error),
            }
        }
        if !conflicting_ids.is_empty() {
            return Err(ConnectionRepositoryError::migration_incomplete(
                missing_ids,
                conflicting_ids,
            ));
        }

        let missing = missing_ids.into_iter().collect::<HashSet<_>>();
        let mut prepared: Vec<(ConnectionConfig, Option<String>)> = Vec::new();
        for config in &mut configs {
            if !missing.contains(&config.id) {
                continue;
            }
            let secret = std::mem::take(&mut config.password);
            let reference = if secret.is_empty() {
                None
            } else {
                let binding = credential_binding(config);
                match self.vault.put(&binding, &secret).await {
                    Ok(reference) => {
                        if let Err(error) = self.stage_credential_cleanup(&reference).await {
                            self.schedule_credential_cleanup(&reference).await;
                            for (_, reference) in &prepared {
                                if let Some(reference) = reference {
                                    self.schedule_credential_cleanup(reference).await;
                                }
                            }
                            return Err(error);
                        }
                        Some(reference)
                    }
                    Err(error) => {
                        for (_, reference) in &prepared {
                            if let Some(reference) = reference {
                                self.schedule_credential_cleanup(reference).await;
                            }
                        }
                        return Err(error.into());
                    }
                }
            };
            prepared.push((config.clone(), reference));
        }

        let result = self
            .commit_legacy_migration(&prepared, &verified, configs.len())
            .await;
        if result.is_err() {
            for (_, reference) in &prepared {
                if let Some(reference) = reference {
                    self.schedule_credential_cleanup(reference).await;
                }
            }
        }
        result
    }

    async fn verify_completed_legacy_migration(
        &self,
        configs: &[ConnectionConfig],
    ) -> Result<LegacyMigrationResult, ConnectionRepositoryError> {
        let revision = self.current_revision().await?;
        let mut missing_ids = Vec::new();
        let mut conflicting_ids = Vec::new();
        for config in configs {
            match self.get_record(&config.id).await {
                Ok(record) => {
                    if !self.legacy_config_matches(config, &record).await? {
                        conflicting_ids.push(config.id.clone());
                    }
                }
                Err(error) if error.code == ConnectionRepositoryErrorCode::ProfileNotFound => {
                    missing_ids.push(config.id.clone());
                }
                Err(error) => return Err(error),
            }
        }
        if !missing_ids.is_empty() || !conflicting_ids.is_empty() {
            return Err(ConnectionRepositoryError::migration_incomplete(
                missing_ids,
                conflicting_ids,
            ));
        }
        let actual_revision = self.current_revision().await?;
        if actual_revision != revision {
            return Err(ConnectionRepositoryError::migration_revision_changed(
                revision,
                actual_revision,
                "验证已完成的旧连接迁移",
            ));
        }
        Ok(LegacyMigrationResult {
            imported: 0,
            skipped: configs.len(),
            already_complete: true,
            revision,
        })
    }

    async fn legacy_config_matches(
        &self,
        config: &ConnectionConfig,
        record: &SharedConnectionRecord,
    ) -> Result<bool, ConnectionRepositoryError> {
        let profile = &record.profile;
        if profile.name != config.name
            || profile.db_type != config.db_type
            || profile.host != config.host
            || profile.port != config.port
            || profile.username != config.username
            || profile.database != config.database
            || profile.color != config.color
        {
            return Ok(false);
        }

        match (config.password.is_empty(), record.credential_ref.as_deref()) {
            (true, None) => Ok(true),
            (true, Some(_)) | (false, None) => Ok(false),
            (false, Some(reference)) => {
                let binding = credential_binding(&record.profile.public_config());
                match self.vault.get(reference, &binding).await {
                    Ok(secret) => Ok(secret == config.password),
                    Err(error) if error.code == CredentialVaultErrorCode::Missing => Ok(false),
                    Err(error) => Err(error.into()),
                }
            }
        }
    }

    async fn commit_legacy_migration(
        &self,
        prepared: &[(ConnectionConfig, Option<String>)],
        verified: &[(String, i64)],
        total: usize,
    ) -> Result<LegacyMigrationResult, ConnectionRepositoryError> {
        let mut transaction = self
            .initialized_pool()
            .await?
            .begin()
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "迁移旧连接"))?;
        let already_complete = sqlx::query_scalar::<_, String>(
            "SELECT value FROM shared_connection_meta WHERE key = 'legacy_migration_complete'",
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "读取迁移状态"))?
        .as_deref()
            == Some("1");
        if already_complete {
            transaction.rollback().await.ok();
            let conflicting_ids = prepared
                .iter()
                .map(|(config, _)| config.id.clone())
                .chain(verified.iter().map(|(id, _)| id.clone()))
                .collect();
            return Err(ConnectionRepositoryError::migration_incomplete(
                Vec::new(),
                conflicting_ids,
            ));
        }

        let revision = if prepared.is_empty() {
            sqlx::query_scalar::<_, i64>(
                "SELECT revision FROM shared_connection_state WHERE singleton = 1",
            )
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "读取连接迁移版本"))?
        } else {
            next_revision(&mut transaction).await?
        };
        let mut stale_ids = Vec::new();
        for (connection_id, expected_revision) in verified {
            let matches = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM shared_connections WHERE id = ? AND revision = ?",
            )
            .bind(connection_id)
            .bind(expected_revision)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "验证已存在的旧连接"))?;
            if matches != 1 {
                stale_ids.push(connection_id.clone());
            }
        }
        if !stale_ids.is_empty() {
            transaction.rollback().await.ok();
            return Err(ConnectionRepositoryError::migration_incomplete(
                Vec::new(),
                stale_ids,
            ));
        }

        let mut imported = 0;
        for (config, reference) in prepared {
            let changed = sqlx::query(
                "INSERT INTO shared_connections \
                 (id, name, db_type, host, port, username, database_name, color, \
                  credential_ref, revision, mcp_enabled) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)",
            )
            .bind(&config.id)
            .bind(&config.name)
            .bind(db_type_to_str(&config.db_type))
            .bind(&config.host)
            .bind(i64::from(config.port))
            .bind(&config.username)
            .bind(&config.database)
            .bind(&config.color)
            .bind(reference)
            .bind(revision)
            .execute(&mut *transaction)
            .await;
            let changed = match changed {
                Ok(changed) => changed,
                Err(error) if is_unique_violation(&error) => {
                    transaction.rollback().await.ok();
                    return Err(ConnectionRepositoryError::migration_incomplete(
                        Vec::new(),
                        vec![config.id.clone()],
                    ));
                }
                Err(error) => {
                    return Err(ConnectionRepositoryError::from_sqlx(error, "导入旧连接"));
                }
            };
            imported += changed.rows_affected() as usize;
            if let Some(reference) = reference {
                remove_pending_credential_cleanup(&mut transaction, reference).await?;
            }
        }
        if imported + verified.len() != total {
            transaction.rollback().await.ok();
            return Err(ConnectionRepositoryError::migration_incomplete(
                Vec::new(),
                prepared
                    .iter()
                    .map(|(config, _)| config.id.clone())
                    .chain(verified.iter().map(|(id, _)| id.clone()))
                    .collect(),
            ));
        }
        sqlx::query(
            "INSERT INTO shared_connection_meta (key, value) \
             VALUES ('legacy_migration_complete', '1') \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .execute(&mut *transaction)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "保存迁移状态"))?;
        transaction
            .commit()
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "迁移旧连接"))?;

        Ok(LegacyMigrationResult {
            imported,
            skipped: verified.len(),
            already_complete: false,
            revision,
        })
    }

    async fn stage_credential_cleanup(
        &self,
        reference: &str,
    ) -> Result<(), ConnectionRepositoryError> {
        let pool = self.initialized_pool().await?;
        sqlx::query(
            "INSERT INTO pending_credential_cleanup (credential_ref, cleanup_after) \
             VALUES (?, ?) \
             ON CONFLICT(credential_ref) DO UPDATE SET \
                 cleanup_after = MAX(cleanup_after, excluded.cleanup_after)",
        )
        .bind(reference)
        .bind(CREDENTIAL_STAGING_SENTINEL)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "记录凭据清理任务"))
    }

    async fn mark_credential_cleanup_ready(
        &self,
        reference: &str,
    ) -> Result<(), ConnectionRepositoryError> {
        let pool = self.initialized_pool().await?;
        sqlx::query(
            "INSERT INTO pending_credential_cleanup (credential_ref, cleanup_after) \
             VALUES (?, 0) \
             ON CONFLICT(credential_ref) DO UPDATE SET cleanup_after = 0",
        )
        .bind(reference)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "记录凭据清理任务"))
    }

    async fn schedule_credential_cleanup(&self, reference: &str) -> bool {
        let pool = match self.initialized_pool().await {
            Ok(pool) => pool,
            Err(error) => {
                log::error!(
                    "Unable to persist Astesia credential cleanup task ({}): {}",
                    error.code.as_str(),
                    error.message
                );
                return false;
            }
        };
        if let Err(error) = self.mark_credential_cleanup_ready(reference).await {
            log::error!(
                "Unable to persist Astesia credential cleanup task ({}): {}",
                error.code.as_str(),
                error.message
            );
            return false;
        }
        self.cleanup_credential_with_pool(pool, reference).await
    }

    async fn cleanup_credential(&self, reference: &str) -> bool {
        let pool = match self.initialized_pool().await {
            Ok(pool) => pool,
            Err(error) => {
                log::warn!(
                    "Astesia credential cleanup remains pending ({}): {}",
                    error.code.as_str(),
                    error.message
                );
                return false;
            }
        };
        self.cleanup_credential_with_pool(pool, reference).await
    }

    async fn cleanup_credential_with_pool(&self, pool: &SqlitePool, reference: &str) -> bool {
        let still_referenced = match sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM shared_connections WHERE credential_ref = ?",
        )
        .bind(reference)
        .fetch_one(pool)
        .await
        {
            Ok(count) => count > 0,
            Err(error) => {
                let error =
                    ConnectionRepositoryError::from_sqlx(error, "确认待清理凭据是否仍被引用");
                log::warn!(
                    "Astesia credential cleanup deferred ({}): {}",
                    error.code.as_str(),
                    error.message
                );
                return false;
            }
        };
        if still_referenced {
            let _ = sqlx::query(
                "DELETE FROM pending_credential_cleanup \
                 WHERE credential_ref = ? \
                   AND EXISTS (\
                       SELECT 1 FROM shared_connections WHERE credential_ref = ?\
                   )",
            )
            .bind(reference)
            .bind(reference)
            .execute(pool)
            .await;
            return true;
        }

        match self.vault.delete(reference).await {
            Ok(()) => {
                let _ =
                    sqlx::query("DELETE FROM pending_credential_cleanup WHERE credential_ref = ?")
                        .bind(reference)
                        .execute(pool)
                        .await;
                true
            }
            Err(error) => {
                log::warn!(
                    "Astesia credential cleanup remains pending ({}): {}",
                    error.code.as_str(),
                    error.message
                );
                false
            }
        }
    }

    async fn retry_pending_credential_cleanup(&self, pool: &SqlitePool) {
        let Ok(_cleanup) = self.cleanup_lock.try_lock() else {
            return;
        };
        let references = match sqlx::query_scalar::<_, String>(
            "SELECT credential_ref FROM pending_credential_cleanup \
             WHERE cleanup_after <= unixepoch() \
             ORDER BY credential_ref",
        )
        .fetch_all(pool)
        .await
        {
            Ok(references) => references,
            Err(error) => {
                let error = ConnectionRepositoryError::from_sqlx(error, "读取待清理凭据任务");
                log::warn!(
                    "Unable to retry Astesia credential cleanup ({}): {}",
                    error.code.as_str(),
                    error.message
                );
                return;
            }
        };
        for reference in references {
            self.cleanup_credential_with_pool(pool, &reference).await;
        }
    }
}

pub fn default_database_path() -> Result<PathBuf, ConnectionRepositoryError> {
    let data_dir = dirs::data_dir().ok_or_else(|| {
        ConnectionRepositoryError::storage_unavailable("无法确定当前用户的 Astesia 数据目录")
    })?;
    Ok(data_dir.join(APP_IDENTIFIER).join(DATABASE_FILENAME))
}

async fn initialize_schema(pool: &SqlitePool) -> Result<(), ConnectionRepositoryError> {
    let version = sqlx::query_scalar::<_, i64>("PRAGMA user_version")
        .fetch_one(pool)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "读取仓库版本"))?;
    if version > SCHEMA_VERSION {
        return Err(ConnectionRepositoryError::new(
            ConnectionRepositoryErrorCode::StorageCorrupt,
            format!("共享连接仓库版本 {version} 高于当前支持的 {SCHEMA_VERSION}"),
            "请升级 Astesia；不要用旧版本打开由新版本创建的连接仓库。",
        ));
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS shared_connection_state (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            revision INTEGER NOT NULL CHECK (revision >= 0)
        )",
    )
    .execute(pool)
    .await
    .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "初始化连接仓库"))?;
    sqlx::query(
        "INSERT OR IGNORE INTO shared_connection_state (singleton, revision) VALUES (1, 0)",
    )
    .execute(pool)
    .await
    .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "初始化连接版本"))?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS shared_connections (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            db_type TEXT NOT NULL,
            host TEXT NOT NULL,
            port INTEGER NOT NULL CHECK (port >= 0 AND port <= 65535),
            username TEXT NOT NULL,
            database_name TEXT,
            color TEXT,
            credential_ref TEXT UNIQUE,
            revision INTEGER NOT NULL CHECK (revision >= 0),
            mcp_enabled INTEGER NOT NULL DEFAULT 1 CHECK (mcp_enabled IN (0, 1)),
            group_name TEXT,
            tags_json TEXT NOT NULL DEFAULT '[]'
        )",
    )
    .execute(pool)
    .await
    .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "初始化连接表"))?;
    ensure_shared_connections_column(pool, "group_name", "TEXT").await?;
    ensure_shared_connections_column(pool, "tags_json", "TEXT NOT NULL DEFAULT '[]'").await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS shared_connection_meta (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "初始化迁移状态"))?;
    let generated_repository_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT OR IGNORE INTO shared_connection_meta (key, value) \
         VALUES ('repository_id', ?)",
    )
    .bind(&generated_repository_id)
    .execute(pool)
    .await
    .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "初始化连接仓库标识"))?;
    let repository_id = sqlx::query_scalar::<_, String>(
        "SELECT value FROM shared_connection_meta WHERE key = 'repository_id'",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "验证连接仓库标识"))?;
    Uuid::parse_str(&repository_id).map_err(|_| {
        ConnectionRepositoryError::new(
            ConnectionRepositoryErrorCode::StorageCorrupt,
            "共享连接仓库包含无效的 repository_id",
            "请从备份恢复仓库，或联系 Astesia 维护者；不要手动生成替代标识。",
        )
    })?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS pending_credential_cleanup (
            credential_ref TEXT PRIMARY KEY NOT NULL,
            cleanup_after INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(pool)
    .await
    .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "初始化凭据清理队列"))?;
    let has_cleanup_after = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pragma_table_info('pending_credential_cleanup') \
         WHERE name = 'cleanup_after'",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "检查凭据清理队列版本"))?
        > 0;
    if !has_cleanup_after {
        let alter_result = sqlx::query(
            "ALTER TABLE pending_credential_cleanup \
             ADD COLUMN cleanup_after INTEGER NOT NULL DEFAULT 0",
        )
        .execute(pool)
        .await;
        if let Err(error) = alter_result {
            let upgraded_by_peer = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM pragma_table_info('pending_credential_cleanup') \
                 WHERE name = 'cleanup_after'",
            )
            .fetch_one(pool)
            .await
            .map_err(|check_error| {
                ConnectionRepositoryError::from_sqlx(check_error, "确认凭据清理队列升级结果")
            })? > 0;
            if !upgraded_by_peer {
                return Err(ConnectionRepositoryError::from_sqlx(
                    error,
                    "升级凭据清理队列",
                ));
            }
        }
    }
    sqlx::query(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))
        .execute(pool)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "保存仓库版本"))?;
    Ok(())
}

async fn ensure_shared_connections_column(
    pool: &SqlitePool,
    column_name: &str,
    definition: &str,
) -> Result<(), ConnectionRepositoryError> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pragma_table_info('shared_connections') WHERE name = ?",
    )
    .bind(column_name)
    .fetch_one(pool)
    .await
    .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "检查连接表版本"))?
        > 0;
    if exists {
        return Ok(());
    }

    let alter_result = sqlx::query(&format!(
        "ALTER TABLE shared_connections ADD COLUMN {column_name} {definition}"
    ))
    .execute(pool)
    .await;
    if let Err(error) = alter_result {
        let upgraded_by_peer = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pragma_table_info('shared_connections') WHERE name = ?",
        )
        .bind(column_name)
        .fetch_one(pool)
        .await
        .map_err(|check_error| {
            ConnectionRepositoryError::from_sqlx(check_error, "确认连接表升级结果")
        })? > 0;
        if !upgraded_by_peer {
            return Err(ConnectionRepositoryError::from_sqlx(error, "升级连接表"));
        }
    }
    Ok(())
}

async fn next_revision(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<i64, ConnectionRepositoryError> {
    sqlx::query_scalar::<_, i64>(
        "UPDATE shared_connection_state \
         SET revision = revision + 1 \
         WHERE singleton = 1 AND revision < 9223372036854775807 \
         RETURNING revision",
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "更新连接版本"))?
    .ok_or_else(|| {
        ConnectionRepositoryError::new(
            ConnectionRepositoryErrorCode::StorageCorrupt,
            "共享连接仓库的 revision 已耗尽",
            "请联系 Astesia 维护者，不要重置或删除仓库文件。",
        )
    })
}

async fn remove_pending_credential_cleanup(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    reference: &str,
) -> Result<(), ConnectionRepositoryError> {
    sqlx::query("DELETE FROM pending_credential_cleanup WHERE credential_ref = ?")
        .bind(reference)
        .execute(&mut **transaction)
        .await
        .map(|_| ())
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "确认凭据已被连接引用"))
}

fn row_to_profile(row: &SqliteRow) -> Result<SharedConnectionProfile, ConnectionRepositoryError> {
    Ok(row_to_record(row)?.profile)
}

fn profile_from_config(
    config: &ConnectionConfig,
    has_credential: bool,
    revision: i64,
    mcp_enabled: bool,
    group_name: Option<String>,
    tags: Vec<String>,
) -> SharedConnectionProfile {
    SharedConnectionProfile {
        id: config.id.clone(),
        name: config.name.clone(),
        db_type: config.db_type.clone(),
        host: config.host.clone(),
        port: config.port,
        username: config.username.clone(),
        database: config.database.clone(),
        color: config.color.clone(),
        group_name,
        tags,
        has_credential,
        revision,
        mcp_enabled,
    }
}

fn row_to_record(row: &SqliteRow) -> Result<SharedConnectionRecord, ConnectionRepositoryError> {
    let db_type = db_type_from_str(row.try_get::<String, _>("db_type")?.as_str())?;
    let port = row.try_get::<i64, _>("port")?;
    let port = u16::try_from(port).map_err(|_| {
        ConnectionRepositoryError::new(
            ConnectionRepositoryErrorCode::StorageCorrupt,
            "共享连接仓库包含无效端口",
            "请从备份恢复仓库，或重新创建受影响的连接。",
        )
    })?;
    let credential_ref = row.try_get::<Option<String>, _>("credential_ref")?;
    let tags_json = row.try_get::<String, _>("tags_json")?;
    let tags = serde_json::from_str::<Vec<String>>(&tags_json).map_err(|_| {
        ConnectionRepositoryError::new(
            ConnectionRepositoryErrorCode::StorageCorrupt,
            "共享连接仓库包含无效标签数据",
            "请从备份恢复仓库，或重新保存受影响的连接。",
        )
    })?;
    Ok(SharedConnectionRecord {
        profile: SharedConnectionProfile {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            db_type,
            host: row.try_get("host")?,
            port,
            username: row.try_get("username")?,
            database: row.try_get("database_name")?,
            color: row.try_get("color")?,
            group_name: row.try_get("group_name")?,
            tags,
            has_credential: credential_ref.is_some(),
            revision: row.try_get("revision")?,
            mcp_enabled: row.try_get("mcp_enabled")?,
        },
        credential_ref,
    })
}

impl From<sqlx::Error> for ConnectionRepositoryError {
    fn from(error: sqlx::Error) -> Self {
        ConnectionRepositoryError::from_sqlx(error, "读取共享连接仓库")
    }
}

fn validate_config(config: &ConnectionConfig) -> Result<(), ConnectionRepositoryError> {
    let id = config.id.trim();
    if id.is_empty() || id.chars().count() > MAX_CONNECTION_ID_CHARS {
        return Err(ConnectionRepositoryError::invalid(format!(
            "connection_id 必须为 1-{MAX_CONNECTION_ID_CHARS} 个字符"
        )));
    }
    if config.name.trim().is_empty() || config.name.chars().count() > MAX_NAME_CHARS {
        return Err(ConnectionRepositoryError::invalid(format!(
            "连接名称必须为 1-{MAX_NAME_CHARS} 个字符"
        )));
    }
    if config.host.chars().count() > MAX_ENDPOINT_CHARS || config.host.chars().any(char::is_control)
    {
        return Err(ConnectionRepositoryError::invalid(
            "host/SQLite 路径过长或包含控制字符",
        ));
    }
    if config.username.chars().count() > MAX_USERNAME_CHARS
        || config.username.chars().any(char::is_control)
    {
        return Err(ConnectionRepositoryError::invalid(
            "username 过长或包含控制字符",
        ));
    }
    if config.password.len() > MAX_PASSWORD_BYTES {
        return Err(ConnectionRepositoryError::invalid(format!(
            "密码不能超过 {MAX_PASSWORD_BYTES} 字节"
        )));
    }
    if config
        .database
        .as_deref()
        .is_some_and(|value| value.chars().any(char::is_control))
    {
        return Err(ConnectionRepositoryError::invalid(
            "database 不能包含控制字符",
        ));
    }
    Ok(())
}

fn normalize_group_name(
    group_name: Option<String>,
) -> Result<Option<String>, ConnectionRepositoryError> {
    let Some(group_name) = group_name else {
        return Ok(None);
    };
    let group_name = group_name.trim();
    if group_name.is_empty() {
        return Ok(None);
    }
    if group_name.chars().count() > MAX_GROUP_NAME_CHARS || group_name.chars().any(char::is_control)
    {
        return Err(ConnectionRepositoryError::invalid(format!(
            "分组名称不能超过 {MAX_GROUP_NAME_CHARS} 个字符，且不能包含控制字符"
        )));
    }
    Ok(Some(group_name.to_string()))
}

fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, ConnectionRepositoryError> {
    let mut normalized = Vec::with_capacity(tags.len());
    let mut seen = HashSet::with_capacity(tags.len());
    for tag in tags {
        let tag = tag.trim();
        if tag.is_empty() {
            continue;
        }
        if tag.chars().count() > MAX_TAG_CHARS || tag.chars().any(char::is_control) {
            return Err(ConnectionRepositoryError::invalid(format!(
                "标签不能超过 {MAX_TAG_CHARS} 个字符，且不能包含控制字符"
            )));
        }
        let key = tag.to_lowercase();
        if seen.insert(key) {
            normalized.push(tag.to_string());
            if normalized.len() > MAX_TAGS {
                return Err(ConnectionRepositoryError::invalid(format!(
                    "每个连接最多可设置 {MAX_TAGS} 个标签"
                )));
            }
        }
    }
    Ok(normalized)
}

fn serialize_tags(tags: &[String]) -> Result<String, ConnectionRepositoryError> {
    serde_json::to_string(tags).map_err(|_| {
        ConnectionRepositoryError::new(
            ConnectionRepositoryErrorCode::StorageUnavailable,
            "无法序列化连接标签",
            "请检查标签内容后重试。",
        )
    })
}

fn validate_unique_configs(configs: &[ConnectionConfig]) -> Result<(), ConnectionRepositoryError> {
    let mut ids = HashSet::new();
    for config in configs {
        if !ids.insert(config.id.as_str()) {
            return Err(ConnectionRepositoryError::invalid(format!(
                "旧连接数据包含重复 connection_id：{}",
                config.id
            )));
        }
    }
    Ok(())
}

/// Builds versioned, unambiguous AEAD associated data for a credential.
///
/// Display-only fields are intentionally excluded so a rename or color change
/// can retain the credential. Endpoint and account fields are included so an
/// edited repository cannot redirect a valid ciphertext to another target.
fn credential_binding(config: &ConnectionConfig) -> Vec<u8> {
    let mut binding = b"astesia.connection-credential.v1".to_vec();
    append_binding_field(&mut binding, config.id.as_bytes());
    append_binding_field(&mut binding, db_type_to_str(&config.db_type).as_bytes());
    append_binding_field(&mut binding, config.host.as_bytes());
    append_binding_field(&mut binding, &config.port.to_be_bytes());
    append_binding_field(&mut binding, config.username.as_bytes());
    match config.database.as_deref() {
        Some(database) => {
            binding.push(1);
            append_binding_field(&mut binding, database.as_bytes());
        }
        None => binding.push(0),
    }
    binding
}

fn append_binding_field(binding: &mut Vec<u8>, value: &[u8]) {
    binding.extend_from_slice(&(value.len() as u64).to_be_bytes());
    binding.extend_from_slice(value);
}

fn credential_scope(
    repository_id: &str,
    repository_revision: i64,
    profiles: &[SharedConnectionProfile],
) -> CredentialVerificationScope {
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    digest.update(b"astesia.credential-verification-scope.v3");
    let mut sorted_profiles = profiles.iter().collect::<Vec<_>>();
    sorted_profiles.sort_by(|left, right| left.id.cmp(&right.id));
    for profile in &sorted_profiles {
        update_digest_field(&mut digest, profile.id.as_bytes());
        update_digest_field(&mut digest, profile.name.as_bytes());
        update_digest_field(&mut digest, db_type_to_str(&profile.db_type).as_bytes());
        update_digest_field(&mut digest, profile.host.as_bytes());
        update_digest_field(&mut digest, &profile.port.to_be_bytes());
        update_digest_field(&mut digest, profile.username.as_bytes());
        update_digest_optional_field(&mut digest, profile.database.as_deref());
        update_digest_optional_field(&mut digest, profile.color.as_deref());
        update_digest_optional_field(&mut digest, profile.group_name.as_deref());
        update_digest_field(&mut digest, &(profile.tags.len() as u64).to_be_bytes());
        for tag in &profile.tags {
            update_digest_field(&mut digest, tag.as_bytes());
        }
        update_digest_field(&mut digest, &[u8::from(profile.has_credential)]);
        update_digest_field(&mut digest, &profile.revision.to_be_bytes());
        update_digest_field(&mut digest, &[u8::from(profile.mcp_enabled)]);
    }
    let digest = digest.finish();
    let mut profile_digest = String::with_capacity(digest.as_ref().len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest.as_ref() {
        profile_digest.push(HEX[usize::from(byte >> 4)] as char);
        profile_digest.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    CredentialVerificationScope {
        repository_id: repository_id.to_string(),
        repository_revision,
        profile_count: sorted_profiles.len(),
        credential_count: sorted_profiles
            .iter()
            .filter(|profile| profile.has_credential)
            .count(),
        profile_digest,
    }
}

fn update_digest_field(digest: &mut ring::digest::Context, value: &[u8]) {
    digest.update(&(value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn update_digest_optional_field(digest: &mut ring::digest::Context, value: Option<&str>) {
    match value {
        Some(value) => {
            digest.update(&[1]);
            update_digest_field(digest, value.as_bytes());
        }
        None => digest.update(&[0]),
    }
}

fn db_type_to_str(db_type: &DbType) -> &'static str {
    match db_type {
        DbType::MySQL => "mysql",
        DbType::PostgreSQL => "postgresql",
        DbType::SQLite => "sqlite",
        DbType::SQLServer => "sqlserver",
        DbType::MongoDB => "mongodb",
        DbType::Redis => "redis",
    }
}

fn db_type_from_str(value: &str) -> Result<DbType, ConnectionRepositoryError> {
    match value {
        "mysql" => Ok(DbType::MySQL),
        "postgresql" => Ok(DbType::PostgreSQL),
        "sqlite" => Ok(DbType::SQLite),
        "sqlserver" => Ok(DbType::SQLServer),
        "mongodb" => Ok(DbType::MongoDB),
        "redis" => Ok(DbType::Redis),
        _ => Err(ConnectionRepositoryError::new(
            ConnectionRepositoryErrorCode::StorageCorrupt,
            format!("共享连接仓库包含未知数据库类型：{value}"),
            "请从备份恢复仓库，或重新创建受影响的连接。",
        )),
    }
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database_error) if database_error.is_unique_violation()
    )
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicBool, Ordering},
            Mutex as StdMutex,
        },
    };

    use async_trait::async_trait;
    use tempfile::TempDir;
    use tokio::sync::Barrier;
    use uuid::Uuid;

    use super::*;
    use crate::credential_vault::{
        test_support::MemoryCredentialVault, CredentialVault, CredentialVaultError,
    };

    #[derive(Default)]
    struct DeleteFailingVault {
        secrets: StdMutex<HashMap<String, (Vec<u8>, String)>>,
        fail_delete: AtomicBool,
    }

    impl DeleteFailingVault {
        fn shared() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn set_fail_delete(&self, fail: bool) {
            self.fail_delete.store(fail, Ordering::SeqCst);
        }

        fn contains_secret(&self, secret: &str) -> bool {
            self.secrets
                .lock()
                .expect("secrets lock")
                .values()
                .any(|(_, stored)| stored == secret)
        }

        fn reference_for_secret(&self, secret: &str) -> String {
            self.secrets
                .lock()
                .expect("secrets lock")
                .iter()
                .find_map(|(reference, (_, stored))| (stored == secret).then(|| reference.clone()))
                .expect("secret reference")
        }
    }

    #[async_trait]
    impl CredentialVault for DeleteFailingVault {
        async fn put(&self, binding: &[u8], secret: &str) -> Result<String, CredentialVaultError> {
            let reference = Uuid::new_v4().to_string();
            self.secrets
                .lock()
                .expect("secrets lock")
                .insert(reference.clone(), (binding.to_vec(), secret.to_string()));
            Ok(reference)
        }

        async fn get(
            &self,
            reference: &str,
            binding: &[u8],
        ) -> Result<String, CredentialVaultError> {
            let stored = self
                .secrets
                .lock()
                .expect("secrets lock")
                .get(reference)
                .cloned()
                .ok_or_else(|| CredentialVaultError {
                    code: CredentialVaultErrorCode::Missing,
                    message: "missing test credential".to_string(),
                    remediation: "restore test credential".to_string(),
                })?;
            if stored.0 != binding {
                return Err(CredentialVaultError {
                    code: CredentialVaultErrorCode::Corrupt,
                    message: "test credential binding mismatch".to_string(),
                    remediation: "restore test credential binding".to_string(),
                });
            }
            Ok(stored.1)
        }

        async fn delete(&self, reference: &str) -> Result<(), CredentialVaultError> {
            if self.fail_delete.load(Ordering::SeqCst) {
                return Err(CredentialVaultError {
                    code: CredentialVaultErrorCode::StoreUnavailable,
                    message: "test credential store unavailable".to_string(),
                    remediation: "restore test credential store".to_string(),
                });
            }
            self.secrets.lock().expect("secrets lock").remove(reference);
            Ok(())
        }
    }

    struct BlockingGetVault {
        secrets: StdMutex<HashMap<String, (Vec<u8>, String)>>,
        block_get: AtomicBool,
        get_entered: Barrier,
        release_get: Barrier,
    }

    impl BlockingGetVault {
        fn shared() -> Arc<Self> {
            Arc::new(Self {
                secrets: StdMutex::new(HashMap::new()),
                block_get: AtomicBool::new(false),
                get_entered: Barrier::new(2),
                release_get: Barrier::new(2),
            })
        }
    }

    #[async_trait]
    impl CredentialVault for BlockingGetVault {
        async fn put(&self, binding: &[u8], secret: &str) -> Result<String, CredentialVaultError> {
            let reference = Uuid::new_v4().to_string();
            self.secrets
                .lock()
                .expect("secrets lock")
                .insert(reference.clone(), (binding.to_vec(), secret.to_string()));
            Ok(reference)
        }

        async fn get(
            &self,
            reference: &str,
            binding: &[u8],
        ) -> Result<String, CredentialVaultError> {
            if self.block_get.load(Ordering::SeqCst) {
                self.get_entered.wait().await;
                self.release_get.wait().await;
            }
            let (stored_binding, secret) = self
                .secrets
                .lock()
                .expect("secrets lock")
                .get(reference)
                .cloned()
                .ok_or_else(|| CredentialVaultError {
                    code: CredentialVaultErrorCode::Missing,
                    message: "missing test credential".to_string(),
                    remediation: "restore test credential".to_string(),
                })?;
            if stored_binding != binding {
                return Err(CredentialVaultError {
                    code: CredentialVaultErrorCode::Corrupt,
                    message: "test credential binding mismatch".to_string(),
                    remediation: "restore test credential binding".to_string(),
                });
            }
            Ok(secret)
        }

        async fn delete(&self, reference: &str) -> Result<(), CredentialVaultError> {
            self.secrets.lock().expect("secrets lock").remove(reference);
            Ok(())
        }
    }

    fn config(id: &str, password: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: id.to_string(),
            name: format!("Connection {id}"),
            db_type: DbType::PostgreSQL,
            host: "db.internal".to_string(),
            port: 5432,
            username: "reader".to_string(),
            password: password.to_string(),
            database: Some("analytics".to_string()),
            color: Some("#336791".to_string()),
        }
    }

    fn repository(
        temp_dir: &TempDir,
        vault: Arc<MemoryCredentialVault>,
    ) -> SharedConnectionRepository {
        SharedConnectionRepository::new(temp_dir.path().join("connections.sqlite3"), vault)
    }

    #[tokio::test]
    async fn shares_metadata_and_secret_without_serializing_the_secret() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let first = repository(&temp_dir, vault.clone());
        let second = repository(&temp_dir, vault.clone());

        let created = first
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create");
        assert!(created.has_credential);
        assert!(vault.contains_secret("super-secret"));

        let listed = second.list().await.expect("list from second repository");
        assert_eq!(listed, vec![created.clone()]);
        let serialized = serde_json::to_string(&listed).expect("serialize profiles");
        assert!(!serialized.contains("super-secret"));
        assert!(!serialized.contains("credential_ref"));

        let (resolved, revision) = second.resolve_config("analytics").await.expect("resolve");
        assert_eq!(resolved.password, "super-secret");
        assert_eq!(revision, created.revision);
    }

    #[tokio::test]
    async fn groups_and_tags_round_trip_without_replacing_the_credential() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let repository = repository(&temp_dir, vault.clone());
        let created = repository
            .save(SaveConnectionRequest {
                config: config("analytics", "super-secret"),
                expected_revision: None,
                mcp_enabled: true,
                group_name: Some("  Production  ".to_string()),
                tags: vec![
                    "Critical".to_string(),
                    " reporting ".to_string(),
                    "critical".to_string(),
                ],
            })
            .await
            .expect("create grouped profile");
        assert_eq!(created.group_name.as_deref(), Some("Production"));
        assert_eq!(created.tags, vec!["Critical", "reporting"]);

        let mut updated_config = config("analytics", "");
        updated_config.name = "Renamed analytics".to_string();
        let updated = repository
            .save(SaveConnectionRequest {
                config: updated_config,
                expected_revision: Some(created.revision),
                mcp_enabled: true,
                group_name: Some("Archive".to_string()),
                tags: vec!["read-only".to_string()],
            })
            .await
            .expect("update grouped profile");
        assert_eq!(updated.group_name.as_deref(), Some("Archive"));
        assert_eq!(updated.tags, vec!["read-only"]);
        assert!(vault.contains_secret("super-secret"));
        assert_eq!(
            repository
                .resolve_config("analytics")
                .await
                .expect("resolve existing credential")
                .0
                .password,
            "super-secret"
        );
    }

    #[tokio::test]
    async fn schema_v3_is_upgraded_with_empty_group_and_tags() {
        let temp_dir = TempDir::new().expect("temp dir");
        let path = temp_dir.path().join("connections.sqlite3");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await
            .expect("create v3 database");
        sqlx::query(
            "CREATE TABLE shared_connections (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                db_type TEXT NOT NULL,
                host TEXT NOT NULL,
                port INTEGER NOT NULL,
                username TEXT NOT NULL,
                database_name TEXT,
                color TEXT,
                credential_ref TEXT UNIQUE,
                revision INTEGER NOT NULL,
                mcp_enabled INTEGER NOT NULL DEFAULT 1
            )",
        )
        .execute(&pool)
        .await
        .expect("create v3 connections table");
        sqlx::query(
            "INSERT INTO shared_connections (
                id, name, db_type, host, port, username, revision, mcp_enabled
            ) VALUES ('legacy', 'Legacy', 'sqlite', ':memory:', 0, '', 1, 1)",
        )
        .execute(&pool)
        .await
        .expect("insert v3 profile");
        sqlx::query("PRAGMA user_version = 3")
            .execute(&pool)
            .await
            .expect("mark v3 schema");
        pool.close().await;

        let repository = SharedConnectionRepository::new(path, MemoryCredentialVault::shared());
        let profiles = repository.list().await.expect("upgrade and list");
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].group_name, None);
        assert!(profiles[0].tags.is_empty());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("PRAGMA user_version")
                .fetch_one(
                    repository
                        .initialized_pool()
                        .await
                        .expect("initialized pool"),
                )
                .await
                .expect("schema version"),
            SCHEMA_VERSION
        );
    }

    #[tokio::test]
    async fn mcp_usage_lease_blocks_profile_save_and_delete_across_repositories() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let mcp_repository = repository(&temp_dir, vault.clone());
        let app_repository = repository(&temp_dir, vault);
        let created = mcp_repository
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create");
        let usage = mcp_repository
            .acquire_mcp_usage("analytics")
            .expect("MCP usage");

        assert!(app_repository
            .is_connection_externally_in_use("analytics")
            .expect("probe"));

        let mut changed = config("analytics", "");
        changed.name = "Blocked update".to_string();
        let save_error = app_repository
            .save(SaveConnectionRequest {
                config: changed,
                expected_revision: Some(created.revision),
                mcp_enabled: true,
                group_name: None,
                tags: Vec::new(),
            })
            .await
            .expect_err("save must be non-blockingly rejected");
        assert_eq!(
            save_error.code,
            ConnectionRepositoryErrorCode::ConnectionInUse
        );
        assert!(save_error.retryable);
        assert_eq!(save_error.details["transport"], "mcp");

        let delete_error = app_repository
            .delete("analytics", created.revision)
            .await
            .expect_err("delete must be non-blockingly rejected");
        assert_eq!(
            delete_error.code,
            ConnectionRepositoryErrorCode::ConnectionInUse
        );
        assert_eq!(
            app_repository
                .get("analytics")
                .await
                .expect("unchanged")
                .name,
            created.name
        );

        drop(usage);
        assert!(!app_repository
            .is_connection_externally_in_use("analytics")
            .expect("released probe"));
        let mut changed = config("analytics", "");
        changed.name = "Allowed update".to_string();
        let updated = app_repository
            .save(SaveConnectionRequest {
                config: changed,
                expected_revision: Some(created.revision),
                mcp_enabled: true,
                group_name: None,
                tags: Vec::new(),
            })
            .await
            .expect("save after release");
        let deleted = app_repository
            .delete("analytics", updated.revision)
            .await
            .expect("delete after release");
        assert!(deleted.deleted);
    }

    #[tokio::test]
    async fn vault_unavailable_keeps_metadata_listable_and_fails_closed() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let repository = repository(&temp_dir, vault.clone());
        repository
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create");
        vault.fail_with(Some(CredentialVaultErrorCode::StoreUnavailable));

        assert_eq!(repository.list().await.expect("list").len(), 1);
        let error = repository
            .resolve_config("analytics")
            .await
            .expect_err("credential read must fail");
        assert_eq!(
            error.code,
            ConnectionRepositoryErrorCode::CredentialStoreUnavailable
        );

        vault.fail_with(Some(CredentialVaultErrorCode::MigrationRequired));
        let error = repository
            .resolve_config("analytics")
            .await
            .expect_err("strict sidecar must report legacy migration requirement");
        assert_eq!(
            error.code,
            ConnectionRepositoryErrorCode::CredentialMigrationRequired
        );
        assert!(error.remediation.contains("Astesia App"));
    }

    #[tokio::test]
    async fn storage_migration_covers_disabled_profiles_but_sidecar_scope_does_not() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let repository = repository(&temp_dir, vault);
        repository
            .create(config("enabled", "enabled-secret"), true)
            .await
            .expect("create enabled");
        repository
            .create(config("disabled", "disabled-secret"), false)
            .await
            .expect("create disabled");
        repository
            .create(config("enabled-without-secret", ""), true)
            .await
            .expect("create enabled profile without a credential");

        assert_eq!(
            repository
                .migrate_all_credential_storage()
                .await
                .expect("verify all stored credentials"),
            2
        );
        let scope = repository
            .verify_enabled_credentials()
            .await
            .expect("verify sidecar scope");
        assert_eq!(scope.profile_count, 2);
        assert_eq!(scope.credential_count, 1);
        assert_eq!(scope.repository_revision, 3);

        sqlx::query("UPDATE shared_connections SET name = ? WHERE id = ?")
            .bind("Changed without revision")
            .bind("enabled-without-secret")
            .execute(repository.initialized_pool().await.expect("pool"))
            .await
            .expect("tamper non-credential profile");
        let changed_scope = repository
            .credential_verification_scope()
            .await
            .expect("changed sidecar scope");
        assert_eq!(
            changed_scope.repository_revision, scope.repository_revision,
            "the digest, not revision, must catch direct profile tampering"
        );
        assert_ne!(changed_scope.profile_digest, scope.profile_digest);
    }

    #[tokio::test]
    async fn repository_identifier_persists_and_distinguishes_database_files() {
        let first_dir = TempDir::new().expect("first temp dir");
        let second_dir = TempDir::new().expect("second temp dir");
        let vault = MemoryCredentialVault::shared();
        let first = repository(&first_dir, vault.clone());
        let reopened = repository(&first_dir, vault.clone());
        let second = repository(&second_dir, vault);

        let (first_repository_id, reopened_repository_id) =
            tokio::join!(first.repository_id(), reopened.repository_id());
        assert_eq!(
            first_repository_id.expect("initialize first repository identifier"),
            reopened_repository_id.expect("initialize repository identifier concurrently")
        );
        first
            .create(config("analytics", ""), true)
            .await
            .expect("create in first repository");
        second
            .create(config("analytics", ""), true)
            .await
            .expect("create in second repository");
        let first_scope = first
            .credential_verification_scope()
            .await
            .expect("first scope");
        let reopened_scope = reopened
            .credential_verification_scope()
            .await
            .expect("reopened scope");
        let second_scope = second
            .credential_verification_scope()
            .await
            .expect("second scope");

        assert_eq!(first_scope.repository_id, reopened_scope.repository_id);
        assert_ne!(first_scope.repository_id, second_scope.repository_id);
        assert_eq!(
            first_scope.profile_digest, second_scope.profile_digest,
            "otherwise identical repositories should be distinguished by repository_id"
        );

        sqlx::query(
            "UPDATE shared_connection_meta SET value = 'not-a-uuid' WHERE key = 'repository_id'",
        )
        .execute(first.initialized_pool().await.expect("pool"))
        .await
        .expect("corrupt repository identifier");
        let error = first
            .credential_verification_scope()
            .await
            .expect_err("invalid repository identifier must fail closed");
        assert_eq!(error.code, ConnectionRepositoryErrorCode::StorageCorrupt);
    }

    #[tokio::test]
    async fn changing_endpoint_requires_a_new_password() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let repository = repository(&temp_dir, vault);
        let created = repository
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create");
        let mut changed = config("analytics", "");
        changed.host = "attacker.invalid".to_string();

        let error = repository
            .save(SaveConnectionRequest {
                config: changed,
                expected_revision: Some(created.revision),
                mcp_enabled: true,
                group_name: None,
                tags: Vec::new(),
            })
            .await
            .expect_err("old credential must not cross endpoint boundary");
        assert_eq!(
            error.code,
            ConnectionRepositoryErrorCode::CredentialReentryRequired
        );
    }

    #[tokio::test]
    async fn credential_binding_rejects_repository_endpoint_tampering() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let repository = repository(&temp_dir, vault);
        repository
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create");
        sqlx::query("UPDATE shared_connections SET host = ? WHERE id = ?")
            .bind("attacker.invalid")
            .bind("analytics")
            .execute(repository.initialized_pool().await.expect("pool"))
            .await
            .expect("tamper endpoint");

        let error = repository
            .resolve_config("analytics")
            .await
            .expect_err("AAD must reject a changed endpoint");
        assert_eq!(error.code, ConnectionRepositoryErrorCode::CredentialCorrupt);
    }

    #[tokio::test]
    async fn stale_revision_loses_without_advancing_revision() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let repository = repository(&temp_dir, vault);
        let created = repository
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create");
        let mut first_update = config("analytics", "");
        first_update.name = "Winner".to_string();
        let winner = repository
            .save(SaveConnectionRequest {
                config: first_update,
                expected_revision: Some(created.revision),
                mcp_enabled: true,
                group_name: None,
                tags: Vec::new(),
            })
            .await
            .expect("first update");
        let revision_after_winner = repository.current_revision().await.expect("revision");

        let mut stale_update = config("analytics", "");
        stale_update.name = "Loser".to_string();
        let error = repository
            .save(SaveConnectionRequest {
                config: stale_update,
                expected_revision: Some(created.revision),
                mcp_enabled: true,
                group_name: None,
                tags: Vec::new(),
            })
            .await
            .expect_err("stale update");
        assert_eq!(error.code, ConnectionRepositoryErrorCode::ProfileConflict);
        assert_eq!(
            repository.current_revision().await.expect("revision"),
            revision_after_winner
        );
        assert_eq!(repository.get("analytics").await.expect("profile"), winner);
    }

    #[tokio::test]
    async fn legacy_migration_is_idempotent() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let repository = repository(&temp_dir, vault);

        let first = repository
            .migrate_legacy(vec![config("analytics", "super-secret")])
            .await
            .expect("first migration");
        let second = repository
            .migrate_legacy(vec![config("analytics", "super-secret")])
            .await
            .expect("second migration");

        assert_eq!(first.imported, 1);
        assert!(!first.already_complete);
        assert_eq!(second.imported, 0);
        assert!(second.already_complete);
        assert_eq!(second.revision, first.revision);
        assert_eq!(
            repository
                .resolve_config("analytics")
                .await
                .expect("resolve")
                .0
                .password,
            "super-secret"
        );
    }

    #[tokio::test]
    async fn completed_legacy_migration_rejects_a_concurrent_revision_change() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = BlockingGetVault::shared();
        let repository = SharedConnectionRepository::new(
            temp_dir.path().join("connections.sqlite3"),
            vault.clone(),
        );
        let first = repository
            .migrate_legacy(vec![config("analytics", "super-secret")])
            .await
            .expect("first migration");
        vault.block_get.store(true, Ordering::SeqCst);

        let migration_repository = repository.clone();
        let migration = tokio::spawn(async move {
            migration_repository
                .migrate_legacy(vec![config("analytics", "super-secret")])
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), vault.get_entered.wait())
            .await
            .expect("completed migration must reach credential verification");
        repository
            .create(config("concurrent", ""), true)
            .await
            .expect("concurrent repository change");
        vault.release_get.wait().await;

        let error = migration
            .await
            .expect("migration task")
            .expect_err("revision change must keep the legacy data");
        assert_eq!(
            error.code,
            ConnectionRepositoryErrorCode::MigrationIncomplete
        );
        assert!(error.retryable);
        assert_eq!(error.details["migration_revision"], first.revision);
        assert_eq!(error.details["actual_revision"], first.revision + 1);
    }

    #[tokio::test]
    async fn completed_migration_rejects_conflicting_legacy_password() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let repository = repository(&temp_dir, vault);
        repository
            .migrate_legacy(vec![config("analytics", "super-secret")])
            .await
            .expect("first migration");

        let error = repository
            .migrate_legacy(vec![config("analytics", "different")])
            .await
            .expect_err("conflicting legacy data must remain");
        assert_eq!(
            error.code,
            ConnectionRepositoryErrorCode::MigrationIncomplete
        );
        assert_eq!(error.details["conflicting_ids"], json!(["analytics"]));
        assert_eq!(
            repository
                .resolve_config("analytics")
                .await
                .expect("resolve")
                .0
                .password,
            "super-secret"
        );
    }

    #[tokio::test]
    async fn first_migration_rejects_conflicting_existing_password() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let repository = repository(&temp_dir, vault);
        repository
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create existing");

        let error = repository
            .migrate_legacy(vec![config("analytics", "different")])
            .await
            .expect_err("existing password must be verified");
        assert_eq!(
            error.code,
            ConnectionRepositoryErrorCode::MigrationIncomplete
        );
        assert!(!repository
            .migration_complete()
            .await
            .expect("migration status"));
    }

    #[tokio::test]
    async fn migration_verifies_existing_and_imports_missing_profiles() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let repository = repository(&temp_dir, vault);
        repository
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create existing");

        let result = repository
            .migrate_legacy(vec![
                config("analytics", "super-secret"),
                config("warehouse", "warehouse-secret"),
            ])
            .await
            .expect("verified migration");
        assert_eq!(result.imported, 1);
        assert_eq!(result.skipped, 1);
        assert!(repository
            .migration_complete()
            .await
            .expect("migration status"));
        assert_eq!(repository.list().await.expect("profiles").len(), 2);
    }

    #[tokio::test]
    async fn metadata_reads_do_not_retry_failed_credential_cleanup() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = DeleteFailingVault::shared();
        let repository = SharedConnectionRepository::new(
            temp_dir.path().join("connections.sqlite3"),
            vault.clone(),
        );
        repository
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create existing");
        vault.set_fail_delete(true);

        let error = repository
            .create(config("analytics", "orphan-candidate"), true)
            .await
            .expect_err("duplicate create");
        assert_eq!(error.code, ConnectionRepositoryErrorCode::ProfileConflict);
        assert!(vault.contains_secret("orphan-candidate"));
        let pending =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pending_credential_cleanup")
                .fetch_one(repository.initialized_pool().await.expect("pool"))
                .await
                .expect("pending count");
        assert_eq!(pending, 1);

        vault.set_fail_delete(false);
        repository.list().await.expect("metadata-only list");
        assert!(vault.contains_secret("orphan-candidate"));
        let pending =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pending_credential_cleanup")
                .fetch_one(repository.initialized_pool().await.expect("pool"))
                .await
                .expect("pending count");
        assert_eq!(pending, 1);

        repository
            .create(config("warehouse", ""), true)
            .await
            .expect("explicit save retries cleanup");
        assert!(!vault.contains_secret("orphan-candidate"));
        assert!(vault.contains_secret("super-secret"));
        let pending =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pending_credential_cleanup")
                .fetch_one(repository.initialized_pool().await.expect("pool"))
                .await
                .expect("pending count");
        assert_eq!(pending, 0);
    }

    #[tokio::test]
    async fn staged_credential_never_expires_while_an_operation_may_resume() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = DeleteFailingVault::shared();
        let repository = SharedConnectionRepository::new(
            temp_dir.path().join("connections.sqlite3"),
            vault.clone(),
        );
        let binding = credential_binding(&config("analytics", ""));
        let reference = vault
            .put(&binding, "operation-in-progress")
            .await
            .expect("stage candidate");
        repository
            .stage_credential_cleanup(&reference)
            .await
            .expect("stage credential");

        repository.list().await.expect("metadata-only list");

        assert!(vault.contains_secret("operation-in-progress"));
        let cleanup_after = sqlx::query_scalar::<_, i64>(
            "SELECT cleanup_after FROM pending_credential_cleanup WHERE credential_ref = ?",
        )
        .bind(&reference)
        .fetch_one(repository.initialized_pool().await.expect("pool"))
        .await
        .expect("staging sentinel");
        assert_eq!(cleanup_after, CREDENTIAL_STAGING_SENTINEL);
    }

    #[tokio::test]
    async fn retry_removes_active_staging_entry_without_deleting_credential() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = DeleteFailingVault::shared();
        let repository = SharedConnectionRepository::new(
            temp_dir.path().join("connections.sqlite3"),
            vault.clone(),
        );
        repository
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create");
        let reference = vault.reference_for_secret("super-secret");
        repository
            .stage_credential_cleanup(&reference)
            .await
            .expect("stage active reference");
        sqlx::query(
            "UPDATE pending_credential_cleanup SET cleanup_after = 0 WHERE credential_ref = ?",
        )
        .bind(&reference)
        .execute(repository.initialized_pool().await.expect("pool"))
        .await
        .expect("make staging entry eligible");

        repository.list().await.expect("metadata-only list");
        repository
            .create(config("warehouse", ""), true)
            .await
            .expect("explicit save retries cleanup");
        assert!(vault.contains_secret("super-secret"));
        let pending =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pending_credential_cleanup")
                .fetch_one(repository.initialized_pool().await.expect("pool"))
                .await
                .expect("pending count");
        assert_eq!(pending, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_queues_the_reference_returned_by_the_cas_row() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = DeleteFailingVault::shared();
        let repository = SharedConnectionRepository::new(
            temp_dir.path().join("connections.sqlite3"),
            vault.clone(),
        );
        let created = repository
            .create(config("analytics", "old-secret"), true)
            .await
            .expect("create");
        let old_reference = vault.reference_for_secret("old-secret");
        let binding = credential_binding(&config("analytics", ""));
        let new_reference = vault
            .put(&binding, "new-secret")
            .await
            .expect("new credential");
        repository
            .stage_credential_cleanup(&new_reference)
            .await
            .expect("stage new credential");

        let mut writer = repository
            .initialized_pool()
            .await
            .expect("pool")
            .begin()
            .await
            .expect("writer transaction");
        let updated_revision = next_revision(&mut writer).await.expect("revision");
        sqlx::query(
            "UPDATE shared_connections SET credential_ref = ?, revision = ? \
             WHERE id = ? AND revision = ?",
        )
        .bind(&new_reference)
        .bind(updated_revision)
        .bind("analytics")
        .bind(created.revision)
        .execute(&mut *writer)
        .await
        .expect("concurrent update");
        remove_pending_credential_cleanup(&mut writer, &new_reference)
            .await
            .expect("activate credential");

        vault.set_fail_delete(true);
        let deleting_repository = repository.clone();
        let delete_task = tokio::spawn(async move {
            deleting_repository
                .delete("analytics", updated_revision)
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        writer.commit().await.expect("commit concurrent update");
        delete_task
            .await
            .expect("delete task")
            .expect("delete updated row");

        let pending = sqlx::query_scalar::<_, String>(
            "SELECT credential_ref FROM pending_credential_cleanup ORDER BY credential_ref",
        )
        .fetch_all(repository.initialized_pool().await.expect("pool"))
        .await
        .expect("pending references");
        assert_eq!(pending, vec![new_reference]);
        assert!(!pending.contains(&old_reference));
    }
}
