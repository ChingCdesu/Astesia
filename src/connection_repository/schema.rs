use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    format::{META_TABLE, PENDING_CLEANUP_TABLE, PROFILES_TABLE, SCHEMA_VERSION, STATE_TABLE},
    ConnectionRepositoryError, ConnectionRepositoryErrorCode,
};

pub(super) async fn initialize_schema(pool: &SqlitePool) -> Result<(), ConnectionRepositoryError> {
    let version = sqlx::query_scalar::<_, i64>("PRAGMA user_version")
        .fetch_one(pool)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "读取仓库版本"))?;
    if version > SCHEMA_VERSION {
        return Err(ConnectionRepositoryError::unsupported_schema(version));
    }

    let create_state = format!(
        "CREATE TABLE IF NOT EXISTS {STATE_TABLE} (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            revision INTEGER NOT NULL CHECK (revision >= 0)
        )"
    );
    sqlx::query(&create_state)
        .execute(pool)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "初始化连接仓库"))?;
    let initialize_state =
        format!("INSERT OR IGNORE INTO {STATE_TABLE} (singleton, revision) VALUES (1, 0)");
    sqlx::query(&initialize_state)
        .execute(pool)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "初始化连接版本"))?;
    let create_profiles = format!(
        "CREATE TABLE IF NOT EXISTS {PROFILES_TABLE} (
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
        )"
    );
    sqlx::query(&create_profiles)
        .execute(pool)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "初始化连接表"))?;
    ensure_shared_connections_column(pool, "group_name", "TEXT").await?;
    ensure_shared_connections_column(pool, "tags_json", "TEXT NOT NULL DEFAULT '[]'").await?;
    let create_meta = format!(
        "CREATE TABLE IF NOT EXISTS {META_TABLE} (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        )"
    );
    sqlx::query(&create_meta)
        .execute(pool)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "初始化迁移状态"))?;
    let generated_repository_id = Uuid::new_v4().to_string();
    let initialize_repository_id =
        format!("INSERT OR IGNORE INTO {META_TABLE} (key, value) VALUES ('repository_id', ?)");
    sqlx::query(&initialize_repository_id)
        .bind(&generated_repository_id)
        .execute(pool)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "初始化连接仓库标识"))?;
    let select_repository_id =
        format!("SELECT value FROM {META_TABLE} WHERE key = 'repository_id'");
    let repository_id = sqlx::query_scalar::<_, String>(&select_repository_id)
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
    let create_pending_cleanup = format!(
        "CREATE TABLE IF NOT EXISTS {PENDING_CLEANUP_TABLE} (
            credential_ref TEXT PRIMARY KEY NOT NULL,
            cleanup_after INTEGER NOT NULL DEFAULT 0
        )"
    );
    sqlx::query(&create_pending_cleanup)
        .execute(pool)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "初始化凭据清理队列"))?;
    let cleanup_columns = format!(
        "SELECT COUNT(*) FROM pragma_table_info('{PENDING_CLEANUP_TABLE}') WHERE name = 'cleanup_after'"
    );
    let has_cleanup_after = sqlx::query_scalar::<_, i64>(&cleanup_columns)
        .fetch_one(pool)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "检查凭据清理队列版本"))?
        > 0;
    if !has_cleanup_after {
        let add_cleanup_after = format!(
            "ALTER TABLE {PENDING_CLEANUP_TABLE} ADD COLUMN cleanup_after INTEGER NOT NULL DEFAULT 0"
        );
        let alter_result = sqlx::query(&add_cleanup_after).execute(pool).await;
        if let Err(error) = alter_result {
            let upgraded_by_peer = sqlx::query_scalar::<_, i64>(&cleanup_columns)
                .fetch_one(pool)
                .await
                .map_err(|check_error| {
                    ConnectionRepositoryError::from_sqlx(check_error, "确认凭据清理队列升级结果")
                })?
                > 0;
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
    let profile_columns =
        format!("SELECT COUNT(*) FROM pragma_table_info('{PROFILES_TABLE}') WHERE name = ?");
    let exists = sqlx::query_scalar::<_, i64>(&profile_columns)
        .bind(column_name)
        .fetch_one(pool)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "检查连接表版本"))?
        > 0;
    if exists {
        return Ok(());
    }

    let add_column = format!("ALTER TABLE {PROFILES_TABLE} ADD COLUMN {column_name} {definition}");
    let alter_result = sqlx::query(&add_column).execute(pool).await;
    if let Err(error) = alter_result {
        let upgraded_by_peer = sqlx::query_scalar::<_, i64>(&profile_columns)
            .bind(column_name)
            .fetch_one(pool)
            .await
            .map_err(|check_error| {
                ConnectionRepositoryError::from_sqlx(check_error, "确认连接表升级结果")
            })?
            > 0;
        if !upgraded_by_peer {
            return Err(ConnectionRepositoryError::from_sqlx(error, "升级连接表"));
        }
    }
    Ok(())
}

pub(super) async fn next_revision(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<i64, ConnectionRepositoryError> {
    let update_revision = format!(
        "UPDATE {STATE_TABLE} \
         SET revision = revision + 1 \
         WHERE singleton = 1 AND revision < 9223372036854775807 \
         RETURNING revision"
    );
    sqlx::query_scalar::<_, i64>(&update_revision)
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

pub(super) async fn remove_pending_credential_cleanup(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    reference: &str,
) -> Result<(), ConnectionRepositoryError> {
    let remove_cleanup = format!("DELETE FROM {PENDING_CLEANUP_TABLE} WHERE credential_ref = ?");
    sqlx::query(&remove_cleanup)
        .bind(reference)
        .execute(&mut **transaction)
        .await
        .map(|_| ())
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "确认凭据已被连接引用"))
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use tempfile::TempDir;

    use super::*;
    use crate::{
        connection_repository::SharedConnectionRepository,
        credential_vault::test_support::MemoryCredentialVault,
    };

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
}
