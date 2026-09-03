use crate::application::QueryTarget;
use crate::connection_runtime::DriverHandle;
use crate::db::{DbType, SqlDialect, TableCopyMode, TableRef, UnsupportedFeature};
use crate::tasks::{NewTask, TaskContext, TaskOutcome};

use super::{CopyContent, TransferEffects, TransferFailure, TransferService};

#[derive(Debug)]
pub struct CopyOptions {
    pub content: CopyContent,
    pub new_table_name: String,
}

fn copy_outcome(effects: TransferEffects) -> TaskOutcome {
    effects.finish("复制", |_| "复制完成".to_string())
}

impl TransferService {
    pub async fn start_table_copy(
        &self,
        source_target: QueryTarget,
        source_table: TableRef,
        target_target: QueryTarget,
        options: CopyOptions,
    ) -> Result<String, String> {
        let source_driver_handle = self.driver_for_target(&source_target).await?;
        let target_driver_handle = self.driver_for_target(&target_target).await?;

        // Source and target may share a driver, so never hold both guards at once.
        let db_type = {
            let source_db_type = {
                let driver = source_driver_handle.lock_active().await?;
                driver.db_type()
            };
            let target_db_type = {
                let driver = target_driver_handle.lock_active().await?;
                driver.db_type()
            };
            if source_db_type != target_db_type {
                return Err("仅支持同类型数据库间复制".to_string());
            }
            source_db_type
        };
        if db_type.capabilities().table_copy != TableCopyMode::SameEngine {
            return Err(UnsupportedFeature::new(db_type, "table copy").to_string());
        }

        let task_name = format!("复制表 {} → {}", source_table, options.new_table_name);
        let job = TableCopyJob {
            source_driver: source_driver_handle,
            source_database: source_target.database,
            source_table,
            target_driver: target_driver_handle,
            target_database: target_target.database,
            db_type,
            options,
        };
        let task_id = self
            .tasks
            .spawn(
                NewTask {
                    name: task_name,
                    initial_message: "开始复制...".to_string(),
                },
                move |task| execute_table_copy(task, job),
            )
            .await;

        Ok(task_id)
    }
}

struct TableCopyJob {
    source_driver: DriverHandle,
    source_database: String,
    source_table: TableRef,
    target_driver: DriverHandle,
    target_database: String,
    db_type: DbType,
    options: CopyOptions,
}

async fn execute_table_copy(task: TaskContext, job: TableCopyJob) -> TaskOutcome {
    let total_steps = job.options.content.step_count();
    let mut step = 0;
    let mut effects = TransferEffects::default();

    if job.options.content.includes_structure() {
        step += 1;
        let progress = copy_progress(step, total_steps);
        task.progress(progress, "正在复制表结构...").await;
        if let Err(failure) = copy_structure(&task, &job, &mut effects).await {
            return effects.interrupted("复制", failure);
        }
    }

    if job.options.content.includes_data() {
        step += 1;
        let progress = copy_progress(step, total_steps);
        task.progress(progress, "正在复制数据...").await;
        match copy_data(&task, &job, progress, &mut effects).await {
            Ok(()) => {}
            Err(failure) => return effects.interrupted("复制", failure),
        }
    }

    copy_outcome(effects)
}

async fn copy_structure(
    task: &TaskContext,
    job: &TableCopyJob,
    effects: &mut TransferEffects,
) -> Result<(), TransferFailure> {
    ensure_not_cancelled(task)?;
    let create_sql = {
        let driver = job
            .source_driver
            .lock_active()
            .await
            .map_err(|_| TransferFailure::failed("源连接已断开"))?;
        driver
            .get_create_table_sql(&job.source_database, &job.source_table)
            .await
            .map_err(|error| TransferFailure::failed(format!("获取表结构失败: {error}")))?
    };
    let create_sql = SqlDialect::new(job.db_type)
        .retarget_create_table(&create_sql, &job.options.new_table_name)
        .map_err(|error| TransferFailure::failed(format!("重写表结构失败: {error}")))?;

    ensure_not_cancelled(task)?;
    let driver = job
        .target_driver
        .lock_active()
        .await
        .map_err(|_| TransferFailure::failed("目标连接已断开"))?;
    driver
        .execute_query(&job.target_database, &create_sql)
        .await
        .map_err(|error| TransferFailure::failed(format!("创建表失败: {error}")))?;
    effects.record_applied(1);
    Ok(())
}

