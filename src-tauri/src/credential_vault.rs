use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use ring::{
    aead::{self, Aad, LessSafeKey, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use serde::Serialize;
use sqlx::{sqlite::SqliteConnectOptions, Connection, Row, SqliteConnection};
use uuid::Uuid;
use zeroize::Zeroizing;

const CREDENTIAL_SERVICE: &str = "com.astesia.app.database";
const MASTER_CREDENTIAL_SERVICE: &str = "com.astesia.app.credential-vault";
const MASTER_CREDENTIAL_ACCOUNT: &str = "master-key-v1";
#[cfg(target_os = "macos")]
const PROTECTED_MASTER_CREDENTIAL_ACCOUNT: &str = "master-key-v2-user-presence";
const MASTER_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const AUTH_TAG_LEN: usize = 16;
const DATABASE_DIRECTORY: &str = "com.astesia.app";
const DATABASE_FILENAME: &str = "credential-vault.sqlite3";
const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(120);
const ENCRYPTION_INSERT_ATTEMPTS: usize = 8;
const CREATE_SCHEMA_SQL: &str = "\
CREATE TABLE IF NOT EXISTS credential_envelopes (
    reference TEXT PRIMARY KEY NOT NULL,
    nonce BLOB NOT NULL CHECK(length(nonce) = 12),
    ciphertext BLOB NOT NULL CHECK(length(ciphertext) >= 16),
    legacy_cleanup_pending INTEGER NOT NULL DEFAULT 0
        CHECK(legacy_cleanup_pending IN (0, 1))
)";
const CREATE_NONCE_INDEX_SQL: &str = "\
CREATE UNIQUE INDEX IF NOT EXISTS credential_envelopes_nonce_unique
ON credential_envelopes(nonce)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialVaultErrorCode {
    Missing,
    MigrationRequired,
    StoreUnavailable,
    AccessDenied,
    Corrupt,
    Invalid,
}

impl CredentialVaultErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "credential_missing",
            Self::MigrationRequired => "credential_migration_required",
            Self::StoreUnavailable => "credential_store_unavailable",
            Self::AccessDenied => "credential_access_denied",
            Self::Corrupt => "credential_corrupt",
            Self::Invalid => "credential_invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialVaultError {
    pub code: CredentialVaultErrorCode,
    pub message: String,
    pub remediation: String,
}

impl CredentialVaultError {
    fn new(code: CredentialVaultErrorCode, operation: &str) -> Self {
        let (message, remediation) = match code {
            CredentialVaultErrorCode::Missing => (
                format!("无法{operation}：该连接没有可用的已保存凭据"),
                "请在 Astesia App 中重新输入并保存该连接的密码。".to_string(),
            ),
            CredentialVaultErrorCode::MigrationRequired => (
                format!("无法{operation}：旧版凭据尚未完成安全迁移"),
                "请先打开 Astesia App 并完成强制凭据迁移，然后重新启动 MCP 客户端。STDIO 不会直接读取或删除旧版系统钥匙串条目。"
                    .to_string(),
            ),
            CredentialVaultErrorCode::StoreUnavailable => (
                format!("无法{operation}：操作系统凭据库不可用"),
                platform_remediation(),
            ),
            CredentialVaultErrorCode::AccessDenied => (
                format!("无法{operation}：操作系统凭据库已锁定或拒绝访问"),
                "请解锁当前用户的系统凭据库、允许 Astesia 访问后重试。".to_string(),
            ),
            CredentialVaultErrorCode::Corrupt => (
                format!("无法{operation}：系统凭据库中的数据损坏或格式不受支持"),
                "请在 Astesia App 中重新保存该连接的密码。".to_string(),
            ),
            CredentialVaultErrorCode::Invalid => (
                format!("无法{operation}：凭据标识或内容不符合系统凭据库要求"),
                "请检查连接标识和密码长度后重试。".to_string(),
            ),
        };
        Self {
            code,
            message,
            remediation,
        }
    }

    fn task_failed(operation: &str) -> Self {
        Self::new(CredentialVaultErrorCode::StoreUnavailable, operation)
    }

    fn master_missing(operation: &str) -> Self {
        Self {
            code: CredentialVaultErrorCode::Corrupt,
            message: format!(
                "无法{operation}：加密凭据保险库已存在，但操作系统凭据库中的主密钥缺失"
            ),
            remediation:
                "Astesia 已拒绝生成替代主密钥，以免覆盖现有密文。请恢复系统凭据库中的 Astesia 主密钥，或在 App 中删除无法解密的连接后重新保存凭据。"
                    .to_string(),
        }
    }

    fn master_changed(operation: &str) -> Self {
        Self {
            code: CredentialVaultErrorCode::Corrupt,
            message: format!("无法{operation}：操作系统凭据库中的主密钥与当前加密凭据保险库不匹配"),
            remediation:
                "Astesia 已拒绝写入新密文。请关闭其他 Astesia 进程并恢复正确的系统主密钥后重试。"
                    .to_string(),
        }
    }
}

impl fmt::Display for CredentialVaultError {
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

impl std::error::Error for CredentialVaultError {}

#[async_trait]
pub trait CredentialVault: Send + Sync {
    async fn put(&self, binding: &[u8], secret: &str) -> Result<String, CredentialVaultError>;
    async fn get(&self, reference: &str, binding: &[u8]) -> Result<String, CredentialVaultError>;
    async fn delete(&self, reference: &str) -> Result<(), CredentialVaultError>;
}

pub type CredentialVaultHandle = Arc<dyn CredentialVault>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyCredentialMode {
    MigrateInApp,
    RejectInSidecar,
}

trait KeyringBackend: Send + Sync {
    fn get_master(&self) -> keyring::Result<Vec<u8>>;
    fn set_master(&self, secret: &[u8]) -> keyring::Result<()>;
    fn get_legacy(&self, reference: &str) -> keyring::Result<String>;
    fn delete_legacy(&self, reference: &str) -> keyring::Result<()>;
}

#[derive(Debug, Default)]
struct PlatformKeyringBackend;

impl KeyringBackend for PlatformKeyringBackend {
    fn get_master(&self) -> keyring::Result<Vec<u8>> {
        #[cfg(target_os = "macos")]
        {
            match get_macos_protected_master() {
                Ok(secret) => Ok(secret),
                Err(keyring::Error::NoEntry) => {
                    // The classic shared item is intentionally retained as a
                    // migration bridge: the App and the independently signed
                    // sidecar have separate data-protection keychain scopes.
                    // Each process imports it into its own user-presence item
                    // only when that process first needs a database secret.
                    let secret = master_platform_entry()?.get_secret()?;
                    set_macos_protected_master(&secret)?;
                    Ok(secret)
                }
                Err(error) => Err(error),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            master_platform_entry()?.get_secret()
        }
    }

    fn set_master(&self, secret: &[u8]) -> keyring::Result<()> {
        #[cfg(target_os = "macos")]
        {
            // Write the cross-process migration bridge first. If creating the
            // protected item fails, no envelope is committed and a retry can
            // safely import this same key.
            master_platform_entry()?.set_secret(secret)?;
            set_macos_protected_master(secret)
        }
        #[cfg(not(target_os = "macos"))]
        {
            master_platform_entry()?.set_secret(secret)
        }
    }

    fn get_legacy(&self, reference: &str) -> keyring::Result<String> {
        platform_entry(reference)?.get_password()
    }

    fn delete_legacy(&self, reference: &str) -> keyring::Result<()> {
        platform_entry(reference)?.delete_credential()
    }
}

pub struct SystemCredentialVault {
    database_path: Option<PathBuf>,
    keyring: Arc<dyn KeyringBackend>,
    legacy_mode: LegacyCredentialMode,
    operation_lock: tokio::sync::Mutex<()>,
    master_key: tokio::sync::OnceCell<Zeroizing<[u8; MASTER_KEY_LEN]>>,
}

impl fmt::Debug for SystemCredentialVault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SystemCredentialVault")
            .field("database_path", &self.database_path)
            .field("legacy_mode", &self.legacy_mode)
            .field("master_key_loaded", &self.master_key.initialized())
            .finish_non_exhaustive()
    }
}

