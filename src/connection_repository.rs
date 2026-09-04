use std::{fmt, path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::sync::{Mutex, OnceCell};

use crate::{
    connection_usage::ConnectionUsageLocks,
    credential_vault::{CredentialVaultError, CredentialVaultErrorCode, CredentialVaultHandle},
    db::{ConnectionConfig, DbType},
};

mod format;
mod migration;
mod mutations;
mod probe;
mod schema;
mod store;

use format::{is_sqlite_busy_error, MIN_SUPPORTED_SCHEMA_VERSION, SCHEMA_VERSION};
pub(crate) use probe::probe_default_native_state;

#[cfg(test)]
use format::credential_binding;
#[cfg(test)]
use mutations::CREDENTIAL_STAGING_SENTINEL;
#[cfg(test)]
use schema::{next_revision, remove_pending_credential_cleanup};

const APP_IDENTIFIER: &str = "com.astesia.app";
const DATABASE_FILENAME: &str = "connections.sqlite3";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeStateProbe {
    Fresh,
    Ready { schema_version: i64 },
}

impl SharedConnectionProfile {
    pub fn public_config(&self) -> ConnectionConfig {
        ConnectionConfig {
            id: self.id.clone(),
            name: self.name.clone(),
            db_type: self.db_type,
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
    UnsupportedSchema,
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
            Self::UnsupportedSchema => "unsupported_schema",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionRepositoryError {
    pub code: ConnectionRepositoryErrorCode,
    pub message: String,
    pub remediation: String,
    pub retryable: bool,
    pub details: Box<Value>,
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
            details: Box::new(json!({})),
        }
    }

    fn with_details(mut self, details: Value) -> Self {
        self.details = Box::new(details);
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

    fn unsupported_schema(version: i64) -> Self {
        let direction = if version > SCHEMA_VERSION {
            "newer"
        } else {
            "older"
        };
        Self::new(
            ConnectionRepositoryErrorCode::UnsupportedSchema,
            format!(
                "共享连接仓库版本 {version} 不在当前支持范围 {MIN_SUPPORTED_SCHEMA_VERSION}–{SCHEMA_VERSION} 内"
            ),
            if version > SCHEMA_VERSION {
                "请升级 Astesia；不要用旧版本打开由新版本创建的连接仓库。"
            } else {
                "请使用创建该仓库的 Astesia 版本先完成升级，或从受支持版本的备份恢复；不要让当前版本初始化此文件。"
            },
        )
        .with_details(json!({
            "kind": "unsupported_schema",
            "direction": direction,
            "schema_version": version,
            "minimum_supported_schema_version": MIN_SUPPORTED_SCHEMA_VERSION,
            "supported_schema_version": SCHEMA_VERSION,
        }))
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
        if is_sqlite_busy_error(&error) {
            return Self::new(
                ConnectionRepositoryErrorCode::StorageBusy,
                format!("无法{operation}：连接仓库正被其他 Astesia 进程占用"),
                "请稍后重试；不要同时重复提交相同修改。",
            );
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
            details: Box::new(json!({})),
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

pub fn default_database_path() -> Result<PathBuf, ConnectionRepositoryError> {
    let data_dir = dirs::data_dir().ok_or_else(|| {
        ConnectionRepositoryError::storage_unavailable("无法确定当前用户的 Astesia 数据目录")
    })?;
    Ok(data_dir.join(APP_IDENTIFIER).join(DATABASE_FILENAME))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicBool, Ordering},
            Mutex as StdMutex,
        },
        time::Duration,
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
    async fn changing_a_profile_to_sqlite_removes_its_unused_credential() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let repository = repository(&temp_dir, vault.clone());
        let created = repository
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create credentialed profile");
        let mut sqlite = config("analytics", "");
        sqlite.db_type = DbType::SQLite;
        sqlite.host = ":memory:".to_string();
        sqlite.port = 0;
        sqlite.username.clear();
        sqlite.database = None;

        let updated = repository
            .save(SaveConnectionRequest {
                config: sqlite,
                expected_revision: Some(created.revision),
                mcp_enabled: created.mcp_enabled,
                group_name: created.group_name,
                tags: created.tags,
            })
            .await
            .expect("change profile to SQLite");

        assert_eq!(updated.db_type, DbType::SQLite);
        assert!(!updated.has_credential);
        assert!(!vault.contains_secret("super-secret"));
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
