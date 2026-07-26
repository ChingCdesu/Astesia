use serde::Deserialize;
use tauri::{Emitter, State};
use tokio_util::sync::CancellationToken;
use crate::db::DbType;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CopyOptions {
    pub include_structure: bool,
    pub include_data: bool,
    pub new_table_name: String,
}

/// Quote a SQL identifier according to database type
fn quote_ident(name: &str, db_type: &DbType) -> String {
    match db_type {
        DbType::MySQL | DbType::SQLite => {
            format!("`{}`", name.replace('`', "``"))
        }
        DbType::ClickHouse => {
            let escaped = name.replace('\\', "\\\\").replace('`', "\\`");
            format!("`{escaped}`")
        }
        DbType::PostgreSQL => format!("\"{}\"", name),
        DbType::SQLServer => format!("[{}]", name),
        _ => name.to_string(),
    }
}

fn sql_value(value: &serde_json::Value, db_type: &DbType) -> String {
    match value {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::Bool(value) => if *value { "1" } else { "0" }.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => quote_string_value(value, db_type),
        value => quote_string_value(&value.to_string(), db_type),
    }
}

fn quote_string_value(value: &str, db_type: &DbType) -> String {
    if db_type == &DbType::ClickHouse {
        let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
        format!("'{escaped}'")
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

fn replace_clickhouse_create_table_name(sql: &str, new_name: &str) -> String {
    const CREATE_TABLE: &str = "CREATE TABLE";
    const IF_NOT_EXISTS: &str = "IF NOT EXISTS";

    let upper = sql.to_ascii_uppercase();
    let Some(marker_start) = upper.find(CREATE_TABLE) else {
        return sql.to_string();
    };
    let bytes = sql.as_bytes();
    let mut name_start = marker_start + CREATE_TABLE.len();
    while bytes.get(name_start).is_some_and(u8::is_ascii_whitespace) {
        name_start += 1;
    }

    if upper[name_start..].starts_with(IF_NOT_EXISTS) {
        name_start += IF_NOT_EXISTS.len();
        while bytes.get(name_start).is_some_and(u8::is_ascii_whitespace) {
            name_start += 1;
        }
    }

    let mut name_end = name_start;
    let mut quoted_by = None;
    let mut escaped = false;
    while let Some(&byte) = bytes.get(name_end) {
        if let Some(quote) = quoted_by {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote {
                quoted_by = None;
            }
            name_end += 1;
            continue;
        }

        match byte {
            b'`' | b'"' => {
                quoted_by = Some(byte);
                name_end += 1;
            }
            b'(' => break,
            byte if byte.is_ascii_whitespace() => break,
            _ => name_end += 1,
        }
    }

    if name_end == name_start {
        return sql.to_string();
    }
    format!("{}{}{}", &sql[..name_start], new_name, &sql[name_end..])
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

    // Verify same DB type and get it (short lock)
    let db_type = {
        let connections = state.connections.lock().await;
        let source_driver = connections.get(&source_connection_id).ok_or("源连接不存在")?;
        let target_driver = connections.get(&target_connection_id).ok_or("目标连接不存在")?;
        if source_driver.db_type() != target_driver.db_type() {
            return Err("仅支持同类型数据库间复制".to_string());
        }
        source_driver.db_type()
    };

    // Register task
    let task_id = uuid::Uuid::new_v4().to_string();
    {
        let mut tasks = state.task_manager.tasks.lock().await;
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

    // Register cancellation token
    let cancel_token = CancellationToken::new();
    state.task_manager.register_token(&task_id, cancel_token.clone()).await;

    // Clone Arc references for the spawned task
    let connections = state.connections.clone();
    let tasks_ref = state.task_manager.tasks.clone();
    let tokens_ref = state.task_manager.cancellation_tokens.clone();
    let task_id_clone = task_id.clone();

    tokio::spawn(async move {
        let total_steps = (if options.include_structure { 1 } else { 0 })
            + (if options.include_data { 1 } else { 0 });
        let mut step = 0;

        let emit_progress = |app: &tauri::AppHandle, id: &str, progress: f32, message: &str| {
            let _ = app.emit("task-progress", serde_json::json!({
                "id": id,
                "progress": progress,
                "message": message,
            }));
        };

        let mark_failed = |tasks_ref: &std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, crate::tasks::BackgroundTask>>>,
                           tokens_ref: &std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, CancellationToken>>>,
                           task_id: &str,
                           message: String| {
            let tasks_ref = tasks_ref.clone();
            let tokens_ref = tokens_ref.clone();
            let task_id = task_id.to_string();
            async move {
                let mut tasks = tasks_ref.lock().await;
                if let Some(task) = tasks.get_mut(&task_id) {
                    task.status = crate::tasks::TaskStatus::Failed;
                    task.message = message;
                    task.completed_at = Some(chrono::Utc::now());
                }
                drop(tasks);
                let mut tokens = tokens_ref.lock().await;
                tokens.remove(&task_id);
            }
        };

        // Step 1: Copy structure
        if options.include_structure {
            step += 1;
            let progress = step as f32 / (total_steps + 1) as f32;
            {
                let mut tasks = tasks_ref.lock().await;
                if let Some(task) = tasks.get_mut(&task_id_clone) {
                    task.progress = progress;
                    task.message = "正在复制表结构...".to_string();
                }
            }
            emit_progress(&app_handle, &task_id_clone, progress, "正在复制表结构...");

            // Get DDL from source
            let create_sql = {
                let conns = connections.lock().await;
                match conns.get(&source_connection_id) {
                    Some(driver) => driver.get_create_table_sql(&source_database, &source_table).await,
                    None => {
                        mark_failed(&tasks_ref, &tokens_ref, &task_id_clone, "源连接已断开".to_string()).await;
                        let _ = app_handle.emit("task-complete", serde_json::json!({"id": task_id_clone}));
                        return;
                    }
                }
            };

            match create_sql {
                Ok(sql) => {
                    // Replace table name in DDL for all quote styles
                    let quoted_new_table = quote_ident(&options.new_table_name, &db_type);
                    let new_sql = if db_type == DbType::ClickHouse {
                        replace_clickhouse_create_table_name(&sql, &quoted_new_table)
                    } else {
                        let quoted_source_table = quote_ident(&source_table, &db_type);
                        sql.replace(
                            &format!(
                                "{}.{}",
                                quote_ident(&source_database, &db_type),
                                quoted_source_table
                            ),
                            &quoted_new_table,
                        )
                        .replace(&quoted_source_table, &quoted_new_table)
                        .replace(&format!("\"{}\"", source_table), &quoted_new_table)
                        .replace(&format!("[{}]", source_table), &quoted_new_table)
                    };

                    let exec_result = {
                        let conns = connections.lock().await;
                        match conns.get(&target_connection_id) {
                            Some(driver) => driver.execute_query(&target_database, &new_sql).await,
                            None => {
                                mark_failed(&tasks_ref, &tokens_ref, &task_id_clone, "目标连接已断开".to_string()).await;
                                let _ = app_handle.emit("task-complete", serde_json::json!({"id": task_id_clone}));
                                return;
                            }
                        }
                    };

                    if let Err(e) = exec_result {
                        mark_failed(&tasks_ref, &tokens_ref, &task_id_clone, format!("创建表失败: {}", e)).await;
                        let _ = app_handle.emit("task-complete", serde_json::json!({"id": task_id_clone}));
                        return;
                    }
                }
                Err(e) => {
                    mark_failed(&tasks_ref, &tokens_ref, &task_id_clone, format!("获取表结构失败: {}", e)).await;
                    let _ = app_handle.emit("task-complete", serde_json::json!({"id": task_id_clone}));
                    return;
                }
            }
        }

        // Step 2: Copy data
        if options.include_data {
            step += 1;
            let progress = step as f32 / (total_steps + 1) as f32;
            {
                let mut tasks = tasks_ref.lock().await;
                if let Some(task) = tasks.get_mut(&task_id_clone) {
                    task.progress = progress;
                    task.message = "正在复制数据...".to_string();
                }
            }
            emit_progress(&app_handle, &task_id_clone, progress, "正在复制数据...");

            let mut page = 1u32;
            let page_size = 1000u32;
            let mut total_rows = 0u64;
            let quoted_new_table = quote_ident(&options.new_table_name, &db_type);

            loop {
                // Check for cancellation
                if cancel_token.is_cancelled() {
                    let mut tasks = tasks_ref.lock().await;
                    if let Some(task) = tasks.get_mut(&task_id_clone) {
                        task.status = crate::tasks::TaskStatus::Cancelled;
                        task.message = "已取消".to_string();
                        task.completed_at = Some(chrono::Utc::now());
                    }
                    drop(tasks);
                    let mut tokens = tokens_ref.lock().await;
                    tokens.remove(&task_id_clone);
                    let _ = app_handle.emit("task-complete", serde_json::json!({"id": task_id_clone}));
                    return;
                }

                let fetch_result = {
                    let conns = connections.lock().await;
                    match conns.get(&source_connection_id) {
                        Some(driver) => {
                            driver.get_table_data(&source_database, &source_table, page, page_size).await
                        }
                        None => {
                            mark_failed(&tasks_ref, &tokens_ref, &task_id_clone, "源连接已断开".to_string()).await;
                            let _ = app_handle.emit("task-complete", serde_json::json!({"id": task_id_clone}));
                            return;
                        }
                    }
                };

                match fetch_result {
                    Ok(result) => {
                        if result.rows.is_empty() { break; }

                        let col_names: Vec<String> = result
                            .columns
                            .iter()
                            .map(|c| quote_ident(&c.name, &db_type))
                            .collect();
                        let is_last_page = result.rows.len() < page_size as usize;

                        for row in &result.rows {
                            let values: Vec<String> = row
                                .iter()
                                .map(|value| sql_value(value, &db_type))
                                .collect();

                            let insert_sql = format!(
                                "INSERT INTO {} ({}) VALUES ({})",
                                quoted_new_table,
                                col_names.join(", "),
                                values.join(", ")
                            );

                            let exec_result = {
                                let conns = connections.lock().await;
                                match conns.get(&target_connection_id) {
                                    Some(driver) => driver.execute_query(&target_database, &insert_sql).await,
                                    None => {
                                        mark_failed(&tasks_ref, &tokens_ref, &task_id_clone, "目标连接已断开".to_string()).await;
                                        let _ = app_handle.emit("task-complete", serde_json::json!({"id": task_id_clone}));
                                        return;
                                    }
                                }
                            };

                            if let Err(e) = exec_result {
                                log::warn!("Insert failed: {}", e);
                            } else {
                                total_rows += 1;
                            }
                        }

                        let msg = format!("已复制 {} 行数据...", total_rows);
                        {
                            let mut tasks = tasks_ref.lock().await;
                            if let Some(task) = tasks.get_mut(&task_id_clone) {
                                task.message = msg.clone();
                            }
                        }
                        emit_progress(&app_handle, &task_id_clone, step as f32 / (total_steps + 1) as f32, &msg);

                        if is_last_page { break; }
                        page += 1;
                    }
                    Err(_) => break,
                }
            }
        }

        // Mark complete & cleanup token
        {
            let mut tasks = tasks_ref.lock().await;
            if let Some(task) = tasks.get_mut(&task_id_clone) {
                task.status = crate::tasks::TaskStatus::Completed;
                task.progress = 1.0;
                task.message = "复制完成".to_string();
                task.completed_at = Some(chrono::Utc::now());
            }
        }
        {
            let mut tokens = tokens_ref.lock().await;
            tokens.remove(&task_id_clone);
        }
        let _ = app_handle.emit("task-complete", serde_json::json!({
            "id": task_id_clone,
        }));
    });

    Ok(task_id)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{replace_clickhouse_create_table_name, sql_value};
    use crate::db::DbType;

    #[test]
    fn replaces_quoted_and_unquoted_clickhouse_table_names() {
        assert_eq!(
            replace_clickhouse_create_table_name(
                "CREATE TABLE analytics.events\n(\n    `id` UInt64\n)\nENGINE = MergeTree",
                "`events_copy`",
            ),
            "CREATE TABLE `events_copy`\n(\n    `id` UInt64\n)\nENGINE = MergeTree"
        );
        assert_eq!(
            replace_clickhouse_create_table_name(
                "CREATE TABLE IF NOT EXISTS `analytics`.`odd\\`name` (`id` UInt64)",
                "`copy`",
            ),
            "CREATE TABLE IF NOT EXISTS `copy` (`id` UInt64)"
        );
    }

    #[test]
    fn escapes_clickhouse_values_during_copy() {
        assert_eq!(
            sql_value(&Value::String("it's\\ready".to_string()), &DbType::ClickHouse),
            "'it\\'s\\\\ready'"
        );
    }
}
