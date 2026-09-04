use std::collections::HashSet;

use super::{
    format::{
        credential_binding, db_type_to_str, is_unique_violation, validate_config,
        validate_unique_configs,
    },
    schema::{next_revision, remove_pending_credential_cleanup},
    ConnectionRepositoryError, ConnectionRepositoryErrorCode, LegacyMigrationResult,
    SharedConnectionRecord, SharedConnectionRepository,
};
use crate::{credential_vault::CredentialVaultErrorCode, db::ConnectionConfig};

impl SharedConnectionRepository {
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
}
