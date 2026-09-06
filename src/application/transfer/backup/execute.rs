use crate::application::atomic_output::AtomicOutput;
use crate::connection_runtime::DriverHandle;
use crate::tasks::{TaskContext, TaskOutcome};

use super::plan::BackupPlan;
use super::render::BackupRenderer;

const PAGE_SIZE: u32 = 1_000;

pub(super) async fn execute_backup(
    plan: BackupPlan,
    driver: DriverHandle,
    task: TaskContext,
) -> TaskOutcome {
    let total = plan.tables.len();
    let mut renderer = BackupRenderer::new(plan.db_type, &plan.database);
    if let Err(error) = renderer.render_drop_tables(&plan.tables, plan.drop_tables) {
        return TaskOutcome::Failed(format!("生成 DROP TABLE 语句失败: {error}"));
    }
    if task.is_cancelled() {
        return TaskOutcome::Cancelled("任务已取消".to_string());
    }
    let mut output = match AtomicOutput::new(&plan.output_path) {
        Ok(output) => output,
        Err(error) => return TaskOutcome::Failed(format!("写入文件失败: {error}")),
    };
    if let Err(error) = renderer.drain_output(&mut output) {
        return TaskOutcome::Failed(format!("写入文件失败: {error}"));
    }

    let mut fatal_error = None;
    let mut partial_failures = Vec::new();
    'tables: for (index, table) in plan.tables.iter().enumerate() {
        if task.is_cancelled() {
            return TaskOutcome::Cancelled("任务已取消".to_string());
        }

        if plan.content.includes_structure() {
            let create_sql = match driver.lock_active().await {
                Ok(driver) => {
                    driver
                        .get_create_table_sql(&plan.database, &table.reference)
                        .await
                }
                Err(_) => {
                    fatal_error = Some("连接已断开".to_string());
                    break 'tables;
                }
            };
            match create_sql {
                Ok(create_sql) => {
                    if let Err(error) = renderer.render_structure(table, &create_sql) {
                        partial_failures
                            .push(format!("表 {} 的结构渲染失败: {error}", table.reference));
                        renderer.render_structure_error(&table.reference, &error.to_string());
                    }
                }
                Err(error) => {
                    partial_failures
                        .push(format!("表 {} 的结构读取失败: {error}", table.reference));
                    renderer.render_structure_error(&table.reference, &error.to_string());
                }
            }
            if let Err(error) = renderer.drain_output(&mut output) {
                return TaskOutcome::Failed(format!("写入文件失败: {error}"));
            }
        }

        if plan.content.includes_data() {
            let mut page = 1;
            loop {
                if task.is_cancelled() {
                    return TaskOutcome::Cancelled("任务已取消".to_string());
                }

                let result = match driver.lock_active().await {
                    Ok(driver) => {
                        driver
                            .get_table_data(&plan.database, &table.reference, page, PAGE_SIZE)
                            .await
                    }
                    Err(_) => {
                        fatal_error = Some("连接已断开".to_string());
                        break;
                    }
                };
                match result {
                    Ok(result) => {
                        if result.rows.is_empty() {
                            break;
                        }
                        let row_count = result.rows.len();
                        if let Err(error) = renderer.render_data_page(table, &result) {
                            partial_failures.push(format!(
                                "表 {} 第 {page} 页数据渲染失败: {error}",
                                table.reference
                            ));
                            break;
                        }
                        if let Err(error) = renderer.drain_output(&mut output) {
                            return TaskOutcome::Failed(format!("写入文件失败: {error}"));
                        }
                        if row_count < PAGE_SIZE as usize {
                            break;
                        }
                        page += 1;
                    }
                    Err(error) => {
                        partial_failures.push(format!(
                            "表 {} 第 {page} 页数据读取失败: {error}",
                            table.reference
                        ));
                        break;
                    }
                }
            }
            if fatal_error.is_some() {
                break 'tables;
            }
            if let Err(error) = renderer.finish_table_data(table) {
                partial_failures.push(format!(
                    "表 {} 的自增序列渲染失败: {error}",
                    table.reference
                ));
            }
        }
        if let Err(error) = renderer.drain_output(&mut output) {
            return TaskOutcome::Failed(format!("写入文件失败: {error}"));
        }

        let progress = (index + 1) as f32 / total as f32;
        task.progress(progress, format!("已处理 {}/{} 表", index + 1, total))
            .await;
    }

    if let Some(error) = fatal_error {
        if task.is_cancelled() {
            return TaskOutcome::Cancelled(error);
        }
        return TaskOutcome::Failed(error);
    }
    if task.is_cancelled() {
        return TaskOutcome::Cancelled("已取消".to_string());
    }

    renderer.finish_success();
    if let Err(error) = renderer.drain_output(&mut output) {
        return TaskOutcome::Failed(format!("写入文件失败: {error}"));
    }
    if task.is_cancelled() {
        return TaskOutcome::Cancelled("已取消".to_string());
    }
    match output.commit() {
        Ok(()) => backup_outcome(total, partial_failures),
        Err(error) => TaskOutcome::Failed(format!("写入文件失败: {error}")),
    }
}

fn backup_outcome(total: usize, partial_failures: Vec<String>) -> TaskOutcome {
    if partial_failures.is_empty() {
        TaskOutcome::Completed(format!("备份完成: {total} 个表"))
    } else {
        TaskOutcome::Partial(format!(
            "备份部分完成: {total} 个表；{}",
            partial_failures.join("；")
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::tasks::TaskOutcome;

    use super::backup_outcome;

    #[test]
    fn reports_omitted_backup_content_as_partial_completion() {
        assert!(matches!(
            backup_outcome(3, vec!["表 users 的结构读取失败".to_string()]),
            TaskOutcome::Partial(message)
                if message.contains("备份部分完成") && message.contains("users")
        ));
    }

    #[test]
    fn reports_complete_backup_when_no_content_was_omitted() {
        assert!(matches!(
            backup_outcome(3, Vec::new()),
            TaskOutcome::Completed(message) if message == "备份完成: 3 个表"
        ));
    }
}