impl Default for SystemCredentialVault {
    fn default() -> Self {
        Self {
            database_path: default_database_path(),
            keyring: Arc::new(PlatformKeyringBackend),
            legacy_mode: LegacyCredentialMode::MigrateInApp,
            operation_lock: tokio::sync::Mutex::new(()),
            master_key: tokio::sync::OnceCell::new(),
        }
    }
}

impl SystemCredentialVault {
    pub fn shared() -> CredentialVaultHandle {
        Arc::new(Self::default())
    }

    pub fn shared_strict() -> CredentialVaultHandle {
        Arc::new(Self {
            legacy_mode: LegacyCredentialMode::RejectInSidecar,
            ..Self::default()
        })
    }

    #[cfg(test)]
    fn with_backend(database_path: PathBuf, keyring: Arc<dyn KeyringBackend>) -> Self {
        Self::with_backend_and_mode(database_path, keyring, LegacyCredentialMode::MigrateInApp)
    }

    #[cfg(test)]
    fn with_backend_and_mode(
        database_path: PathBuf,
        keyring: Arc<dyn KeyringBackend>,
        legacy_mode: LegacyCredentialMode,
    ) -> Self {
        Self {
            database_path: Some(database_path),
            keyring,
            legacy_mode,
            operation_lock: tokio::sync::Mutex::new(()),
            master_key: tokio::sync::OnceCell::new(),
        }
    }

    fn database_path(&self, operation: &str) -> Result<&Path, CredentialVaultError> {
        self.database_path.as_deref().ok_or_else(|| {
            CredentialVaultError::new(CredentialVaultErrorCode::StoreUnavailable, operation)
        })
    }

