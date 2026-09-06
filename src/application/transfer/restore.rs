use std::io::{BufReader, BufWriter, Seek, Write};

use tokio::io::AsyncReadExt;

use crate::application::QueryTarget;
use crate::db::{DbType, SqlRenderError, SqlScript, UnsupportedFeature};
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
        target: QueryTarget,
        file_path: String,
    ) -> Result<String, String> {
        let database = target.database.clone();
        let driver_handle = self.driver_for_target(&target).await?;
        let db_type = {
            let driver = driver_handle.lock_active().await?;
            driver.db_type()
        };
        if !db_type.capabilities().restore {
            return Err(UnsupportedFeature::new(db_type, "restore").to_string());
        }
        let (spool, total) =
            tokio::task::spawn_blocking(move || validate_restore(db_type, &file_path))
                .await
                .map_err(|error| format!("读取备份任务失败: {error}"))??;
        let task_id = self
            .tasks
            .spawn(
                NewTask {
                    name: format!("恢复 {}", database),
                    initial_message: "开始恢复...".to_string(),
                },
                move |task| async move {
                    let mut effects = TransferEffects::default();

                    let mut statements =
                        tokio::io::BufReader::new(tokio::fs::File::from_std(spool));
                    for i in 0..total {
                        if task.is_cancelled() {
                            return effects
                                .interrupted("恢复", TransferFailure::cancelled("任务已取消"));
                        }

                        let stmt = match read_statement(&mut statements).await {
                            Ok(statement) => statement,
                            Err(error) => {
                                return effects.interrupted("恢复", TransferFailure::failed(error))
                            }
                        };
                        let result = match driver_handle.lock_active().await {
                            Ok(driver) => driver.execute_query(&database, &stmt).await,
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
                                effects.record_failure(1, format!("SQL 语句执行失败: {e}"));
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

// The private spool fixes the validated input before any statement can change the database.
fn validate_restore(db_type: DbType, path: &str) -> Result<(std::fs::File, usize), String> {
    let input = std::fs::File::open(path).map_err(|error| format!("读取文件失败: {error}"))?;
    let spool = tempfile::tempfile().map_err(|error| format!("创建恢复临时文件失败: {error}"))?;
    let mut output = BufWriter::new(spool);
    let count = SqlScript::for_each_statement(db_type, BufReader::new(input), |statement| {
        output
            .write_all(&(statement.len() as u64).to_le_bytes())
            .and_then(|_| output.write_all(statement.as_bytes()))
            .map_err(|error| {
                SqlRenderError::InvalidScript(format!("写入恢复临时文件失败: {error}"))
            })
    })
    .map_err(|error| format!("解析备份文件失败: {error}"))?;
    output
        .flush()
        .map_err(|error| format!("写入恢复临时文件失败: {error}"))?;
    let mut spool = output.into_inner().map_err(|error| error.to_string())?;
    spool.rewind().map_err(|error| error.to_string())?;
    Ok((spool, count))
}

async fn read_statement(
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
) -> Result<String, String> {
    let length = reader
        .read_u64_le()
        .await
        .map_err(|error| error.to_string())?;
    let length = usize::try_from(length).map_err(|error| error.to_string())?;
    let mut bytes = vec![0; length];
    reader
        .read_exact(&mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use crate::{
        db::{DbType, SqlScript},
        tasks::TaskOutcome,
    };

    use super::{restore_outcome, TransferEffects};

    #[tokio::test]
    async fn validated_spool_preserves_statements_after_source_changes() {
        let input = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(input.path(), "-- before\nSELECT '你;好'; SELECT 2;").unwrap();
        let (file, count) =
            super::validate_restore(DbType::SQLite, input.path().to_str().unwrap()).unwrap();
        assert_eq!(count, 2);
        std::fs::write(input.path(), "SELECT 'changed';").unwrap();
        let mut reader = tokio::io::BufReader::new(tokio::fs::File::from_std(file));
        assert_eq!(
            super::read_statement(&mut reader).await.unwrap(),
            "SELECT '你;好'"
        );
        assert_eq!(
            super::read_statement(&mut reader).await.unwrap(),
            "SELECT 2"
        );
    }

    #[test]
    fn invalid_tail_never_exposes_an_executable_spool() {
        let input = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            input.path(),
            "CREATE TABLE should_not_exist (id INTEGER); SELECT 'unterminated",
        )
        .unwrap();
        assert!(super::validate_restore(DbType::SQLite, input.path().to_str().unwrap()).is_err());
    }

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
