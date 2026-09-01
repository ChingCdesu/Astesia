use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::json;
use sqlx::{sqlite::SqliteConnectOptions, Connection, Row, SqliteConnection};
use uuid::Uuid;

use super::{
    default_database_path,
    format::{
        credential_binding, profile_select_for_schema, required_tables_query, row_to_record,
        CURRENT_SCHEMA_TABLES, MIN_SUPPORTED_SCHEMA_VERSION, SCHEMA_VERSION,
    },
    ConnectionRepositoryError, ConnectionRepositoryErrorCode, NativeStateProbe, DATABASE_FILENAME,
};
use crate::{
    credential_vault::{CredentialVaultHandle, SystemCredentialVault},
    db::ConnectionConfig,
};

pub(crate) async fn probe_default_native_state(
) -> Result<NativeStateProbe, ConnectionRepositoryError> {
    let vault = SystemCredentialVault::shared_strict();
    probe_native_state_with_vault(&default_database_path()?, &vault).await
}

async fn probe_native_state_with_vault(
    database_path: &Path,
    vault: &CredentialVaultHandle,
) -> Result<NativeStateProbe, ConnectionRepositoryError> {
    let (probe, credentials) = inspect_native_state(database_path).await?;
    for credential in credentials {
        let binding = credential_binding(&credential.config);
        let _secret = vault.get(&credential.reference, &binding).await?;
    }
    Ok(probe)
}

async fn probe_native_state(
    database_path: &Path,
) -> Result<NativeStateProbe, ConnectionRepositoryError> {
    Ok(inspect_native_state(database_path).await?.0)
}

struct NativeCredentialProbe {
    reference: String,
    config: ConnectionConfig,
}

#[derive(PartialEq, Eq)]
struct NativeStateFiles {
    database: Vec<u8>,
    wal: Option<Vec<u8>>,
    journal: Option<Vec<u8>>,
}

async fn stage_native_state(
    database_path: &Path,
) -> Result<tempfile::TempDir, ConnectionRepositoryError> {
    let mut stable_files = None;
    for _ in 0..4 {
        let first = read_native_state_files(database_path).await?;
        let second = read_native_state_files(database_path).await?;
        if first == second {
            stable_files = Some(first);
            break;
        }
        tokio::task::yield_now().await;
    }
    let files = stable_files.ok_or_else(|| {
        ConnectionRepositoryError::new(
            ConnectionRepositoryErrorCode::StorageBusy,
            "共享连接仓库在 Native State Probe 期间持续变化",
            "请等待其他 Astesia 或 MCP 操作完成后重试。",
        )
    })?;
    let directory = tempfile::Builder::new()
        .prefix("astesia-native-state-")
        .tempdir()
        .map_err(|_| {
            ConnectionRepositoryError::storage_unavailable("无法创建 Native State Probe 临时目录")
        })?;
    let staged_database = directory.path().join(DATABASE_FILENAME);
    tokio::fs::write(&staged_database, files.database)
        .await
        .map_err(|_| {
            ConnectionRepositoryError::storage_unavailable("无法暂存 Native State Probe 数据库快照")
        })?;
    if let Some(wal) = files.wal {
        tokio::fs::write(sqlite_sidecar_path(&staged_database, "-wal"), wal)
            .await
            .map_err(|_| {
                ConnectionRepositoryError::storage_unavailable(
                    "无法暂存 Native State Probe WAL 快照",
                )
            })?;
    }
    if let Some(journal) = files.journal {
        tokio::fs::write(sqlite_sidecar_path(&staged_database, "-journal"), journal)
            .await
            .map_err(|_| {
                ConnectionRepositoryError::storage_unavailable(
                    "无法暂存 Native State Probe journal 快照",
                )
            })?;
    }
    Ok(directory)
}