    async fn connect_database(
        &self,
        operation: &str,
    ) -> Result<SqliteConnection, CredentialVaultError> {
        let path = self.database_path(operation)?;
        let parent = path.parent().ok_or_else(|| {
            CredentialVaultError::new(CredentialVaultErrorCode::Invalid, operation)
        })?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| CredentialVaultError::task_failed(operation))?;

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .busy_timeout(DATABASE_BUSY_TIMEOUT);
        let mut connection = SqliteConnection::connect_with(&options)
            .await
            .map_err(|error| map_database_error(error, operation))?;
        sqlx::query(CREATE_SCHEMA_SQL)
            .execute(&mut connection)
            .await
            .map_err(|error| map_database_error(error, operation))?;
        sqlx::query(CREATE_NONCE_INDEX_SQL)
            .execute(&mut connection)
            .await
            .map_err(|error| map_database_error(error, operation))?;
        Ok(connection)
    }

    async fn master_key(
        &self,
        operation: &'static str,
    ) -> Result<&[u8; MASTER_KEY_LEN], CredentialVaultError> {
        let key = self
            .master_key
            .get_or_try_init(|| self.bootstrap_master(operation))
            .await?;
        Ok(key)
    }

    async fn verified_master_for_write(
        &self,
        operation: &'static str,
    ) -> Result<Zeroizing<[u8; MASTER_KEY_LEN]>, CredentialVaultError> {
        if self.master_key.initialized() {
            return self.verify_cached_master_for_write(operation).await;
        }

        // A cold write loads (or creates) the OS-backed master only once. On
        // macOS this avoids presenting two local-authentication sheets for one
        // explicit save while retaining revalidation for later writes.
        let key = self.master_key(operation).await?;
        Ok(Zeroizing::new(*key))
    }

    async fn verify_cached_master_for_write(
        &self,
        operation: &'static str,
    ) -> Result<Zeroizing<[u8; MASTER_KEY_LEN]>, CredentialVaultError> {
        let cached = self.master_key.get().ok_or_else(|| {
            CredentialVaultError::new(CredentialVaultErrorCode::StoreUnavailable, operation)
        })?;
        let keyring = self.keyring.clone();
        let stored = tokio::task::spawn_blocking(move || keyring.get_master())
            .await
            .map_err(|_| CredentialVaultError::task_failed(operation))?;
        let stored = match stored {
            Ok(secret) => decode_master_key(secret, operation)?,
            Err(keyring::Error::NoEntry) => {
                return Err(CredentialVaultError::master_missing(operation));
            }
            Err(error) => return Err(map_keyring_error(error, operation)),
        };
        if cached.as_ref() != stored.as_ref() {
            return Err(CredentialVaultError::master_changed(operation));
        }
        Ok(stored)
    }

    async fn bootstrap_master(
        &self,
        operation: &'static str,
    ) -> Result<Zeroizing<[u8; MASTER_KEY_LEN]>, CredentialVaultError> {
        let mut connection = self.connect_database(operation).await?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut connection)
            .await
            .map_err(|error| map_database_error(error, operation))?;

        let result = self
            .bootstrap_master_in_transaction(&mut connection, operation)
            .await;
        match result {
            Ok(key) => {
                if let Err(error) = sqlx::query("COMMIT").execute(&mut connection).await {
                    let _ = sqlx::query("ROLLBACK").execute(&mut connection).await;
                    return Err(map_database_error(error, operation));
                }
                Ok(key)
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut connection).await;
                Err(error)
            }
        }
    }

    async fn bootstrap_master_in_transaction(
        &self,
        connection: &mut SqliteConnection,
        operation: &'static str,
    ) -> Result<Zeroizing<[u8; MASTER_KEY_LEN]>, CredentialVaultError> {
        let encrypted_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM credential_envelopes")
                .fetch_one(&mut *connection)
                .await
                .map_err(|error| map_database_error(error, operation))?;

        let keyring = self.keyring.clone();
        let stored = tokio::task::spawn_blocking(move || keyring.get_master())
            .await
            .map_err(|_| CredentialVaultError::task_failed(operation))?;
        match stored {
            Ok(secret) => decode_master_key(secret, operation),
            Err(keyring::Error::NoEntry) if encrypted_count == 0 => {
                let key = generate_master_key(operation)?;
                let keyring = self.keyring.clone();
                let to_store = Zeroizing::new(key.to_vec());
                tokio::task::spawn_blocking(move || keyring.set_master(&to_store))
                    .await
                    .map_err(|_| CredentialVaultError::task_failed(operation))?
                    .map_err(|error| map_keyring_error(error, operation))?;
                Ok(key)
            }
            Err(keyring::Error::NoEntry) => Err(CredentialVaultError::master_missing(operation)),
            Err(error) => Err(map_keyring_error(error, operation)),
        }
    }

    async fn load_envelope(
        &self,
        reference: &str,
        operation: &str,
    ) -> Result<Option<EncryptedEnvelope>, CredentialVaultError> {
        let mut connection = self.connect_database(operation).await?;
        load_envelope_from_connection(&mut connection, reference, operation).await
    }

    async fn decrypt_envelope(
        &self,
        envelope: EncryptedEnvelope,
        binding: &[u8],
        operation: &'static str,
    ) -> Result<String, CredentialVaultError> {
        let key = self.master_key(operation).await?;
        decrypt_secret(
            key,
            &envelope.nonce,
            &envelope.ciphertext,
            binding,
            operation,
        )
    }

    async fn cleanup_legacy_if_pending(
        &self,
        reference: &str,
        pending: bool,
    ) -> Result<(), CredentialVaultError> {
        if !pending {
            return Ok(());
        }
        if self.legacy_mode == LegacyCredentialMode::RejectInSidecar {
            return Err(CredentialVaultError::new(
                CredentialVaultErrorCode::MigrationRequired,
                "完成旧版凭据迁移",
            ));
        }

        const OPERATION: &str = "完成旧版凭据迁移";
        let mut connection = self.connect_database(OPERATION).await?;
        begin_immediate(&mut connection, OPERATION).await?;
        let result = async {
            let envelope =
                load_envelope_from_connection(&mut connection, reference, OPERATION).await?;
            if !envelope.is_some_and(|envelope| envelope.legacy_cleanup_pending) {
                return Ok(());
            }

            let keyring = self.keyring.clone();
            let reference_for_keyring = reference.to_string();
            tokio::task::spawn_blocking(move || {
                match keyring.delete_legacy(&reference_for_keyring) {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(error) => Err(map_keyring_error(error, "清理旧版数据库凭据")),
                }
            })
            .await
            .map_err(|_| CredentialVaultError::task_failed("清理旧版数据库凭据"))??;

            sqlx::query(
                "UPDATE credential_envelopes \
                 SET legacy_cleanup_pending = 0 \
                 WHERE reference = ? AND legacy_cleanup_pending = 1",
            )
            .bind(reference)
            .execute(&mut connection)
            .await
            .map_err(|error| map_database_error(error, OPERATION))?;
            Ok(())
        }
        .await;
        finish_transaction(&mut connection, result, OPERATION).await
    }

    async fn migrate_legacy(
        &self,
        reference: &str,
        binding: &[u8],
        operation: &'static str,
    ) -> Result<String, CredentialVaultError> {
        if self.legacy_mode == LegacyCredentialMode::RejectInSidecar {
            return Err(CredentialVaultError::new(
                CredentialVaultErrorCode::MigrationRequired,
                operation,
            ));
        }

        let _ = self.master_key(operation).await?;
        let mut connection = self.connect_database(operation).await?;
        begin_immediate(&mut connection, operation).await?;

        let result = async {
            if let Some(existing) =
                load_envelope_from_connection(&mut connection, reference, operation).await?
            {
                return Ok((Some(existing), None));
            }

            let keyring = self.keyring.clone();
            let legacy_reference = reference.to_string();
            let legacy_secret =
                tokio::task::spawn_blocking(move || keyring.get_legacy(&legacy_reference))
                    .await
                    .map_err(|_| CredentialVaultError::task_failed(operation))?
                    .map_err(|error| map_keyring_error(error, operation))?;
            let legacy_secret = Zeroizing::new(legacy_secret);
            // A legacy Keychain prompt can stay open indefinitely. Verify the
            // OS-backed master after that prompt, immediately before writing.
            let key = self.verify_cached_master_for_write(operation).await?;

            insert_envelope_with_retry(
                &mut connection,
                reference,
                &key,
                binding,
                &legacy_secret,
                true,
                operation,
            )
            .await?;
            Ok((None, Some(legacy_secret)))
        }
        .await;
        let (stored, legacy_secret) =
            finish_transaction(&mut connection, result, operation).await?;

        match stored {
            Some(stored) => {
                let pending = stored.legacy_cleanup_pending;
                let result = self.decrypt_envelope(stored, binding, operation).await?;
                self.cleanup_legacy_if_pending(reference, pending).await?;
                Ok(result)
            }
            None => {
                self.cleanup_legacy_if_pending(reference, true).await?;
                Ok(legacy_secret
                    .expect("new legacy envelope must retain its plaintext until cleanup")
                    .to_string())
            }
        }
    }
}

#[async_trait]
impl CredentialVault for SystemCredentialVault {
    async fn put(&self, binding: &[u8], secret: &str) -> Result<String, CredentialVaultError> {
        let _operation = self.operation_lock.lock().await;
        let mut connection = self.connect_database("保存数据库凭据").await?;
        let key = self.verified_master_for_write("保存数据库凭据").await?;
        for _ in 0..ENCRYPTION_INSERT_ATTEMPTS {
            let reference = Uuid::new_v4().to_string();
            let inserted = insert_envelope_once(
                &mut connection,
                &reference,
                &key,
                binding,
                secret,
                false,
                "保存数据库凭据",
            )
            .await?;
            if inserted {
                return Ok(reference);
            }
        }
        Err(CredentialVaultError::new(
            CredentialVaultErrorCode::Invalid,
            "保存数据库凭据",
        ))
    }

    async fn get(&self, reference: &str, binding: &[u8]) -> Result<String, CredentialVaultError> {
        let _operation = self.operation_lock.lock().await;
        if let Some(envelope) = self.load_envelope(reference, "读取数据库凭据").await? {
            if envelope.legacy_cleanup_pending
                && self.legacy_mode == LegacyCredentialMode::RejectInSidecar
            {
                return Err(CredentialVaultError::new(
                    CredentialVaultErrorCode::MigrationRequired,
                    "读取数据库凭据",
                ));
            }
            let pending = envelope.legacy_cleanup_pending;
            let secret = self
                .decrypt_envelope(envelope, binding, "读取数据库凭据")
                .await?;
            self.cleanup_legacy_if_pending(reference, pending).await?;
            return Ok(secret);
        }
        if self.legacy_mode == LegacyCredentialMode::RejectInSidecar {
            return Err(CredentialVaultError::new(
                CredentialVaultErrorCode::MigrationRequired,
                "读取数据库凭据",
            ));
        }
        self.migrate_legacy(reference, binding, "读取数据库凭据")
            .await
    }