async fn copy_data(
    task: &TaskContext,
    job: &TableCopyJob,
    progress: f32,
    effects: &mut TransferEffects,
) -> Result<(), TransferFailure> {
    const PAGE_SIZE: u32 = 1_000;

    let dialect = SqlDialect::new(job.db_type);
    dialect
        .quote_identifier(&job.options.new_table_name)
        .map_err(|error| TransferFailure::failed(format!("目标表名称无效: {error}")))?;

    let mut page = 1;
    let mut total_rows = 0;
    loop {
        ensure_not_cancelled(task)?;
        let result = {
            let driver = job
                .source_driver
                .lock_active()
                .await
                .map_err(|_| TransferFailure::failed("源连接已断开"))?;
            driver
                .get_table_data(&job.source_database, &job.source_table, page, PAGE_SIZE)
                .await
        };
        let result = match result {
            Ok(result) if result.rows.is_empty() => break,
            Ok(result) => result,
            Err(error) => {
                effects.record_failure(1, format!("读取第 {page} 页源数据失败：{error}"));
                break;
            }
        };

        let columns = result
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        for column in &columns {
            dialect
                .quote_identifier(column)
                .map_err(|error| TransferFailure::failed(format!("源表列名称无效: {error}")))?;
        }

        let is_last_page = result.rows.len() < PAGE_SIZE as usize;
        let mut failed_rows = 0;
        for row in &result.rows {
            if let Err(failure) = ensure_not_cancelled(task) {
                record_failed_rows(effects, failed_rows);
                return Err(failure);
            }
            let insert_sql = match dialect.build_insert_row_unqualified(
                &job.options.new_table_name,
                &columns,
                row,
            ) {
                Ok(sql) => sql,
                Err(error) => {
                    record_failed_rows(effects, failed_rows);
                    return Err(TransferFailure::failed(format!(
                        "源表数据无法转换为 SQL: {error}"
                    )));
                }
            };
            let insert_result = {
                let driver = match job.target_driver.lock_active().await {
                    Ok(driver) => driver,
                    Err(_) => {
                        record_failed_rows(effects, failed_rows);
                        return Err(TransferFailure::failed("目标连接已断开"));
                    }
                };
                driver
                    .execute_query(&job.target_database, &insert_sql)
                    .await
            };
            if let Err(error) = insert_result {
                log::warn!("Insert failed: {error}");
                failed_rows += 1;
            } else {
                total_rows += 1;
                effects.record_applied(1);
            }
        }
        record_failed_rows(effects, failed_rows);

        task.progress(progress, format!("已复制 {total_rows} 行数据..."))
            .await;
        if is_last_page {
            break;
        }
        page += 1;
    }

    Ok(())
}

fn record_failed_rows(effects: &mut TransferEffects, failed_rows: u64) {
    effects.record_failure(failed_rows, "行写入失败");
}

fn ensure_not_cancelled(task: &TaskContext) -> Result<(), TransferFailure> {
    if task.is_cancelled() {
        Err(TransferFailure::cancelled("任务已取消"))
    } else {
        Ok(())
    }
}

fn copy_progress(step: usize, total_steps: usize) -> f32 {
    step as f32 / (total_steps + 1) as f32
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{copy_outcome, TransferEffects};
    use crate::db::{DbType, SqlDialect};
    use crate::tasks::TaskOutcome;

    #[test]
    fn delegates_create_table_retargeting_to_the_dialect() {
        let dialect = SqlDialect::new(DbType::ClickHouse);
        assert_eq!(
            dialect
                .retarget_create_table(
                    "CREATE TABLE analytics.events\n(\n    `id` UInt64\n)\nENGINE = MergeTree",
                    "events_copy",
                )
                .unwrap(),
            "CREATE TABLE `events_copy`\n(\n    `id` UInt64\n)\nENGINE = MergeTree"
        );
    }

    #[test]
    fn escapes_clickhouse_values_during_copy() {
        assert_eq!(
            SqlDialect::new(DbType::ClickHouse)
                .literal(&Value::String("it's\\ready".to_string()))
                .unwrap(),
            "'it\\'s\\\\ready'"
        );
    }

    #[test]
    fn classifies_page_and_row_failures_by_applied_effects() {
        let mut failed = TransferEffects::default();
        failed.record_failure(1, "读取第 2 页源数据失败：timeout");
        failed.record_failure(3, "3 行写入失败");
        assert!(matches!(
            copy_outcome(failed),
            TaskOutcome::Failed(message)
                if message == "复制失败：已应用 0 项更改；4 项失败；读取第 2 页源数据失败：timeout；3 行写入失败"
        ));

        let mut partial = TransferEffects::default();
        partial.record_applied(2);
        partial.record_failure(1, "读取第 2 页源数据失败：timeout");
        partial.record_failure(3, "3 行写入失败");
        assert!(matches!(
            copy_outcome(partial),
            TaskOutcome::Partial(message)
                if message == "复制部分完成：已应用 2 项更改；4 项失败；读取第 2 页源数据失败：timeout；3 行写入失败"
        ));
        assert!(matches!(
            copy_outcome(TransferEffects::default()),
            TaskOutcome::Completed(message) if message == "复制完成"
        ));
    }
}
