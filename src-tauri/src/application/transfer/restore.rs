use crate::db::{SqlScript, UnsupportedFeature};
use crate::tasks::{NewTask, TaskOutcome};

use super::{TransferEffects, TransferFailure, TransferService};

fn restore_outcome(effects: TransferEffects) -> TaskOutcome {
    effects.finish("恢复", |success_count| {
        format!("恢复完成: 成功 {success_count} / 失败 0")
    })
}

impl TransferService {
    pub async fn start_restore(
        &self,
        connection_id: String,
        database: String,
        file_path: String,
    ) -> Result<String, String> {
        let sql_content =
            std::fs::read_to_string(&file_path).map_err(|e| format!("读取文件失败: {}", e))?;

        let driver_handle = self.connections.driver(&connection_id).await?;
        let db_type = {
            let driver = driver_handle.lock_active().await?;
            driver.db_type()
        };
        if !db_type.capabilities().restore {
            return Err(UnsupportedFeature::new(db_type, "restore").to_string());
        }
        let statements = SqlScript::parse(db_type, &sql_content)
            .map_err(|error| format!("解析备份文件失败: {error}"))?
            .into_statements();
        let total = statements.len();
        let task_id = self
            .tasks
            .spawn(
                NewTask {
                    name: format!("恢复 {}", database),
                    initial_message: "开始恢复...".to_string(),
                },
                move |task| async move {
                    let mut effects = TransferEffects::default();

                    for (i, stmt) in statements.iter().enumerate() {
                        if task.is_cancelled() {
                            return effects
                                .interrupted("恢复", TransferFailure::cancelled("任务已取消"));
                        }

                        let result = match driver_handle.lock_active().await {
                            Ok(driver) => driver.execute_query(&database, stmt).await,
                            Err(_) => {
                                return effects
                                    .interrupted("恢复", TransferFailure::failed("连接已断开"));
                            }
                        };

                        match result {
                            Ok(_) => {
                                effects.record_applied(1);
                            }
                            Err(e) => {
                                effects.record_failure(1, "SQL 语句执行失败");
                                log::warn!("Restore statement failed: {}", e);
                            }
                        }

                        let progress = (i + 1) as f32 / total as f32;
                        task.progress(
                            progress,
                            format!(
                                "已执行 {}/{} 语句 (失败: {})",
                                i + 1,
                                total,
                                effects.failure_count()
                            ),
                        )
                        .await;
                    }

                    restore_outcome(effects)
                },
            )
            .await;

        Ok(task_id)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        db::{DbType, SqlScript},
        tasks::TaskOutcome,
    };

    use super::{restore_outcome, TransferEffects};

    #[test]
    fn classifies_statement_failures_by_applied_effects() {
        let mut partial = TransferEffects::default();
        partial.record_applied(4);
        partial.record_failure(1, "SQL 语句执行失败");
        assert!(matches!(
            restore_outcome(partial),
            TaskOutcome::Partial(message)
                if message == "恢复部分完成：已应用 4 项更改；1 项失败；SQL 语句执行失败"
        ));

        let mut failed = TransferEffects::default();
        failed.record_failure(5, "SQL 语句执行失败");
        assert!(matches!(
            restore_outcome(failed),
            TaskOutcome::Failed(message)
                if message == "恢复失败：已应用 0 项更改；5 项失败；SQL 语句执行失败"
        ));

        let mut completed = TransferEffects::default();
        completed.record_applied(5);
        assert!(matches!(
            restore_outcome(completed),
            TaskOutcome::Completed(message) if message == "恢复完成: 成功 5 / 失败 0"
        ));
    }

    #[test]
    fn parses_backup_comments_literals_and_postgres_blocks_as_complete_statements() {
        let backup = "-- Astesia Database Backup\n\
                      -- Database: app\n\n\
                      CREATE TABLE \"events\" (\"message\" text);\n\n\
                      INSERT INTO \"events\" VALUES (E'one;two');\n\n\
                      DO $$ BEGIN PERFORM 1; PERFORM 2; END $$;";

        let statements = SqlScript::parse(DbType::PostgreSQL, backup)
            .unwrap()
            .into_statements();

        assert_eq!(statements.len(), 3);
        assert!(statements[0].starts_with("CREATE TABLE"));
        assert!(statements[1].contains("one;two"));
        assert!(statements[2].contains("PERFORM 1; PERFORM 2;"));
    }
}