    async fn delete(&self, reference: &str) -> Result<(), CredentialVaultError> {
        let _operation = self.operation_lock.lock().await;
        const OPERATION: &str = "删除数据库凭据";
        let mut connection = self.connect_database(OPERATION).await?;
        begin_immediate(&mut connection, OPERATION).await?;
        let result = async {
            let envelope =
                load_envelope_from_connection(&mut connection, reference, OPERATION).await?;
            if self.legacy_mode == LegacyCredentialMode::RejectInSidecar
                && envelope
                    .as_ref()
                    .is_none_or(|envelope| envelope.legacy_cleanup_pending)
            {
                return Err(CredentialVaultError::new(
                    CredentialVaultErrorCode::MigrationRequired,
                    OPERATION,
                ));
            }

            if envelope
                .as_ref()
                .is_none_or(|envelope| envelope.legacy_cleanup_pending)
            {
                let keyring = self.keyring.clone();
                let legacy_reference = reference.to_string();
                tokio::task::spawn_blocking(move || {
                    match keyring.delete_legacy(&legacy_reference) {
                        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                        Err(error) => Err(map_keyring_error(error, OPERATION)),
                    }
                })
                .await
                .map_err(|_| CredentialVaultError::task_failed(OPERATION))??;
            }

            sqlx::query("DELETE FROM credential_envelopes WHERE reference = ?")
                .bind(reference)
                .execute(&mut connection)
                .await
                .map_err(|error| map_database_error(error, OPERATION))?;
            Ok(())
        }
        .await;
        finish_transaction(&mut connection, result, OPERATION).await
    }
}

#[derive(Debug)]
struct EncryptedEnvelope {
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
    legacy_cleanup_pending: bool,
}

fn default_database_path() -> Option<PathBuf> {
    dirs::data_dir().map(|data| data.join(DATABASE_DIRECTORY).join(DATABASE_FILENAME))
}

async fn begin_immediate(
    connection: &mut SqliteConnection,
    operation: &str,
) -> Result<(), CredentialVaultError> {
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await
        .map_err(|error| map_database_error(error, operation))?;
    Ok(())
}

async fn finish_transaction<T>(
    connection: &mut SqliteConnection,
    result: Result<T, CredentialVaultError>,
    operation: &str,
) -> Result<T, CredentialVaultError> {
    match result {
        Ok(value) => {
            if let Err(error) = sqlx::query("COMMIT").execute(&mut *connection).await {
                let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
                return Err(map_database_error(error, operation));
            }
            Ok(value)
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            Err(error)
        }
    }
}

async fn insert_envelope_once(
    connection: &mut SqliteConnection,
    reference: &str,
    master: &[u8; MASTER_KEY_LEN],
    binding: &[u8],
    secret: &str,
    legacy_cleanup_pending: bool,
    operation: &str,
) -> Result<bool, CredentialVaultError> {
    let (nonce, ciphertext) = encrypt_secret(master, binding, secret, operation)?;
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO credential_envelopes \
         (reference, nonce, ciphertext, legacy_cleanup_pending) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(reference)
    .bind(nonce.to_vec())
    .bind(ciphertext)
    .bind(i64::from(legacy_cleanup_pending))
    .execute(&mut *connection)
    .await
    .map_err(|error| map_database_error(error, operation))?;
    Ok(inserted.rows_affected() == 1)
}

async fn insert_envelope_with_retry(
    connection: &mut SqliteConnection,
    reference: &str,
    master: &[u8; MASTER_KEY_LEN],
    binding: &[u8],
    secret: &str,
    legacy_cleanup_pending: bool,
    operation: &str,
) -> Result<(), CredentialVaultError> {
    for _ in 0..ENCRYPTION_INSERT_ATTEMPTS {
        if insert_envelope_once(
            connection,
            reference,
            master,
            binding,
            secret,
            legacy_cleanup_pending,
            operation,
        )
        .await?
        {
            return Ok(());
        }
    }
    Err(CredentialVaultError::new(
        CredentialVaultErrorCode::Invalid,
        operation,
    ))
}

async fn load_envelope_from_connection(
    connection: &mut SqliteConnection,
    reference: &str,
    operation: &str,
) -> Result<Option<EncryptedEnvelope>, CredentialVaultError> {
    let row = sqlx::query(
        "SELECT nonce, ciphertext, legacy_cleanup_pending \
         FROM credential_envelopes \
         WHERE reference = ?",
    )
    .bind(reference)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| map_database_error(error, operation))?;

    row.map(|row| {
        let nonce: Vec<u8> = row
            .try_get("nonce")
            .map_err(|_| CredentialVaultError::new(CredentialVaultErrorCode::Corrupt, operation))?;
        let ciphertext: Vec<u8> = row
            .try_get("ciphertext")
            .map_err(|_| CredentialVaultError::new(CredentialVaultErrorCode::Corrupt, operation))?;
        let legacy_cleanup_pending: i64 = row
            .try_get("legacy_cleanup_pending")
            .map_err(|_| CredentialVaultError::new(CredentialVaultErrorCode::Corrupt, operation))?;
        if nonce.len() != NONCE_LEN
            || ciphertext.len() < AUTH_TAG_LEN
            || !matches!(legacy_cleanup_pending, 0 | 1)
        {
            return Err(CredentialVaultError::new(
                CredentialVaultErrorCode::Corrupt,
                operation,
            ));
        }
        Ok(EncryptedEnvelope {
            nonce,
            ciphertext,
            legacy_cleanup_pending: legacy_cleanup_pending == 1,
        })
    })
    .transpose()
}

fn generate_master_key(
    operation: &str,
) -> Result<Zeroizing<[u8; MASTER_KEY_LEN]>, CredentialVaultError> {
    let mut key = Zeroizing::new([0_u8; MASTER_KEY_LEN]);
    SystemRandom::new()
        .fill(&mut *key)
        .map_err(|_| CredentialVaultError::task_failed(operation))?;
    Ok(key)
}

fn decode_master_key(
    secret: Vec<u8>,
    operation: &str,
) -> Result<Zeroizing<[u8; MASTER_KEY_LEN]>, CredentialVaultError> {
    let secret = Zeroizing::new(secret);
    let key: [u8; MASTER_KEY_LEN] = secret
        .as_slice()
        .try_into()
        .map_err(|_| CredentialVaultError::new(CredentialVaultErrorCode::Corrupt, operation))?;
    Ok(Zeroizing::new(key))
}

fn encryption_key(
    master: &[u8; MASTER_KEY_LEN],
    operation: &str,
) -> Result<LessSafeKey, CredentialVaultError> {
    UnboundKey::new(&aead::AES_256_GCM, master)
        .map(LessSafeKey::new)
        .map_err(|_| CredentialVaultError::new(CredentialVaultErrorCode::Invalid, operation))
}

