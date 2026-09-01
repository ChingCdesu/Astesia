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

use super::{LegacyCredentialMode, SystemCredentialVault};
use crate::credential_vault::{
    platform::KeyringBackend, CredentialVault, CredentialVaultErrorCode,
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
    assert_eq!(keyring.master_reads.load(Ordering::SeqCst), 2);
    assert_eq!(keyring.master_writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn database_never_contains_plaintext_passwords() {
    let directory = TempDir::new().expect("tempdir");
    let path = database_path(&directory);
    let vault = vault(&path, Arc::new(MockKeyring::default()));
    let secret = "plaintext-must-not-appear-4f96f687";

    vault.put(b"connection-binding", secret).await.expect("put");
    let database = fs::read(path).expect("read sqlite database");
    assert!(!database
        .windows(secret.len())
        .any(|window| window == secret.as_bytes()));
}

#[tokio::test]
async fn wrong_binding_and_tampered_ciphertext_fail_closed_without_legacy_fallback() {
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

    keyring.insert_legacy(&reference, "legacy-fallback-must-not-be-used");
    let restarted = vault(&path, keyring.clone());
    let tampered = restarted
        .get(&reference, b"correct-binding")
        .await
        .expect_err("tampered ciphertext must fail");
    assert_eq!(tampered.code, CredentialVaultErrorCode::Corrupt);
    assert_eq!(keyring.legacy_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn missing_master_with_ciphertext_is_never_recreated() {
    let directory = TempDir::new().expect("tempdir");
    let path = database_path(&directory);
    let keyring = Arc::new(MockKeyring::default());
    let first = vault(&path, keyring.clone());
    let reference = first.put(b"binding", "protected").await.expect("put");

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
        .expect_err("cached master must be revalidated");
    assert_eq!(error.code, CredentialVaultErrorCode::Corrupt);

    let mut database = open_database(&path).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM credential_envelopes")
        .fetch_one(&mut database)
        .await
        .expect("count envelopes");
    assert_eq!(count, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn master_deleted_during_legacy_prompt_blocks_envelope_write() {
    let directory = TempDir::new().expect("tempdir");
    let path = database_path(&directory);
    let keyring = Arc::new(MockKeyring::default());
    keyring.insert_legacy("legacy-reference", "legacy-password");
    let vault = vault(&path, keyring.clone());
    let (started, release) = keyring.pause_next_legacy_read();

    let migration = tokio::spawn(async move { vault.get("legacy-reference", b"binding").await });
    tokio::task::spawn_blocking(move || started.wait())
        .await
        .expect("wait for legacy prompt");
    keyring.clear_master();
    tokio::task::spawn_blocking(move || release.wait())
        .await
        .expect("release legacy prompt");

    let error = migration
        .await
        .expect("migration task")
        .expect_err("deleted master must block write");
    assert_eq!(error.code, CredentialVaultErrorCode::Corrupt);
    assert!(keyring.has_legacy("legacy-reference"));

    let mut database = open_database(&path).await;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM credential_envelopes WHERE reference = 'legacy-reference'",
    )
    .fetch_one(&mut database)
    .await
    .expect("count envelopes");
    assert_eq!(count, 0);
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
        .expect_err("strict get must require migration");
    assert_eq!(get_error.code, CredentialVaultErrorCode::MigrationRequired);
    let delete_error = vault
        .delete("legacy-reference")
        .await
        .expect_err("strict delete must require migration");
    assert_eq!(
        delete_error.code,
        CredentialVaultErrorCode::MigrationRequired
    );
    assert_eq!(keyring.legacy_reads.load(Ordering::SeqCst), 0);
    assert_eq!(keyring.legacy_deletes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn strict_mode_leaves_pending_migration_untouched() {
    let directory = TempDir::new().expect("tempdir");
    let path = database_path(&directory);
    let keyring = Arc::new(MockKeyring::default());
    keyring.insert_legacy("pending-reference", "legacy-password");
    keyring.fail_next_delete();
    vault(&path, keyring.clone())
        .get("pending-reference", b"binding")
        .await
        .expect_err("prepare pending cleanup marker");

    let reads_before = keyring.legacy_reads.load(Ordering::SeqCst);
    let deletes_before = keyring.legacy_deletes.load(Ordering::SeqCst);
    let strict = strict_vault(&path, keyring.clone());
    assert_eq!(
        strict
            .get("pending-reference", b"binding")
            .await
            .expect_err("strict get must not clean pending data")
            .code,
        CredentialVaultErrorCode::MigrationRequired
    );
    assert_eq!(
        strict
            .delete("pending-reference")
            .await
            .expect_err("strict delete must not clean pending data")
            .code,
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
        "SELECT legacy_cleanup_pending FROM credential_envelopes \
         WHERE reference = 'pending-reference'",
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
    let (started, release) = keyring.pause_next_legacy_read();

    let migration =
        tokio::spawn(async move { migrating_vault.get("racing-reference", b"binding").await });
    tokio::task::spawn_blocking(move || started.wait())
        .await
        .expect("wait for legacy read");

    let mut deletion = tokio::spawn(async move { deleting_vault.delete("racing-reference").await });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut deletion)
            .await
            .is_err(),
        "delete must wait for the migration transaction"
    );

    tokio::task::spawn_blocking(move || release.wait())
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
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM credential_envelopes WHERE reference = 'racing-reference'",
    )
    .fetch_one(&mut database)
    .await
    .expect("count racing envelopes");
    assert_eq!(count, 0);
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

    vault
        .get("legacy-reference", b"legacy-binding")
        .await
        .expect_err("failed cleanup must block credential use");
    assert!(keyring.has_legacy("legacy-reference"));

    let mut database = open_database(&path).await;
    let pending: i64 = sqlx::query_scalar(
        "SELECT legacy_cleanup_pending FROM credential_envelopes \
         WHERE reference = 'legacy-reference'",
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
}

#[tokio::test]
async fn encrypted_delete_skips_keyring_but_pending_delete_retries_cleanup() {
    let directory = TempDir::new().expect("tempdir");
    let path = database_path(&directory);
    let keyring = Arc::new(MockKeyring::default());
    let vault = vault(&path, keyring.clone());
    let reference = vault.put(b"binding", "secret").await.expect("put");
    vault.delete(&reference).await.expect("delete encrypted");
    assert_eq!(keyring.legacy_deletes.load(Ordering::SeqCst), 0);

    keyring.insert_legacy("pending-delete", "legacy-password");
    keyring.fail_next_delete();
    vault
        .get("pending-delete", b"binding")
        .await
        .expect_err("initial cleanup must fail");
    vault
        .delete("pending-delete")
        .await
        .expect("delete retries cleanup");
    assert!(!keyring.has_legacy("pending-delete"));
}

#[tokio::test]
async fn deleting_pending_migration_keeps_envelope_until_legacy_cleanup_succeeds() {
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

    let mut database = open_database(&path).await;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM credential_envelopes WHERE reference = 'pending-delete'",
    )
    .fetch_one(&mut database)
    .await
    .expect("count pending envelope");
    assert_eq!(count, 1);
    drop(database);

    vault
        .delete("pending-delete")
        .await
        .expect("retry legacy delete");
    assert!(!keyring.has_legacy("pending-delete"));
}
