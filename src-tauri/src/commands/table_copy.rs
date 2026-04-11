use serde::Deserialize;
use tauri::State;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CopyOptions {
    pub include_structure: bool,
    pub include_data: bool,
    pub new_table_name: String,
}

#[tauri::command]
pub async fn copy_table(
    state: State<'_, AppState>,
    source_connection_id: String,
    source_database: String,
    source_table: String,
    target_connection_id: String,
    target_database: String,
    options: CopyOptions,
) -> Result<String, String> {
    let app_handle = {
        let h = state.app_handle.lock().await;
        h.clone().ok_or("App handle not initialized")?
    };

    let task_manager = &state.task_manager;

    // Register task
    let task_id = uuid::Uuid::new_v4().to_string();
    {
        let mut tasks = task_manager.tasks.lock().await;
        tasks.insert(task_id.clone(), crate::tasks::BackgroundTask {
            id: task_id.clone(),
            name: format!("复制表 {} → {}", source_table, options.new_table_name),
            status: crate::tasks::TaskStatus::Running,
            progress: 0.0,
            message: "开始复制...".to_string(),
            created_at: chrono::Utc::now(),
            completed_at: None,
        });
    }

    let connections = state.connections.lock().await;
    let source_driver = connections.get(&source_connection_id).ok_or("源连接不存在")?;
    let target_driver = connections.get(&target_connection_id).ok_or("目标连接不存在")?;

    // Verify same DB type
    if source_driver.db_type() != target_driver.db_type() {
        return Err("仅支持同类型数据库间复制".to_string());
    }

    let mut step = 0;
    let total_steps = (if options.include_structure { 1 } else { 0 }) + (if options.include_data { 1 } else { 0 });

    // Step 1: Copy structure
    if options.include_structure {
        step += 1;
        task_manager.update_progress(&task_id, step as f32 / (total_steps + 1) as f32, "正在复制表结构...".to_string(), &app_handle).await;

        match source_driver.get_create_table_sql(&source_database, &source_table).await {
            Ok(create_sql) => {
                // Replace table name in DDL
                let new_sql = create_sql
                    .replace(&format!("`{}`", source_table), &format!("`{}`", options.new_table_name))
                    .replace(&format!("\"{}\"", source_table), &format!("\"{}\"", options.new_table_name))
                    .replace(&format!("[{}]", source_table), &format!("[{}]", options.new_table_name));

                if let Err(e) = target_driver.execute_query(&target_database, &new_sql).await {
                    // Mark failed
                    let mut tasks = task_manager.tasks.lock().await;
                    if let Some(task) = tasks.get_mut(&task_id) {
                        task.status = crate::tasks::TaskStatus::Failed;
                        task.message = format!("创建表失败: {}", e);
                        task.completed_at = Some(chrono::Utc::now());
                    }
                    return Err(format!("创建表失败: {}", e));
                }
            }
            Err(e) => {
                let mut tasks = task_manager.tasks.lock().await;
                if let Some(task) = tasks.get_mut(&task_id) {
                    task.status = crate::tasks::TaskStatus::Failed;
                    task.message = format!("获取表结构失败: {}", e);
                    task.completed_at = Some(chrono::Utc::now());
                }
                return Err(format!("获取表结构失败: {}", e));
            }
        }
    }

    // Step 2: Copy data
    if options.include_data {
        step += 1;
        task_manager.update_progress(&task_id, step as f32 / (total_steps + 1) as f32, "正在复制数据...".to_string(), &app_handle).await;

        let mut page = 1u32;
        let page_size = 1000u32;
        let mut total_rows = 0u64;

        loop {
            match source_driver.get_table_data(&source_database, &source_table, page, page_size).await {
                Ok(result) => {
                    if result.rows.is_empty() { break; }

                    let col_names: Vec<String> = result.columns.iter().map(|c| format!("`{}`", c.name)).collect();

                    for row in &result.rows {
                        let values: Vec<String> = row.iter().map(|v| match v {
                            serde_json::Value::Null => "NULL".to_string(),
                            serde_json::Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
                            serde_json::Value::Number(n) => n.to_string(),
                            serde_json::Value::String(s) => format!("'{}'", s.replace('\'', "''")),
                            _ => format!("'{}'", v.to_string().replace('\'', "''")),
                        }).collect();

                        let insert_sql = format!(
                            "INSERT INTO `{}` ({}) VALUES ({})",
                            options.new_table_name,
                            col_names.join(", "),
                            values.join(", ")
                        );

                        if let Err(e) = target_driver.execute_query(&target_database, &insert_sql).await {
                            log::warn!("Insert failed: {}", e);
                        } else {
                            total_rows += 1;
                        }
                    }

                    task_manager.update_progress(
                        &task_id,
                        step as f32 / (total_steps + 1) as f32,
                        format!("已复制 {} 行数据...", total_rows),
                        &app_handle,
                    ).await;

                    if result.rows.len() < page_size as usize { break; }
                    page += 1;
                }
                Err(_) => break,
            }
        }
    }

    // Mark complete
    {
        let mut tasks = task_manager.tasks.lock().await;
        if let Some(task) = tasks.get_mut(&task_id) {
            task.status = crate::tasks::TaskStatus::Completed;
            task.progress = 1.0;
            task.message = "复制完成".to_string();
            task.completed_at = Some(chrono::Utc::now());
        }
    }

    Ok(task_id)
}