fn encrypt_secret(
    master: &[u8; MASTER_KEY_LEN],
    binding: &[u8],
    secret: &str,
    operation: &str,
) -> Result<([u8; NONCE_LEN], Vec<u8>), CredentialVaultError> {
    let mut nonce_bytes = [0_u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|_| CredentialVaultError::task_failed(operation))?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let key = encryption_key(master, operation)?;
    let mut ciphertext = Zeroizing::new(secret.as_bytes().to_vec());
    key.seal_in_place_append_tag(nonce, Aad::from(binding), &mut *ciphertext)
        .map_err(|_| CredentialVaultError::new(CredentialVaultErrorCode::Invalid, operation))?;
    Ok((nonce_bytes, ciphertext.to_vec()))
}

fn decrypt_secret(
    master: &[u8; MASTER_KEY_LEN],
    nonce: &[u8],
    ciphertext: &[u8],
    binding: &[u8],
    operation: &str,
) -> Result<String, CredentialVaultError> {
    let nonce_bytes: [u8; NONCE_LEN] = nonce
        .try_into()
        .map_err(|_| CredentialVaultError::new(CredentialVaultErrorCode::Corrupt, operation))?;
    if ciphertext.len() < AUTH_TAG_LEN {
        return Err(CredentialVaultError::new(
            CredentialVaultErrorCode::Corrupt,
            operation,
        ));
    }

    let key = encryption_key(master, operation)?;
    let mut plaintext = Zeroizing::new(ciphertext.to_vec());
    let opened = key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::from(binding),
            &mut plaintext,
        )
        .map_err(|_| CredentialVaultError::new(CredentialVaultErrorCode::Corrupt, operation))?;
    String::from_utf8(opened.to_vec())
        .map_err(|_| CredentialVaultError::new(CredentialVaultErrorCode::Corrupt, operation))
}

fn map_database_error(error: sqlx::Error, operation: &str) -> CredentialVaultError {
    let code = match &error {
        sqlx::Error::Database(database)
            if database
                .code()
                .is_some_and(|code| matches!(code.as_ref(), "11" | "26")) =>
        {
            CredentialVaultErrorCode::Corrupt
        }
        _ => CredentialVaultErrorCode::StoreUnavailable,
    };
    log::warn!("Credential vault database operation failed: {error}");
    CredentialVaultError::new(code, operation)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn platform_entry(reference: &str) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(CREDENTIAL_SERVICE, reference)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn master_platform_entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(MASTER_CREDENTIAL_SERVICE, MASTER_CREDENTIAL_ACCOUNT)
}

#[cfg(target_os = "macos")]
fn get_macos_protected_master() -> keyring::Result<Vec<u8>> {
    use security_framework::passwords::{generic_password, PasswordOptions};

    let mut options = PasswordOptions::new_generic_password(
        MASTER_CREDENTIAL_SERVICE,
        PROTECTED_MASTER_CREDENTIAL_ACCOUNT,
    );
    options.use_protected_keychain();
    generic_password(options).map_err(map_macos_keychain_error)
}

#[cfg(target_os = "macos")]
fn set_macos_protected_master(secret: &[u8]) -> keyring::Result<()> {
    use security_framework::passwords::{set_generic_password_options, PasswordOptions};

    let mut options = PasswordOptions::new_generic_password(
        MASTER_CREDENTIAL_SERVICE,
        PROTECTED_MASTER_CREDENTIAL_ACCOUNT,
    );
    options.use_protected_keychain();
    options.set_access_control_options(macos_master_access_control());
    set_generic_password_options(secret, options).map_err(map_macos_keychain_error)
}

#[cfg(target_os = "macos")]
fn macos_master_access_control() -> security_framework::passwords::AccessControlOptions {
    use security_framework::passwords::AccessControlOptions;

    // `watch` is deprecated in favour of companion authentication on newer
    // SDKs, but remains the backwards-compatible Security.framework flag.
    // The OR constraint lets macOS choose Touch ID, Apple Watch, or the local
    // account password according to the user's System Settings.
    AccessControlOptions::BIOMETRY_ANY
        | AccessControlOptions::WATCH
        | AccessControlOptions::DEVICE_PASSCODE
        | AccessControlOptions::OR
}

