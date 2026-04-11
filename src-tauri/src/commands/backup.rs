use serde::Deserialize;
use tauri::State;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct BackupOptions {
    pub tables: Option<Vec<String>>,
    pub include_structure: bool,
    pub include_data: bool,
    pub add_drop_table: bool,
    pub add_drop_if_exists: bool,
    pub output_path: String,
}

#[tauri::command]
pub async fn start_backup(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    options: BackupOptions,
) -> Result<String, String> {
    let app_handle = {
        let h = state.app_handle.lock().await;
        h.clone().ok_or("App handle not initialized")?
    };

    // Get table list
    let tables = {
        let connections = state.connections.lock().await;
        let driver = connections.get(&connection_id).ok_or("连接不存在")?;
        if let Some(ref specific_tables) = options.tables {
            specific_tables.clone()
        } else {
            let all_tables = driver.get_tables(&database).await.map_err(|e| e.to_string())?;
            all_tables.into_iter().map(|t| t.name).collect()
        }
    };

    // Register task
    let task_id = uuid::Uuid::new_v4().to_string();
    {
        let mut tasks = state.task_manager.tasks.lock().await;
        tasks.insert(task_id.clone(), crate::tasks::BackgroundTask {
            id: task_id.clone(),
            name: format!("备份 {}", database),
            status: crate::tasks::TaskStatus::Running,
            progress: 0.0,
            message: "开始备份...".to_string(),
            created_at: chrono::Utc::now(),
            completed_at: None,
        });
    }

    let task_manager = &state.task_manager;
    let connections = state.connections.lock().await;
    let driver = connections.get(&connection_id).ok_or("连接不存在")?;

    let total = tables.len();
    let mut sql_output = String::new();

    sql_output.push_str(&format!(
        "-- Astesia Database Backup\n-- Database: {}\n-- Date: {}\n\n",
        database,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S")
    ));

    for (i, table_name) in tables.iter().enumerate() {
        if options.add_drop_table {
            if options.add_drop_if_exists {
                sql_output.push_str(&format!("DROP TABLE IF EXISTS `{}`;\n", table_name));
            } else {
                sql_output.push_str(&format!("DROP TABLE `{}`;\n", table_name));
            }
        }

        if options.include_structure {
            match driver.get_create_table_sql(&database, table_name).await {
                Ok(create_sql) => {
                    sql_output.push_str(&create_sql);
                    sql_output.push_str(";\n\n");
                }
                Err(e) => {
                    sql_output.push_str(&format!(
                        "-- Error getting DDL for {}: {}\n\n",
                        table_name, e
                    ));
                }
            }
        }

        if options.include_data {
            let mut page = 1u32;
            let page_size = 1000u32;
            loop {
                match driver
                    .get_table_data(&database, table_name, page, page_size)
                    .await
                {
                    Ok(result) => {
                        if result.rows.is_empty() {
                            break;
                        }
                        let col_names: Vec<String> =
                            result.columns.iter().map(|c| format!("`{}`", c.name)).collect();
                        for row in &result.rows {
                            let values: Vec<String> = row
                                .iter()
                                .map(|v| match v {
                                    serde_json::Value::Null => "NULL".to_string(),
                                    serde_json::Value::Bool(b) => {
                                        if *b { "1" } else { "0" }.to_string()
                                    }
                                    serde_json::Value::Number(n) => n.to_string(),
                                    serde_json::Value::String(s) => {
                                        format!("'{}'", s.replace('\'', "''"))
                                    }
                                    _ => format!("'{}'", v.to_string().replace('\'', "''")),
                                })
                                .collect();
                            sql_output.push_str(&format!(
                                "INSERT INTO `{}` ({}) VALUES ({});\n",
                                table_name,
                                col_names.join(", "),
                                values.join(", ")
                            ));
                        }
                        if result.rows.len() < page_size as usize {
                            break;
                        }
                        page += 1;
                    }
                    Err(_) => break,
                }
            }
            sql_output.push('\n');
        }

        // Update progress
        let progress = (i + 1) as f32 / total as f32;
        task_manager
            .update_progress(
                &task_id,
                progress,
                format!("已处理 {}/{} 表", i + 1, total),
                &app_handle,
            )
            .await;
    }

    // Write to file
    std::fs::write(&options.output_path, sql_output)
        .map_err(|e| format!("写入文件失败: {}", e))?;

    // Mark task complete
    {
        let mut tasks = task_manager.tasks.lock().await;
        if let Some(task) = tasks.get_mut(&task_id) {
            task.status = crate::tasks::TaskStatus::Completed;
            task.progress = 1.0;
            task.message = format!("备份完成: {} 个表", total);
            task.completed_at = Some(chrono::Utc::now());
        }
    }

    Ok(task_id)
}

#[tauri::command]
pub async fn start_restore(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    file_path: String,
) -> Result<String, String> {
    let app_handle = {
        let h = state.app_handle.lock().await;
        h.clone().ok_or("App handle not initialized")?
    };

    let sql_content =
        std::fs::read_to_string(&file_path).map_err(|e| format!("读取文件失败: {}", e))?;

    // Split by semicolons (basic splitting, skip comments)
    let statements: Vec<&str> = sql_content
        .split(';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && !s.starts_with("--"))
        .collect();

    let total = statements.len();
    let task_id = uuid::Uuid::new_v4().to_string();

    // Register task
    {
        let mut tasks = state.task_manager.tasks.lock().await;
        tasks.insert(
            task_id.clone(),
            crate::tasks::BackgroundTask {
                id: task_id.clone(),
                name: format!("恢复 {}", database),
                status: crate::tasks::TaskStatus::Running,
                progress: 0.0,
                message: "开始恢复...".to_string(),
                created_at: chrono::Utc::now(),
                completed_at: None,
            },
        );
    }

    let connections = state.connections.lock().await;
    let driver = connections.get(&connection_id).ok_or("连接不存在")?;

    let mut success_count = 0;
    let mut error_count = 0;

    for (i, stmt) in statements.iter().enumerate() {
        match driver.execute_query(&database, stmt).await {
            Ok(_) => success_count += 1,
            Err(e) => {
                error_count += 1;
                log::warn!("Restore statement failed: {}", e);
            }
        }

        let progress = (i + 1) as f32 / total as f32;
        state
            .task_manager
            .update_progress(
                &task_id,
                progress,
                format!("已执行 {}/{} 语句 (失败: {})", i + 1, total, error_count),
                &app_handle,
            )
            .await;
    }

    // Mark complete
    {
        let mut tasks = state.task_manager.tasks.lock().await;
        if let Some(task) = tasks.get_mut(&task_id) {
            task.status = crate::tasks::TaskStatus::Completed;
            task.progress = 1.0;
            task.message = format!("恢复完成: 成功 {} / 失败 {}", success_count, error_count);
            task.completed_at = Some(chrono::Utc::now());
        }
    }

    Ok(task_id)
}
