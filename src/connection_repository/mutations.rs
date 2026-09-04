use serde_json::json;
use sqlx::SqlitePool;

use super::{
    format::{
        credential_binding, db_type_to_str, is_unique_violation, normalize_group_name,
        normalize_tags, profile_from_config, serialize_tags, validate_config,
    },
    schema::{next_revision, remove_pending_credential_cleanup},
    ConnectionRepositoryError, ConnectionRepositoryErrorCode, DeleteConnectionResult,
    SaveConnectionRequest, SharedConnectionProfile, SharedConnectionRepository,
};
use crate::db::{ConnectionConfig, DbType};

// A staged credential may still be committed after an arbitrarily long OS prompt or suspension.
// Only an explicit failed operation may mark it ready for cleanup.
pub(super) const CREDENTIAL_STAGING_SENTINEL: i64 = i64::MAX;

struct StoredProfileUpdate<'a> {
    config: &'a ConnectionConfig,
    expected_revision: i64,
    credential_ref: Option<&'a str>,
    old_credential_ref: Option<&'a str>,
    mcp_enabled: bool,
    group_name: Option<&'a str>,
    tags: &'a [String],
}

impl SharedConnectionRepository {
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
        let clears_credential = config.db_type == DbType::SQLite;
        if replacement_secret.is_empty()
            && !clears_credential
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

        let new_credential_ref = if clears_credential {
            None
        } else if replacement_secret.is_empty() {
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
        let removed_credential = clears_credential && current.credential_ref.is_some();

        let result = self
            .update_with_reference(StoredProfileUpdate {
                config: &config,
                expected_revision,
                credential_ref: new_credential_ref.as_deref(),
                old_credential_ref: current.credential_ref.as_deref(),
                mcp_enabled,
                group_name: group_name.as_deref(),
                tags: &tags,
            })
            .await;
        if result.is_err() && wrote_new_credential {
            if let Some(reference) = new_credential_ref.as_deref() {
                self.schedule_credential_cleanup(reference).await;
            }
        }
        if result.is_ok() && (wrote_new_credential || removed_credential) {
            if let Some(reference) = current.credential_ref.as_deref() {
                self.cleanup_credential(reference).await;
            }
        }
        result
    }

    async fn update_with_reference(
        &self,
        update: StoredProfileUpdate<'_>,
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
        .bind(&update.config.name)
        .bind(db_type_to_str(&update.config.db_type))
        .bind(&update.config.host)
        .bind(i64::from(update.config.port))
        .bind(&update.config.username)
        .bind(&update.config.database)
        .bind(&update.config.color)
        .bind(update.credential_ref)
        .bind(revision)
        .bind(update.mcp_enabled)
        .bind(update.group_name)
        .bind(serialize_tags(update.tags)?)
        .bind(&update.config.id)
        .bind(update.expected_revision)
        .execute(&mut *transaction)
        .await
        .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "更新连接"))?;
        if changed.rows_affected() != 1 {
            transaction.rollback().await.ok();
            let actual = self
                .get(&update.config.id)
                .await
                .ok()
                .map(|profile| profile.revision);
            return Err(ConnectionRepositoryError::conflict(
                &update.config.id,
                update.expected_revision,
                actual,
            ));
        }
        if update.old_credential_ref.is_some() && update.old_credential_ref != update.credential_ref
        {
            sqlx::query(
                "INSERT OR IGNORE INTO pending_credential_cleanup (credential_ref) VALUES (?)",
            )
            .bind(update.old_credential_ref)
            .execute(&mut *transaction)
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "记录旧凭据清理任务"))?;
        }
        if let Some(reference) = update.credential_ref {
            remove_pending_credential_cleanup(&mut transaction, reference).await?;
        }
        transaction
            .commit()
            .await
            .map_err(|error| ConnectionRepositoryError::from_sqlx(error, "更新连接"))?;
        Ok(profile_from_config(
            update.config,
            update.credential_ref.is_some(),
            revision,
            update.mcp_enabled,
            update.group_name.map(str::to_string),
            update.tags.to_vec(),
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

    pub(super) async fn stage_credential_cleanup(
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

    pub(super) async fn schedule_credential_cleanup(&self, reference: &str) -> bool {
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

    pub(super) async fn retry_pending_credential_cleanup(&self, pool: &SqlitePool) {
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