#[cfg(target_os = "macos")]
fn map_macos_keychain_error(error: security_framework::base::Error) -> keyring::Error {
    match error.code() {
        -25_300 => keyring::Error::NoEntry,
        -25_291 | -34_018 => keyring::Error::NoStorageAccess(Box::new(error)),
        _ => keyring::Error::PlatformFailure(Box::new(error)),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_entry(_reference: &str) -> keyring::Result<keyring::Entry> {
    unsupported_platform_error()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn master_platform_entry() -> keyring::Result<keyring::Entry> {
    unsupported_platform_error()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn unsupported_platform_error<T>() -> keyring::Result<T> {
    Err(keyring::Error::NoStorageAccess(Box::new(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Astesia has no credential-store backend for this operating system",
        ),
    )))
}

fn map_keyring_error(error: keyring::Error, operation: &str) -> CredentialVaultError {
    let code = match error {
        keyring::Error::NoEntry => CredentialVaultErrorCode::Missing,
        keyring::Error::NoStorageAccess(_) if no_storage_access_means_unavailable() => {
            CredentialVaultErrorCode::StoreUnavailable
        }
        keyring::Error::NoStorageAccess(_) => CredentialVaultErrorCode::AccessDenied,
        keyring::Error::PlatformFailure(platform_error)
            if platform_failure_is_access_denied(platform_error.as_ref()) =>
        {
            CredentialVaultErrorCode::AccessDenied
        }
        keyring::Error::PlatformFailure(_) => CredentialVaultErrorCode::StoreUnavailable,
        keyring::Error::BadEncoding(_) | keyring::Error::Ambiguous(_) => {
            CredentialVaultErrorCode::Corrupt
        }
        keyring::Error::TooLong(_, _) | keyring::Error::Invalid(_, _) => {
            CredentialVaultErrorCode::Invalid
        }
        _ => CredentialVaultErrorCode::StoreUnavailable,
    };
    CredentialVaultError::new(code, operation)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
const fn no_storage_access_means_unavailable() -> bool {
    true
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const fn no_storage_access_means_unavailable() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn platform_failure_is_access_denied(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> bool {
    error
        .downcast_ref::<security_framework::base::Error>()
        .is_some_and(|error| matches!(error.code(), -128 | -25_243 | -25_293 | -25_308))
}

#[cfg(not(target_os = "macos"))]
fn platform_failure_is_access_denied(
    _error: &(dyn std::error::Error + Send + Sync + 'static),
) -> bool {
    false
}

fn platform_remediation() -> String {
    #[cfg(target_os = "linux")]
    {
        return "请安装并启动兼容 Secret Service 的服务（例如 GNOME Keyring、KWallet 或 KeePassXC Secret Service），确认会话 D-Bus 可用后重试。Astesia 不会回退为明文密码文件。"
            .to_string();
    }
    #[cfg(target_os = "windows")]
    {
        return "请确认当前 Windows 用户的 Credential Manager 可用；不受支持的旧版 Windows 需要升级系统。Astesia 不会回退为明文密码文件。"
            .to_string();
    }
    #[cfg(target_os = "macos")]
    {
        return "请解锁当前用户的 macOS Keychain，并通过系统提供的 Touch ID、Apple Watch 或本机密码验证。Astesia App 与 astesia-mcp 会在各自首次实际读取凭据时请求验证；无图形登录会话无法显示系统授权界面。Astesia 不会回退为明文密码文件。"
            .to_string();
    }
    #[allow(unreachable_code)]
    "此操作系统没有 Astesia 支持的系统凭据库；请在受支持的系统上使用。Astesia 不会回退为明文密码文件。"
        .to_string()
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use super::*;

    #[derive(Default)]
    pub struct MemoryCredentialVault {
        secrets: Mutex<HashMap<String, (Vec<u8>, String)>>,
        failure: Mutex<Option<CredentialVaultErrorCode>>,
    }

    impl MemoryCredentialVault {
        pub fn shared() -> Arc<Self> {
            Arc::new(Self::default())
        }

        pub fn fail_with(&self, code: Option<CredentialVaultErrorCode>) {
            *self.failure.lock().expect("failure lock") = code;
        }

        pub fn contains_secret(&self, secret: &str) -> bool {
            self.secrets
                .lock()
                .expect("secrets lock")
                .values()
                .any(|(_, stored)| stored == secret)
        }

        fn failure(&self, operation: &str) -> Result<(), CredentialVaultError> {
            match *self.failure.lock().expect("failure lock") {
                Some(code) => Err(CredentialVaultError::new(code, operation)),
                None => Ok(()),
            }
        }
    }

    #[async_trait]
    impl CredentialVault for MemoryCredentialVault {
        async fn put(&self, binding: &[u8], secret: &str) -> Result<String, CredentialVaultError> {
            self.failure("保存数据库凭据")?;
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
            self.failure("读取数据库凭据")?;
            let secrets = self.secrets.lock().expect("secrets lock");
            let (stored_binding, secret) = secrets.get(reference).ok_or_else(|| {
                CredentialVaultError::new(CredentialVaultErrorCode::Missing, "读取数据库凭据")
            })?;
            if stored_binding != binding {
                return Err(CredentialVaultError::new(
                    CredentialVaultErrorCode::Corrupt,
                    "读取数据库凭据",
                ));
            }
            Ok(secret.clone())
        }

        async fn delete(&self, reference: &str) -> Result<(), CredentialVaultError> {
            self.failure("删除数据库凭据")?;
            self.secrets.lock().expect("secrets lock").remove(reference);
            Ok(())
        }
    }
}

#[cfg(test)]
mod vault_tests {
    use std::{
        collections::HashMap,
        fs, io,
        path::Path,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Barrier, Mutex,
        },
        time::Duration,
    };

    use sqlx::{Connection, SqliteConnection};
    use tempfile::TempDir;

    use super::{
        test_support::MemoryCredentialVault, CredentialVault, CredentialVaultErrorCode,
        KeyringBackend, LegacyCredentialMode, SystemCredentialVault,
    };

    #[derive(Default)]
    struct MockKeyring {
        master: Mutex<Option<Vec<u8>>>,
        legacy: Mutex<HashMap<String, String>>,
        master_reads: AtomicUsize,
        master_writes: AtomicUsize,
        legacy_reads: AtomicUsize,
        legacy_deletes: AtomicUsize,
        delete_failures: AtomicUsize,
        legacy_read_pause: Mutex<Option<(Arc<Barrier>, Arc<Barrier>)>>,
    }

    impl MockKeyring {
        fn insert_legacy(&self, reference: &str, secret: &str) {
            self.legacy
                .lock()
                .expect("legacy lock")
                .insert(reference.to_string(), secret.to_string());
        }

        fn clear_master(&self) {
            *self.master.lock().expect("master lock") = None;
        }

        fn fail_next_delete(&self) {
            self.delete_failures.store(1, Ordering::SeqCst);
        }

        fn pause_next_legacy_read(&self) -> (Arc<Barrier>, Arc<Barrier>) {
            let started = Arc::new(Barrier::new(2));
            let release = Arc::new(Barrier::new(2));
            *self
                .legacy_read_pause
                .lock()
                .expect("legacy read pause lock") = Some((started.clone(), release.clone()));
            (started, release)
        }

        fn has_legacy(&self, reference: &str) -> bool {
            self.legacy
                .lock()
                .expect("legacy lock")
                .contains_key(reference)
        }
    }

    impl KeyringBackend for MockKeyring {
        fn get_master(&self) -> keyring::Result<Vec<u8>> {
            self.master_reads.fetch_add(1, Ordering::SeqCst);
            self.master
                .lock()
                .expect("master lock")
                .clone()
                .ok_or(keyring::Error::NoEntry)
        }

        fn set_master(&self, secret: &[u8]) -> keyring::Result<()> {
            self.master_writes.fetch_add(1, Ordering::SeqCst);
            *self.master.lock().expect("master lock") = Some(secret.to_vec());
            Ok(())
        }

        fn get_legacy(&self, reference: &str) -> keyring::Result<String> {
            self.legacy_reads.fetch_add(1, Ordering::SeqCst);
            let secret = self
                .legacy
                .lock()
                .expect("legacy lock")
                .get(reference)
                .cloned()
                .ok_or(keyring::Error::NoEntry);
            if let Some((started, release)) = self
                .legacy_read_pause
                .lock()
                .expect("legacy read pause lock")
                .take()
            {
                started.wait();
                release.wait();
            }
            secret
        }

        fn delete_legacy(&self, reference: &str) -> keyring::Result<()> {
            self.legacy_deletes.fetch_add(1, Ordering::SeqCst);
            if self
                .delete_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(keyring::Error::NoStorageAccess(Box::new(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "mock access denied",
                ))));
            }
            self.legacy
                .lock()
                .expect("legacy lock")
                .remove(reference)
                .map(|_| ())
                .ok_or(keyring::Error::NoEntry)
        }
    }

    fn database_path(directory: &TempDir) -> std::path::PathBuf {
        directory.path().join("credential-vault.sqlite3")
    }

    fn vault(path: &Path, keyring: Arc<MockKeyring>) -> SystemCredentialVault {
        SystemCredentialVault::with_backend(path.to_path_buf(), keyring)
    }

    fn strict_vault(path: &Path, keyring: Arc<MockKeyring>) -> SystemCredentialVault {
        SystemCredentialVault::with_backend_and_mode(
            path.to_path_buf(),
            keyring,
            LegacyCredentialMode::RejectInSidecar,
        )
    }

    async fn open_database(path: &Path) -> SqliteConnection {
        SqliteConnection::connect(&format!("sqlite:{}", path.display()))
            .await
            .expect("open credential database")
    }

    #[tokio::test]
    async fn two_vault_processes_cache_master_reads_but_revalidate_each_write() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        let first = vault(&path, keyring.clone());
        let second = vault(&path, keyring.clone());

        let first_reference = first
            .put(b"binding:first", "first-password")
            .await
            .expect("first put");
        assert_eq!(
            first
                .get(&first_reference, b"binding:first")
                .await
                .expect("first get"),
            "first-password"
        );

        let second_reference = second
            .put(b"binding:second", "second-password")
            .await
            .expect("second put");
        assert_eq!(
            second
                .get(&second_reference, b"binding:second")
                .await
                .expect("second get"),
            "second-password"
        );
        assert_eq!(
            second
                .get(&first_reference, b"binding:first")
                .await
                .expect("cross-process get"),
            "first-password"
        );

        assert_eq!(keyring.master_reads.load(Ordering::SeqCst), 2);
        assert_eq!(keyring.master_writes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn database_never_contains_plaintext_passwords() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        let vault = vault(&path, keyring);
        let secret = "plaintext-must-not-appear-4f96f687";

        vault.put(b"connection-binding", secret).await.expect("put");
        let database = fs::read(path).expect("read sqlite database");
        assert!(!database
            .windows(secret.len())
            .any(|window| window == secret.as_bytes()));
    }

    #[tokio::test]
    async fn wrong_aad_and_tampered_ciphertext_fail_closed() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        let writer = vault(&path, keyring.clone());
        let reference = writer
            .put(b"correct-binding", "protected")
            .await
            .expect("put");

        let wrong_binding = writer
            .get(&reference, b"wrong-binding")
            .await
            .expect_err("wrong binding must fail");
        assert_eq!(wrong_binding.code, CredentialVaultErrorCode::Corrupt);

        let mut database = open_database(&path).await;
        sqlx::query(
            "UPDATE credential_envelopes \
             SET ciphertext = zeroblob(length(ciphertext)) \
             WHERE reference = ?",
        )
        .bind(&reference)
        .execute(&mut database)
        .await
        .expect("tamper ciphertext");
        drop(database);

        let tampered = writer
            .get(&reference, b"correct-binding")
            .await
            .expect_err("tampered ciphertext must fail");
        assert_eq!(tampered.code, CredentialVaultErrorCode::Corrupt);

        keyring.insert_legacy(&reference, "legacy-fallback-must-not-be-used");
        let restarted = vault(&path, keyring.clone());
        let fallback_error = restarted
            .get(&reference, b"correct-binding")
            .await
            .expect_err("get must not fall back after finding a corrupt envelope");
        assert_eq!(fallback_error.code, CredentialVaultErrorCode::Corrupt);
        assert_eq!(keyring.legacy_reads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn missing_master_with_ciphertext_is_never_recreated() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        let first = vault(&path, keyring.clone());
        let reference = first.put(b"binding", "protected").await.expect("put");
        assert_eq!(keyring.master_writes.load(Ordering::SeqCst), 1);

        keyring.clear_master();
        let restarted = vault(&path, keyring.clone());
        let error = restarted
            .get(&reference, b"binding")
            .await
            .expect_err("missing master must fail");
        assert_eq!(error.code, CredentialVaultErrorCode::Corrupt);
        assert!(error.message.contains("主密钥缺失"));
        assert_eq!(keyring.master_writes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cached_master_deletion_blocks_put_without_adding_an_envelope() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        let vault = vault(&path, keyring.clone());
        vault.put(b"first", "protected").await.expect("first put");

        keyring.clear_master();
        let error = vault
            .put(b"second", "must-not-be-written")
            .await
            .expect_err("cached master must be revalidated before every put");
        assert_eq!(error.code, CredentialVaultErrorCode::Corrupt);
        assert!(error.message.contains("主密钥缺失"));

        let mut database = open_database(&path).await;
        let envelope_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM credential_envelopes")
            .fetch_one(&mut database)
            .await
            .expect("count envelopes");
        assert_eq!(envelope_count, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn master_deleted_during_legacy_prompt_blocks_envelope_write() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        keyring.insert_legacy("legacy-reference", "legacy-password");
        let vault = vault(&path, keyring.clone());
        let (legacy_read_started, release_legacy_read) = keyring.pause_next_legacy_read();

        let migration =
            tokio::spawn(async move { vault.get("legacy-reference", b"binding").await });
        tokio::task::spawn_blocking(move || legacy_read_started.wait())
            .await
            .expect("wait for legacy prompt");

        keyring.clear_master();
        tokio::task::spawn_blocking(move || release_legacy_read.wait())
            .await
            .expect("release legacy prompt");

        let error = migration
            .await
            .expect("migration task")
            .expect_err("deleted master must block the legacy envelope write");
        assert_eq!(error.code, CredentialVaultErrorCode::Corrupt);
        assert!(error.message.contains("主密钥缺失"));

        let mut database = open_database(&path).await;
        let envelope_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM credential_envelopes \
             WHERE reference = 'legacy-reference'",
        )
        .fetch_one(&mut database)
        .await
        .expect("count legacy envelopes");
        assert_eq!(envelope_count, 0);
        assert!(keyring.has_legacy("legacy-reference"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_bootstrap_writes_master_once() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        let first = vault(&path, keyring.clone());
        let second = vault(&path, keyring.clone());

        let (first_result, second_result) = tokio::join!(
            first.put(b"first", "first-password"),
            second.put(b"second", "second-password")
        );
        first_result.expect("first concurrent put");
        second_result.expect("second concurrent put");

        assert_eq!(keyring.master_writes.load(Ordering::SeqCst), 1);
        assert_eq!(keyring.master_reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn strict_mode_never_reads_or_deletes_legacy_credentials() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        keyring.insert_legacy("legacy-reference", "legacy-password");
        let vault = strict_vault(&path, keyring.clone());

        let get_error = vault
            .get("legacy-reference", b"binding")
            .await
            .expect_err("strict get must require App migration");
        assert_eq!(get_error.code, CredentialVaultErrorCode::MigrationRequired);
        assert!(get_error.remediation.contains("Astesia App"));

        let delete_error = vault
            .delete("legacy-reference")
            .await
            .expect_err("strict delete must require App migration");
        assert_eq!(
            delete_error.code,
            CredentialVaultErrorCode::MigrationRequired
        );
        assert_eq!(keyring.legacy_reads.load(Ordering::SeqCst), 0);
        assert_eq!(keyring.legacy_deletes.load(Ordering::SeqCst), 0);
        assert!(keyring.has_legacy("legacy-reference"));
    }

    #[tokio::test]
    async fn strict_mode_leaves_pending_migration_untouched() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        keyring.insert_legacy("pending-reference", "legacy-password");
        keyring.fail_next_delete();
        let migratable = vault(&path, keyring.clone());
        migratable
            .get("pending-reference", b"binding")
            .await
            .expect_err("prepare a pending cleanup marker");

        let reads_before = keyring.legacy_reads.load(Ordering::SeqCst);
        let deletes_before = keyring.legacy_deletes.load(Ordering::SeqCst);
        let strict = strict_vault(&path, keyring.clone());
        let get_error = strict
            .get("pending-reference", b"binding")
            .await
            .expect_err("strict get must not clean pending legacy data");
        assert_eq!(get_error.code, CredentialVaultErrorCode::MigrationRequired);
        let delete_error = strict
            .delete("pending-reference")
            .await
            .expect_err("strict delete must not clean pending legacy data");
        assert_eq!(
            delete_error.code,
            CredentialVaultErrorCode::MigrationRequired
        );
        assert_eq!(keyring.legacy_reads.load(Ordering::SeqCst), reads_before);
        assert_eq!(
            keyring.legacy_deletes.load(Ordering::SeqCst),
            deletes_before
        );
        assert!(keyring.has_legacy("pending-reference"));

        let mut database = open_database(&path).await;
        let pending: i64 = sqlx::query_scalar(
            "SELECT legacy_cleanup_pending \
             FROM credential_envelopes WHERE reference = 'pending-reference'",
        )
        .fetch_one(&mut database)
        .await
        .expect("pending cleanup marker");
        assert_eq!(pending, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cross_instance_migration_and_delete_cannot_leave_an_orphan_envelope() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        keyring.insert_legacy("racing-reference", "legacy-password");
        let migrating_vault = vault(&path, keyring.clone());
        let deleting_vault = vault(&path, keyring.clone());
        let (legacy_read_started, release_legacy_read) = keyring.pause_next_legacy_read();

        let migration =
            tokio::spawn(async move { migrating_vault.get("racing-reference", b"binding").await });
        tokio::task::spawn_blocking(move || legacy_read_started.wait())
            .await
            .expect("wait for legacy read");

        let mut deletion =
            tokio::spawn(async move { deleting_vault.delete("racing-reference").await });
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut deletion)
                .await
                .is_err(),
            "delete must wait for the migration transaction"
        );

        tokio::task::spawn_blocking(move || release_legacy_read.wait())
            .await
            .expect("release legacy read");
        assert_eq!(
            migration
                .await
                .expect("migration task")
                .expect("migration result"),
            "legacy-password"
        );
        deletion
            .await
            .expect("deletion task")
            .expect("deletion result");

        let mut database = open_database(&path).await;
        let envelope_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM credential_envelopes \
             WHERE reference = 'racing-reference'",
        )
        .fetch_one(&mut database)
        .await
        .expect("count racing envelopes");
        assert_eq!(envelope_count, 0);
        assert!(!keyring.has_legacy("racing-reference"));
    }

    #[tokio::test]
    async fn legacy_migration_commits_before_retryable_cleanup() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        keyring.insert_legacy("legacy-reference", "legacy-password");
        keyring.fail_next_delete();
        let vault = vault(&path, keyring.clone());

        let cleanup_error = vault
            .get("legacy-reference", b"legacy-binding")
            .await
            .expect_err("failed legacy cleanup must block credential use");
        assert!(matches!(
            cleanup_error.code,
            CredentialVaultErrorCode::AccessDenied | CredentialVaultErrorCode::StoreUnavailable
        ));
        assert!(keyring.has_legacy("legacy-reference"));

        let mut database = open_database(&path).await;
        let pending: i64 = sqlx::query_scalar(
            "SELECT legacy_cleanup_pending \
             FROM credential_envelopes WHERE reference = 'legacy-reference'",
        )
        .fetch_one(&mut database)
        .await
        .expect("pending cleanup marker");
        assert_eq!(pending, 1);
        drop(database);

        assert_eq!(
            vault
                .get("legacy-reference", b"legacy-binding")
                .await
                .expect("retry cleanup"),
            "legacy-password"
        );
        assert!(!keyring.has_legacy("legacy-reference"));
        assert_eq!(keyring.legacy_reads.load(Ordering::SeqCst), 1);

        let mut database = open_database(&path).await;
        let pending: i64 = sqlx::query_scalar(
            "SELECT legacy_cleanup_pending \
             FROM credential_envelopes WHERE reference = 'legacy-reference'",
        )
        .fetch_one(&mut database)
        .await
        .expect("cleared cleanup marker");
        assert_eq!(pending, 0);
    }

    #[tokio::test]
    async fn deleting_pending_migration_retries_legacy_cleanup() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        keyring.insert_legacy("pending-delete", "legacy-password");
        keyring.fail_next_delete();
        let vault = vault(&path, keyring.clone());

        vault
            .get("pending-delete", b"binding")
            .await
            .expect_err("initial cleanup must fail");
        keyring.fail_next_delete();
        vault
            .delete("pending-delete")
            .await
            .expect_err("pending delete must surface keyring failure");
        assert!(keyring.has_legacy("pending-delete"));

        vault
            .delete("pending-delete")
            .await
            .expect("retry legacy delete");
        assert!(!keyring.has_legacy("pending-delete"));
    }

    #[tokio::test]
    async fn encrypted_delete_skips_keyring_but_legacy_delete_remains_supported() {
        let directory = TempDir::new().expect("tempdir");
        let path = database_path(&directory);
        let keyring = Arc::new(MockKeyring::default());
        let vault = vault(&path, keyring.clone());
        let reference = vault.put(b"binding", "secret").await.expect("put");

        vault.delete(&reference).await.expect("delete encrypted");
        assert_eq!(keyring.legacy_deletes.load(Ordering::SeqCst), 0);

        keyring.insert_legacy("legacy-delete", "old-secret");
        vault.delete("legacy-delete").await.expect("delete legacy");
        assert_eq!(keyring.legacy_deletes.load(Ordering::SeqCst), 1);
        assert!(!keyring.has_legacy("legacy-delete"));
    }

    #[tokio::test]
    async fn memory_vault_authenticates_binding() {
        let vault = MemoryCredentialVault::shared();
        let reference = vault.put(b"expected", "secret").await.expect("put");
        assert_eq!(
            vault.get(&reference, b"expected").await.expect("get"),
            "secret"
        );
        let error = vault
            .get(&reference, b"different")
            .await
            .expect_err("different binding must fail");
        assert_eq!(error.code, CredentialVaultErrorCode::Corrupt);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::{macos_master_access_control, platform_failure_is_access_denied};
    use security_framework::{access_control::SecAccessControl, passwords::AccessControlOptions};

    #[test]
    fn macos_authorization_failures_are_reported_as_access_denied() {
        for status in [-128, -25_243, -25_293, -25_308] {
            let error = security_framework::base::Error::from_code(status);
            assert!(platform_failure_is_access_denied(&error), "{status}");
        }

        let unavailable = security_framework::base::Error::from_code(-25_291);
        assert!(!platform_failure_is_access_denied(&unavailable));
    }

    #[test]
    fn macos_master_accepts_biometry_watch_or_password() {
        let options = macos_master_access_control();
        assert!(options.contains(AccessControlOptions::BIOMETRY_ANY));
        assert!(options.contains(AccessControlOptions::WATCH));
        assert!(options.contains(AccessControlOptions::DEVICE_PASSCODE));
        assert!(options.contains(AccessControlOptions::OR));
        SecAccessControl::create_with_flags(options.bits())
            .expect("macOS must accept the Astesia master-key access control");
    }
}
