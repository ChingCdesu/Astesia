use std::{fmt, path::Path, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use sqlx::SqliteConnection;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    envelope::{self, EncryptedEnvelope, ENCRYPTION_INSERT_ATTEMPTS, MASTER_KEY_LEN},
    platform::{self, KeyringBackend, PlatformKeyringBackend},
    CredentialVault, CredentialVaultError, CredentialVaultErrorCode, CredentialVaultHandle,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyCredentialMode {
    MigrateInApp,
    RejectInSidecar,
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
            database_path: envelope::default_database_path(),
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
        envelope::connect_database(self.database_path(operation)?, operation).await
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

        // A cold write must avoid a second macOS authentication prompt, while later writes
        // revalidate the cached key immediately before producing new ciphertext.
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
            Ok(secret) => envelope::decode_master_key(secret, operation)?,
            Err(keyring::Error::NoEntry) => {
                return Err(CredentialVaultError::master_missing(operation));
            }
            Err(error) => return Err(platform::map_error(error, operation)),
        };
        if &cached[..] != &stored[..] {
            return Err(CredentialVaultError::master_changed(operation));
        }
        Ok(stored)
    }

    async fn bootstrap_master(
        &self,
        operation: &'static str,
    ) -> Result<Zeroizing<[u8; MASTER_KEY_LEN]>, CredentialVaultError> {
        let mut connection = self.connect_database(operation).await?;
        envelope::begin_immediate(&mut connection, operation).await?;

        let result = self
            .bootstrap_master_in_transaction(&mut connection, operation)
            .await;
        envelope::finish_transaction(&mut connection, result, operation).await
    }

    async fn bootstrap_master_in_transaction(
        &self,
        connection: &mut SqliteConnection,
        operation: &'static str,
    ) -> Result<Zeroizing<[u8; MASTER_KEY_LEN]>, CredentialVaultError> {
        let encrypted_count = envelope::count_envelopes(connection, operation).await?;

        let keyring = self.keyring.clone();
        let stored = tokio::task::spawn_blocking(move || keyring.get_master())
            .await
            .map_err(|_| CredentialVaultError::task_failed(operation))?;
        match stored {
            Ok(secret) => envelope::decode_master_key(secret, operation),
            Err(keyring::Error::NoEntry) if encrypted_count == 0 => {
                let key = envelope::generate_master_key(operation)?;
                let keyring = self.keyring.clone();
                let to_store = Zeroizing::new(key.to_vec());
                tokio::task::spawn_blocking(move || keyring.set_master(&to_store))
                    .await
                    .map_err(|_| CredentialVaultError::task_failed(operation))?
                    .map_err(|error| platform::map_error(error, operation))?;
                Ok(key)
            }
            Err(keyring::Error::NoEntry) => Err(CredentialVaultError::master_missing(operation)),
            Err(error) => Err(platform::map_error(error, operation)),
        }
    }

    async fn load_envelope(
        &self,
        reference: &str,
        operation: &str,
    ) -> Result<Option<EncryptedEnvelope>, CredentialVaultError> {
        let mut connection = self.connect_database(operation).await?;
        envelope::load_envelope(&mut connection, reference, operation).await
    }

    async fn decrypt_envelope(
        &self,
        encrypted: EncryptedEnvelope,
        binding: &[u8],
        operation: &'static str,
    ) -> Result<String, CredentialVaultError> {
        let key = self.master_key(operation).await?;
        envelope::decrypt_secret(
            key,
            &encrypted.nonce,
            &encrypted.ciphertext,
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
        envelope::begin_immediate(&mut connection, OPERATION).await?;
        let result = async {
            let encrypted = envelope::load_envelope(&mut connection, reference, OPERATION).await?;
            if !encrypted.is_some_and(|encrypted| encrypted.legacy_cleanup_pending) {
                return Ok(());
            }

            let keyring = self.keyring.clone();
            let reference_for_keyring = reference.to_string();
            tokio::task::spawn_blocking(move || {
                match keyring.delete_legacy(&reference_for_keyring) {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(error) => Err(platform::map_error(error, "清理旧版数据库凭据")),
                }
            })
            .await
            .map_err(|_| CredentialVaultError::task_failed("清理旧版数据库凭据"))??;

            envelope::mark_legacy_cleanup_complete(&mut connection, reference, OPERATION).await
        }
        .await;
        envelope::finish_transaction(&mut connection, result, OPERATION).await
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
        envelope::begin_immediate(&mut connection, operation).await?;

        let result = async {
            if let Some(existing) =
                envelope::load_envelope(&mut connection, reference, operation).await?
            {
                return Ok((Some(existing), None));
            }

            let keyring = self.keyring.clone();
            let legacy_reference = reference.to_string();
            let legacy_secret =
                tokio::task::spawn_blocking(move || keyring.get_legacy(&legacy_reference))
                    .await
                    .map_err(|_| CredentialVaultError::task_failed(operation))?
                    .map_err(|error| platform::map_error(error, operation))?;
            let legacy_secret = Zeroizing::new(legacy_secret);
            // The legacy prompt can outlive the earlier master read, so the master must be
            // revalidated after the prompt and immediately before the envelope write.
            let key = self.verify_cached_master_for_write(operation).await?;

            envelope::insert_envelope_with_retry(
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
            envelope::finish_transaction(&mut connection, result, operation).await?;

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
            let inserted = envelope::insert_envelope_once(
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
        if let Some(encrypted) = self.load_envelope(reference, "读取数据库凭据").await? {
            if encrypted.legacy_cleanup_pending
                && self.legacy_mode == LegacyCredentialMode::RejectInSidecar
            {
                return Err(CredentialVaultError::new(
                    CredentialVaultErrorCode::MigrationRequired,
                    "读取数据库凭据",
                ));
            }
            let pending = encrypted.legacy_cleanup_pending;
            let secret = self
                .decrypt_envelope(encrypted, binding, "读取数据库凭据")
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
        envelope::begin_immediate(&mut connection, OPERATION).await?;
        let result = async {
            let encrypted = envelope::load_envelope(&mut connection, reference, OPERATION).await?;
            if self.legacy_mode == LegacyCredentialMode::RejectInSidecar
                && encrypted
                    .as_ref()
                    .is_none_or(|encrypted| encrypted.legacy_cleanup_pending)
            {
                return Err(CredentialVaultError::new(
                    CredentialVaultErrorCode::MigrationRequired,
                    OPERATION,
                ));
            }

            if encrypted
                .as_ref()
                .is_none_or(|encrypted| encrypted.legacy_cleanup_pending)
            {
                let keyring = self.keyring.clone();
                let legacy_reference = reference.to_string();
                tokio::task::spawn_blocking(move || {
                    match keyring.delete_legacy(&legacy_reference) {
                        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                        Err(error) => Err(platform::map_error(error, OPERATION)),
                    }
                })
                .await
                .map_err(|_| CredentialVaultError::task_failed(OPERATION))??;
            }

            envelope::delete_envelope(&mut connection, reference, OPERATION).await
        }
        .await;
        envelope::finish_transaction(&mut connection, result, OPERATION).await
    }
}

#[cfg(test)]
mod tests;