async fn read_native_state_files(
    database_path: &Path,
) -> Result<NativeStateFiles, ConnectionRepositoryError> {
    let database = tokio::fs::read(database_path).await.map_err(|_| {
        ConnectionRepositoryError::storage_unavailable("无法读取 Astesia 共享连接仓库")
    })?;
    Ok(NativeStateFiles {
        database,
        wal: read_optional_native_state_file(&sqlite_sidecar_path(database_path, "-wal")).await?,
        journal: read_optional_native_state_file(&sqlite_sidecar_path(database_path, "-journal"))
            .await?,
    })
}

async fn read_optional_native_state_file(
    path: &Path,
) -> Result<Option<Vec<u8>>, ConnectionRepositoryError> {
    match tokio::fs::read(path).await {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(ConnectionRepositoryError::storage_unavailable(
            "无法读取 Astesia 共享连接仓库 sidecar",
        )),
    }
}

fn sqlite_sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut path = database_path.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

async fn inspect_native_state(
    database_path: &Path,
) -> Result<(NativeStateProbe, Vec<NativeCredentialProbe>), ConnectionRepositoryError> {
    match tokio::fs::symlink_metadata(database_path).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((NativeStateProbe::Fresh, Vec::new()));
        }
        Err(_) => {
            return Err(ConnectionRepositoryError::storage_unavailable(
                "无法检查 Astesia 共享连接仓库",
            ));
        }
    }

    let staged_state = stage_native_state(database_path).await?;
    let staged_database = staged_state.path().join(DATABASE_FILENAME);

    // The source is copied because immutable mode misses committed WAL state, while opening the
    // source normally can create or modify its shared-memory sidecar.
    let options = SqliteConnectOptions::new()
        .filename(&staged_database)
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(5))
        .pragma("query_only", "ON");
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|error| native_state_sqlx_error(error, "只读打开共享连接仓库"))?;
    let mut transaction = connection
        .begin()
        .await
        .map_err(|error| native_state_sqlx_error(error, "开始检查共享连接仓库"))?;

    let quick_check = sqlx::query_scalar::<_, String>("PRAGMA quick_check")
        .fetch_all(&mut *transaction)
        .await
        .map_err(|error| native_state_sqlx_error(error, "检查共享连接仓库完整性"))?;
    if quick_check.as_slice() != ["ok"] {
        return Err(native_state_corrupt("共享连接仓库未通过 SQLite 完整性检查")
            .with_details(json!({ "quick_check": quick_check })));
    }

    let schema_version = sqlx::query_scalar::<_, i64>("PRAGMA user_version")
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| native_state_sqlx_error(error, "读取共享连接仓库版本"))?;
    if !(MIN_SUPPORTED_SCHEMA_VERSION..=SCHEMA_VERSION).contains(&schema_version) {
        return Err(ConnectionRepositoryError::unsupported_schema(
            schema_version,
        ));
    }

    let credentials = validate_native_state(&mut transaction, schema_version).await?;

    transaction
        .commit()
        .await
        .map_err(|error| native_state_sqlx_error(error, "完成共享连接仓库检查"))?;
    connection
        .close()
        .await
        .map_err(|error| native_state_sqlx_error(error, "关闭共享连接仓库检查"))?;
    Ok((NativeStateProbe::Ready { schema_version }, credentials))
}

