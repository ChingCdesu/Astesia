use std::{path::PathBuf, time::Duration};

use ring::{
    aead::{self, Aad, LessSafeKey, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use sqlx::{sqlite::SqliteConnectOptions, Connection, Row, SqliteConnection};
use zeroize::Zeroizing;

use super::{CredentialVaultError, CredentialVaultErrorCode};

pub(super) const MASTER_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const AUTH_TAG_LEN: usize = 16;
const DATABASE_DIRECTORY: &str = "com.astesia.app";
const DATABASE_FILENAME: &str = "credential-vault.sqlite3";
const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(120);
pub(super) const ENCRYPTION_INSERT_ATTEMPTS: usize = 8;
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

#[derive(Debug)]
pub(super) struct EncryptedEnvelope {
    pub(super) nonce: Vec<u8>,
    pub(super) ciphertext: Vec<u8>,
    pub(super) legacy_cleanup_pending: bool,
}

pub(super) fn default_database_path() -> Option<PathBuf> {
    dirs::data_dir().map(|data| data.join(DATABASE_DIRECTORY).join(DATABASE_FILENAME))
}

pub(super) async fn connect_database(
    path: &std::path::Path,
    operation: &str,
) -> Result<SqliteConnection, CredentialVaultError> {
    let parent = path
        .parent()
        .ok_or_else(|| CredentialVaultError::new(CredentialVaultErrorCode::Invalid, operation))?;
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

pub(super) async fn begin_immediate(
    connection: &mut SqliteConnection,
    operation: &str,
) -> Result<(), CredentialVaultError> {
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await
        .map_err(|error| map_database_error(error, operation))?;
    Ok(())
}

pub(super) async fn count_envelopes(
    connection: &mut SqliteConnection,
    operation: &str,
) -> Result<i64, CredentialVaultError> {
    sqlx::query_scalar("SELECT COUNT(*) FROM credential_envelopes")
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| map_database_error(error, operation))
}

pub(super) async fn finish_transaction<T>(
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

pub(super) async fn insert_envelope_once(
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

pub(super) async fn insert_envelope_with_retry(
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

pub(super) async fn load_envelope(
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

pub(super) async fn mark_legacy_cleanup_complete(
    connection: &mut SqliteConnection,
    reference: &str,
    operation: &str,
) -> Result<(), CredentialVaultError> {
    sqlx::query(
        "UPDATE credential_envelopes \
         SET legacy_cleanup_pending = 0 \
         WHERE reference = ? AND legacy_cleanup_pending = 1",
    )
    .bind(reference)
    .execute(&mut *connection)
    .await
    .map_err(|error| map_database_error(error, operation))?;
    Ok(())
}

pub(super) async fn delete_envelope(
    connection: &mut SqliteConnection,
    reference: &str,
    operation: &str,
) -> Result<(), CredentialVaultError> {
    sqlx::query("DELETE FROM credential_envelopes WHERE reference = ?")
        .bind(reference)
        .execute(&mut *connection)
        .await
        .map_err(|error| map_database_error(error, operation))?;
    Ok(())
}

pub(super) fn generate_master_key(
    operation: &str,
) -> Result<Zeroizing<[u8; MASTER_KEY_LEN]>, CredentialVaultError> {
    let mut key = Zeroizing::new([0_u8; MASTER_KEY_LEN]);
    SystemRandom::new()
        .fill(&mut *key)
        .map_err(|_| CredentialVaultError::task_failed(operation))?;
    Ok(key)
}

pub(super) fn decode_master_key(
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

pub(super) fn decrypt_secret(
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

fn encryption_key(
    master: &[u8; MASTER_KEY_LEN],
    operation: &str,
) -> Result<LessSafeKey, CredentialVaultError> {
    UnboundKey::new(&aead::AES_256_GCM, master)
        .map(LessSafeKey::new)
        .map_err(|_| CredentialVaultError::new(CredentialVaultErrorCode::Invalid, operation))
}

pub(super) fn map_database_error(error: sqlx::Error, operation: &str) -> CredentialVaultError {
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