async fn validate_native_state(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    schema_version: i64,
) -> Result<Vec<NativeCredentialProbe>, ConnectionRepositoryError> {
    let required_tables_query = required_tables_query();
    let present_tables = sqlx::query_scalar::<_, String>(&required_tables_query)
        .fetch_all(&mut **transaction)
        .await
        .map_err(|error| supported_native_state_sqlx_error(error, "检查共享连接仓库表"))?;
    let missing_tables = CURRENT_SCHEMA_TABLES
        .iter()
        .filter(|required| !present_tables.iter().any(|present| present == **required))
        .copied()
        .collect::<Vec<_>>();
    if !missing_tables.is_empty() {
        return Err(
            native_state_corrupt("共享连接仓库缺少声明版本所需的数据表").with_details(json!({
                "schema_version": schema_version,
                "missing_tables": missing_tables,
            })),
        );
    }

    let revision = sqlx::query_scalar::<_, i64>(
        "SELECT revision FROM shared_connection_state WHERE singleton = 1",
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| supported_native_state_sqlx_error(error, "读取共享连接仓库版本号"))?
    .ok_or_else(|| native_state_corrupt("共享连接仓库缺少全局 revision"))?;
    if revision < 0 {
        return Err(native_state_corrupt("共享连接仓库包含无效的全局 revision"));
    }

    let repository_id = sqlx::query_scalar::<_, String>(
        "SELECT value FROM shared_connection_meta WHERE key = 'repository_id'",
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| supported_native_state_sqlx_error(error, "读取共享连接仓库标识"))?
    .ok_or_else(|| native_state_corrupt("共享连接仓库缺少 repository_id"))?;
    Uuid::parse_str(&repository_id)
        .map_err(|_| native_state_corrupt("共享连接仓库包含无效的 repository_id"))?;

    let cleanup_rows =
        sqlx::query("SELECT credential_ref, cleanup_after FROM pending_credential_cleanup")
            .fetch_all(&mut **transaction)
            .await
            .map_err(|error| {
                supported_native_state_sqlx_error(error, "读取共享连接仓库凭据清理队列")
            })?;
    for row in cleanup_rows {
        row.try_get::<String, _>("credential_ref")
            .map_err(|error| {
                supported_native_state_sqlx_error(error, "解析共享连接仓库凭据清理队列")
            })?;
        row.try_get::<i64, _>("cleanup_after").map_err(|error| {
            supported_native_state_sqlx_error(error, "解析共享连接仓库凭据清理队列")
        })?;
    }

    let profile_query = profile_select_for_schema(schema_version);
    let profile_rows = sqlx::query(&profile_query)
        .fetch_all(&mut **transaction)
        .await
        .map_err(|error| supported_native_state_sqlx_error(error, "读取共享连接仓库连接资料"))?;
    let mut credentials = Vec::new();
    for row in &profile_rows {
        let record = row_to_record(row)?;
        if record.profile.revision < 0 || record.profile.revision > revision {
            return Err(
                native_state_corrupt("共享连接仓库包含超出全局 revision 的连接资料").with_details(
                    json!({
                        "connection_id": record.profile.id,
                        "profile_revision": record.profile.revision,
                        "repository_revision": revision,
                    }),
                ),
            );
        }
        if let Some(reference) = record.credential_ref {
            credentials.push(NativeCredentialProbe {
                reference,
                config: record.profile.public_config(),
            });
        }
    }

    Ok(credentials)
}

fn native_state_corrupt(message: impl Into<String>) -> ConnectionRepositoryError {
    ConnectionRepositoryError::new(
        ConnectionRepositoryErrorCode::StorageCorrupt,
        message,
        "请从备份恢复仓库，或联系 Astesia 维护者；探测失败时不要初始化或替换仓库。",
    )
}

fn native_state_sqlx_error(error: sqlx::Error, operation: &str) -> ConnectionRepositoryError {
    let is_corrupt = match &error {
        sqlx::Error::Decode(_) | sqlx::Error::ColumnDecode { .. } => true,
        sqlx::Error::Database(database_error) => database_error.code().is_some_and(|code| {
            code.parse::<i32>()
                .is_ok_and(|code| matches!(code & 0xff, 11 | 26))
        }),
        _ => false,
    };
    if is_corrupt {
        return native_state_corrupt(format!("无法{operation}：共享连接仓库数据已损坏"));
    }
    ConnectionRepositoryError::from_sqlx(error, operation)
}

fn supported_native_state_sqlx_error(
    error: sqlx::Error,
    operation: &str,
) -> ConnectionRepositoryError {
    if let sqlx::Error::Database(database_error) = &error {
        let transient_or_io = database_error.code().is_some_and(|code| {
            code.parse::<i32>()
                .is_ok_and(|code| matches!(code & 0xff, 5 | 6 | 8 | 10 | 14))
        });
        if transient_or_io {
            return ConnectionRepositoryError::from_sqlx(error, operation);
        }
    }
    native_state_corrupt(format!("无法{operation}：共享连接仓库不符合声明的 schema"))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, sync::Arc, time::SystemTime};

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        connection_repository::SharedConnectionRepository,
        credential_vault::{test_support::MemoryCredentialVault, CredentialVault},
        db::{ConnectionConfig, DbType},
    };

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

    #[derive(Debug, PartialEq, Eq)]
    struct DatabaseFileSnapshot {
        len: u64,
        modified: SystemTime,
        sha256: Vec<u8>,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct NativeStateSourceSnapshot {
        database: DatabaseFileSnapshot,
        wal: Option<DatabaseFileSnapshot>,
        shm: Option<DatabaseFileSnapshot>,
        journal: Option<DatabaseFileSnapshot>,
    }

    fn database_file_snapshot(path: &Path) -> DatabaseFileSnapshot {
        let metadata = fs::metadata(path).expect("database metadata");
        let contents = fs::read(path).expect("database contents");
        DatabaseFileSnapshot {
            len: metadata.len(),
            modified: metadata.modified().expect("database modified time"),
            sha256: ring::digest::digest(&ring::digest::SHA256, &contents)
                .as_ref()
                .to_vec(),
        }
    }

    fn optional_database_file_snapshot(path: &Path) -> Option<DatabaseFileSnapshot> {
        path.exists().then(|| database_file_snapshot(path))
    }

    fn native_state_source_snapshot(path: &Path) -> NativeStateSourceSnapshot {
        NativeStateSourceSnapshot {
            database: database_file_snapshot(path),
            wal: optional_database_file_snapshot(&sqlite_sidecar_path(path, "-wal")),
            shm: optional_database_file_snapshot(&sqlite_sidecar_path(path, "-shm")),
            journal: optional_database_file_snapshot(&sqlite_sidecar_path(path, "-journal")),
        }
    }

    async fn probe_without_modifying(
        path: &Path,
    ) -> Result<NativeStateProbe, ConnectionRepositoryError> {
        let before = native_state_source_snapshot(path);
        let result = probe_native_state(path).await;
        assert_eq!(native_state_source_snapshot(path), before);
        result
    }

    async fn probe_with_vault_without_modifying(
        path: &Path,
        vault: &CredentialVaultHandle,
    ) -> Result<NativeStateProbe, ConnectionRepositoryError> {
        let before = native_state_source_snapshot(path);
        let result = probe_native_state_with_vault(path, vault).await;
        assert_eq!(native_state_source_snapshot(path), before);
        result
    }

    async fn current_repository(temp_dir: &TempDir) -> SharedConnectionRepository {
        let repository = repository(temp_dir, MemoryCredentialVault::shared());
        repository
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create current repository");
        repository
    }

    #[tokio::test]
    async fn native_state_probe_reports_fresh_without_creating_storage() {
        let temp_dir = TempDir::new().expect("temp dir");
        let database_path = temp_dir
            .path()
            .join("missing-parent")
            .join("connections.sqlite3");

        assert_eq!(
            probe_native_state(&database_path).await.expect("probe"),
            NativeStateProbe::Fresh
        );
        assert!(!database_path.exists());
        assert!(!database_path.parent().expect("database parent").exists());
    }

    #[tokio::test]
    async fn native_state_probe_reads_current_wal_state_without_modifying_the_database() {
        let temp_dir = TempDir::new().expect("temp dir");
        let repository = current_repository(&temp_dir).await;
        let database_path = temp_dir.path().join("connections.sqlite3");
        let wal_path = database_path.with_file_name("connections.sqlite3-wal");
        assert!(fs::metadata(wal_path).expect("live WAL").len() > 0);

        assert_eq!(
            probe_without_modifying(&database_path)
                .await
                .expect("probe valid repository"),
            NativeStateProbe::Ready {
                schema_version: SCHEMA_VERSION,
            }
        );
        assert_eq!(repository.list().await.expect("profiles").len(), 1);
    }

    #[tokio::test]
    async fn native_state_probe_does_not_create_a_source_shm_for_live_wal_state() {
        let source_dir = TempDir::new().expect("source temp dir");
        let repository = current_repository(&source_dir).await;
        let source_path = source_dir.path().join("connections.sqlite3");
        let staged_source = stage_native_state(&source_path)
            .await
            .expect("stage source");
        let database_path = staged_source.path().join(DATABASE_FILENAME);
        assert!(
            fs::metadata(sqlite_sidecar_path(&database_path, "-wal"))
                .expect("copied WAL")
                .len()
                > 0
        );
        assert!(!sqlite_sidecar_path(&database_path, "-shm").exists());

        assert_eq!(
            probe_without_modifying(&database_path)
                .await
                .expect("probe copied live WAL"),
            NativeStateProbe::Ready {
                schema_version: SCHEMA_VERSION,
            }
        );
        assert!(!sqlite_sidecar_path(&database_path, "-shm").exists());
        assert_eq!(repository.list().await.expect("profiles").len(), 1);
    }

    #[tokio::test]
    async fn native_state_probe_rejects_a_future_schema_without_modifying_the_database() {
        let temp_dir = TempDir::new().expect("temp dir");
        let repository = current_repository(&temp_dir).await;
        sqlx::query(&format!("PRAGMA user_version = {}", SCHEMA_VERSION + 1))
            .execute(repository.initialized_pool().await.expect("pool"))
            .await
            .expect("set future schema");
        let database_path = temp_dir.path().join("connections.sqlite3");

        let error = probe_without_modifying(&database_path)
            .await
            .expect_err("future schema must fail");
        assert_eq!(error.code, ConnectionRepositoryErrorCode::UnsupportedSchema);
        assert_eq!(error.details["direction"], "newer");
        assert_eq!(error.details["schema_version"], SCHEMA_VERSION + 1);
        assert_eq!(error.details["supported_schema_version"], SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn native_state_probe_accepts_supported_v3_and_rejects_unknown_older_schema() {
        let temp_dir = TempDir::new().expect("temp dir");
        let repository = current_repository(&temp_dir).await;
        let pool = repository.initialized_pool().await.expect("pool");
        let database_path = temp_dir.path().join("connections.sqlite3");

        sqlx::query("PRAGMA user_version = 3")
            .execute(pool)
            .await
            .expect("set supported schema");
        assert_eq!(
            probe_without_modifying(&database_path)
                .await
                .expect("supported schema"),
            NativeStateProbe::Ready { schema_version: 3 }
        );

        sqlx::query("PRAGMA user_version = 1")
            .execute(pool)
            .await
            .expect("set unknown schema");
        let error = probe_without_modifying(&database_path)
            .await
            .expect_err("unknown older schema must fail");
        assert_eq!(error.code, ConnectionRepositoryErrorCode::UnsupportedSchema);
        assert_eq!(error.details["direction"], "older");
        assert_eq!(error.details["minimum_supported_schema_version"], 2);
    }

    #[tokio::test]
    async fn native_state_probe_fails_closed_when_a_bound_credential_is_missing() {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault = MemoryCredentialVault::shared();
        let repository = repository(&temp_dir, vault.clone());
        repository
            .create(config("analytics", "super-secret"), true)
            .await
            .expect("create current repository");
        let reference = sqlx::query_scalar::<_, String>(
            "SELECT credential_ref FROM shared_connections WHERE id = 'analytics'",
        )
        .fetch_one(repository.initialized_pool().await.expect("pool"))
        .await
        .expect("credential reference");
        vault.delete(&reference).await.expect("remove credential");
        let vault: CredentialVaultHandle = vault;
        let database_path = temp_dir.path().join("connections.sqlite3");

        let error = probe_with_vault_without_modifying(&database_path, &vault)
            .await
            .expect_err("missing credential must fail");
        assert_eq!(error.code, ConnectionRepositoryErrorCode::CredentialMissing);
    }

    #[tokio::test]
    async fn native_state_probe_rejects_corrupt_sqlite_without_modifying_the_file() {
        let temp_dir = TempDir::new().expect("temp dir");
        let database_path = temp_dir.path().join("connections.sqlite3");
        fs::write(&database_path, b"not a sqlite database").expect("write corrupt database");

        let error = probe_without_modifying(&database_path)
            .await
            .expect_err("corrupt database must fail");
        assert_eq!(error.code, ConnectionRepositoryErrorCode::StorageCorrupt);
    }

    #[tokio::test]
    async fn native_state_probe_rejects_a_missing_current_table_without_modifying_the_database() {
        let temp_dir = TempDir::new().expect("temp dir");
        let repository = current_repository(&temp_dir).await;
        sqlx::query("DROP TABLE shared_connection_meta")
            .execute(repository.initialized_pool().await.expect("pool"))
            .await
            .expect("drop required table");
        let database_path = temp_dir.path().join("connections.sqlite3");

        let error = probe_without_modifying(&database_path)
            .await
            .expect_err("missing current table must fail");
        assert_eq!(error.code, ConnectionRepositoryErrorCode::StorageCorrupt);
        assert_eq!(
            error.details["missing_tables"],
            json!(["shared_connection_meta"])
        );
    }

    #[tokio::test]
    async fn native_state_probe_rejects_invalid_current_metadata_without_modifying_the_database() {
        let temp_dir = TempDir::new().expect("temp dir");
        let repository = current_repository(&temp_dir).await;
        sqlx::query(
            "UPDATE shared_connection_meta SET value = 'not-a-uuid' \
             WHERE key = 'repository_id'",
        )
        .execute(repository.initialized_pool().await.expect("pool"))
        .await
        .expect("corrupt repository id");
        let database_path = temp_dir.path().join("connections.sqlite3");

        let error = probe_without_modifying(&database_path)
            .await
            .expect_err("invalid metadata must fail");
        assert_eq!(error.code, ConnectionRepositoryErrorCode::StorageCorrupt);
    }

    #[tokio::test]
    async fn native_state_probe_rejects_a_profile_ahead_of_the_global_revision() {
        let temp_dir = TempDir::new().expect("temp dir");
        let repository = current_repository(&temp_dir).await;
        sqlx::query("UPDATE shared_connection_state SET revision = 0 WHERE singleton = 1")
            .execute(repository.initialized_pool().await.expect("pool"))
            .await
            .expect("move global revision behind profile");
        let database_path = temp_dir.path().join("connections.sqlite3");

        let error = probe_without_modifying(&database_path)
            .await
            .expect_err("profile revision ahead of repository must fail");
        assert_eq!(error.code, ConnectionRepositoryErrorCode::StorageCorrupt);
        assert_eq!(error.details["profile_revision"], 1);
        assert_eq!(error.details["repository_revision"], 0);
    }

    #[tokio::test]
    async fn native_state_probe_rejects_an_unparseable_profile_without_modifying_the_database() {
        let temp_dir = TempDir::new().expect("temp dir");
        let repository = current_repository(&temp_dir).await;
        sqlx::query("UPDATE shared_connections SET tags_json = '{'")
            .execute(repository.initialized_pool().await.expect("pool"))
            .await
            .expect("corrupt profile tags");
        let database_path = temp_dir.path().join("connections.sqlite3");

        let error = probe_without_modifying(&database_path)
            .await
            .expect_err("unparseable profile must fail");
        assert_eq!(error.code, ConnectionRepositoryErrorCode::StorageCorrupt);
    }
}
