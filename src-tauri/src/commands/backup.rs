use std::collections::{HashMap, HashSet};
use serde::Deserialize;
use tauri::{Emitter, State};
use tokio_util::sync::CancellationToken;
use crate::db::{DbType, ForeignKeyInfo, IndexInfo};
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

/// Quote a SQL identifier according to database type.
fn quote_ident(name: &str, db_type: &DbType) -> String {
    match db_type {
        DbType::MySQL | DbType::SQLite => format!("`{}`", name),
        DbType::PostgreSQL => format!("\"{}\"", name),
        DbType::SQLServer => format!("[{}]", name),
        _ => name.to_string(),
    }
}

/// Sort tables by foreign key dependencies (topological sort).
/// Tables that are referenced by other tables come first so they can be
/// created/inserted before the tables that depend on them.
fn sort_tables_by_deps(tables: &[String], fk_map: &HashMap<String, Vec<ForeignKeyInfo>>) -> Vec<String> {
    // Build adjacency: table -> set of tables it depends on (references)
    let table_set: HashSet<&str> = tables.iter().map(|s| s.as_str()).collect();
    let mut deps: HashMap<&str, HashSet<&str>> = HashMap::new();
    for t in tables {
        deps.insert(t.as_str(), HashSet::new());
    }
    for (table, fks) in fk_map {
        if let Some(dep_set) = deps.get_mut(table.as_str()) {
            for fk in fks {
                // Only count dependencies within the backup set, skip self-references
                if fk.to_table != *table && table_set.contains(fk.to_table.as_str()) {
                    dep_set.insert(fk.to_table.as_str());
                }
            }
        }
    }

    // Kahn's algorithm for topological sort
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut reverse: HashMap<&str, Vec<&str>> = HashMap::new();
    for t in tables {
        in_degree.insert(t.as_str(), deps.get(t.as_str()).map_or(0, |d| d.len()));
    }
    for (table, dep_set) in &deps {
        for dep in dep_set {
            reverse.entry(*dep).or_default().push(table);
        }
    }

    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&t, _)| t)
        .collect();
    queue.sort(); // deterministic order
    let mut sorted = Vec::new();

    while let Some(t) = queue.pop() {
        sorted.push(t.to_string());
        if let Some(dependents) = reverse.get(t) {
            for dep in dependents {
                if let Some(deg) = in_degree.get_mut(dep) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push(dep);
                    }
                }
            }
        }
        queue.sort(); // keep deterministic
    }

    // Append any remaining tables (circular deps) in original order
    let sorted_set: HashSet<String> = sorted.iter().cloned().collect();
    for t in tables {
        if !sorted_set.contains(t) {
            sorted.push(t.clone());
        }
    }

    sorted
}

/// Generate PostgreSQL CREATE INDEX statements for a table's non-primary indexes.
fn pg_create_index_stmts(table: &str, schema: &str, indexes: &[IndexInfo]) -> Vec<String> {
    let mut stmts = Vec::new();
    for idx in indexes {
        if idx.is_primary {
            continue; // primary key is part of CREATE TABLE
        }
        let unique = if idx.is_unique { "UNIQUE " } else { "" };
        let cols: Vec<String> = idx.columns.iter().map(|c| format!("\"{}\"", c)).collect();
        stmts.push(format!(
            "CREATE {}INDEX \"{}\" ON \"{}\".\"{}\" ({});",
            unique, idx.name, schema, table, cols.join(", ")
        ));
    }
    stmts
}

/// Generate statements to reset auto-increment / sequences after data load.
fn reset_auto_increment_stmts(table: &str, db_type: &DbType, schema: Option<&str>) -> Vec<String> {
    match db_type {
        DbType::MySQL => {
            // MySQL: reset AUTO_INCREMENT to max(id)+1 via a single statement
            vec![format!(
                "-- Reset auto-increment for `{0}`\n\
                 SET @max_id = (SELECT COALESCE(MAX(id), 0) FROM `{0}`);\n\
                 SET @sql = CONCAT('ALTER TABLE `{0}` AUTO_INCREMENT = ', @max_id + 1);\n\
                 PREPARE stmt FROM @sql;\n\
                 EXECUTE stmt;\n\
                 DEALLOCATE PREPARE stmt;",
                table
            )]
        }
        DbType::PostgreSQL => {
            let s = schema.unwrap_or("public");
            // PostgreSQL: reset all sequences owned by columns of this table
            vec![format!(
                "DO $$ DECLARE seq RECORD; max_val BIGINT; BEGIN \
                 FOR seq IN SELECT column_name, pg_get_serial_sequence('{s}.{t}', column_name) AS seqname \
                 FROM information_schema.columns WHERE table_schema = '{s}' AND table_name = '{t}' \
                 AND pg_get_serial_sequence('{s}.{t}', column_name) IS NOT NULL \
                 LOOP \
                 EXECUTE format('SELECT COALESCE(MAX(%I), 0) FROM {s}.{t}', seq.column_name) INTO max_val; \
                 PERFORM setval(seq.seqname, GREATEST(max_val, 1)); \
                 END LOOP; \
                 END $$;",
                s = s,
                t = table,
            )]
        }
        _ => vec![],
    }
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

    // Gather metadata (short lock): tables, db_type, foreign keys, indexes
    let (tables, db_type, fk_map, index_map) = {
        let connections = state.connections.lock().await;
        let driver = connections.get(&connection_id).ok_or("连接不存在")?;
        let db_type = driver.db_type();
        let raw_tables: Vec<String> = if let Some(ref specific_tables) = options.tables {
            specific_tables.clone()
        } else {
            let all_tables = driver.get_tables(&database).await.map_err(|e| e.to_string())?;
            all_tables.into_iter().map(|t| {
                if let Some(ref schema) = t.schema {
                    if schema != "public" {
                        return format!("{}.{}", schema, t.name);
                    }
                }
                t.name
            }).collect()
        };

        // Gather foreign keys for dependency sorting
        let mut fk_map: HashMap<String, Vec<ForeignKeyInfo>> = HashMap::new();
        for t in &raw_tables {
            if let Ok(fks) = driver.get_foreign_keys(&database, t).await {
                if !fks.is_empty() {
                    fk_map.insert(t.clone(), fks);
                }
            }
        }

        // Gather indexes (needed for PostgreSQL)
        let mut index_map: HashMap<String, Vec<IndexInfo>> = HashMap::new();
        if db_type == DbType::PostgreSQL {
            for t in &raw_tables {
                if let Ok(indexes) = driver.get_indexes(&database, t).await {
                    if !indexes.is_empty() {
                        index_map.insert(t.clone(), indexes);
                    }
                }
            }
        }

        (raw_tables, db_type, fk_map, index_map)
    };

    // Sort tables by foreign key dependencies
    let sorted_tables = sort_tables_by_deps(&tables, &fk_map);

    // Register task
    let task_id = uuid::Uuid::new_v4().to_string();
    let cancel_token = CancellationToken::new();
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
    state.task_manager.register_token(&task_id, cancel_token.clone()).await;

    // Clone Arc references for the spawned task
    let connections = state.connections.clone();
    let tasks_ref = state.task_manager.tasks.clone();
    let tokens_ref = state.task_manager.cancellation_tokens.clone();
    let task_id_clone = task_id.clone();

    tokio::spawn(async move {
        let total = sorted_tables.len();
        let mut sql_output = String::new();

        sql_output.push_str(&format!(
            "-- Astesia Database Backup\n-- Database: {}\n-- Date: {}\n\n",
            database,
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S")
        ));

        // For restore: disable FK checks at the start, re-enable at end
        match db_type {
            DbType::MySQL => sql_output.push_str("SET FOREIGN_KEY_CHECKS = 0;\n\n"),
            DbType::SQLite => sql_output.push_str("PRAGMA foreign_keys = OFF;\n\n"),
            _ => {}
        }

        let mut error: Option<String> = None;

        // DROP phase: iterate in reverse dependency order (dependents first)
        if options.add_drop_table || options.add_drop_if_exists {
            let drop_order: Vec<&String> = sorted_tables.iter().rev().collect();
            for table_name in &drop_order {
                let (schema, tbl) = parse_table_ref(table_name);

                // PostgreSQL: drop non-primary indexes first
                if db_type == DbType::PostgreSQL {
                    if let Some(indexes) = index_map.get(table_name.as_str()) {
                        for idx in indexes {
                            if idx.is_primary { continue; }
                            if options.add_drop_if_exists {
                                sql_output.push_str(&format!(
                                    "DROP INDEX IF EXISTS \"{}\".\"{}\";\n", schema, idx.name
                                ));
                            } else {
                                sql_output.push_str(&format!(
                                    "DROP INDEX \"{}\".\"{}\";\n", schema, idx.name
                                ));
                            }
                        }
                    }
                }

                let quoted_table = if db_type == DbType::PostgreSQL {
                    format!("\"{}\".\"{}\"", schema, tbl)
                } else {
                    quote_ident(tbl, &db_type)
                };

                if options.add_drop_if_exists {
                    sql_output.push_str(&format!("DROP TABLE IF EXISTS {};\n", quoted_table));
                } else {
                    sql_output.push_str(&format!("DROP TABLE {};\n", quoted_table));
                }
            }
            sql_output.push('\n');
        }

        // CREATE + INSERT phase: iterate in dependency order (dependencies first)
        for (i, table_name) in sorted_tables.iter().enumerate() {
            // Check cancellation
            if cancel_token.is_cancelled() {
                error = Some("任务已取消".to_string());
                break;
            }

            let (_schema, tbl) = parse_table_ref(table_name);
            let quoted_table = if db_type == DbType::PostgreSQL {
                format!("\"{}\".\"{}\"", _schema, tbl)
            } else {
                quote_ident(tbl, &db_type)
            };

            if options.include_structure {
                let result = {
                    let conns = connections.lock().await;
                    match conns.get(&connection_id) {
                        Some(driver) => driver.get_create_table_sql(&database, table_name).await,
                        None => { error = Some("连接已断开".to_string()); break; }
                    }
                };
                match result {
                    Ok(create_sql) => {
                        sql_output.push_str(&create_sql);
                        sql_output.push_str(";\n\n");

                        // PostgreSQL: add CREATE INDEX statements
                        if db_type == DbType::PostgreSQL {
                            if let Some(indexes) = index_map.get(table_name.as_str()) {
                                let stmts = pg_create_index_stmts(tbl, _schema, indexes);
                                for stmt in stmts {
                                    sql_output.push_str(&stmt);
                                    sql_output.push('\n');
                                }
                                if !indexes.iter().all(|idx| idx.is_primary) {
                                    sql_output.push('\n');
                                }
                            }
                        }
                    }
                    Err(e) => {
                        sql_output.push_str(&format!(
                            "-- Error getting DDL for {}: {}\n\n", table_name, e
                        ));
                    }
                }
            }

            if options.include_data {
                let mut page = 1u32;
                let page_size = 1000u32;
                loop {
                    if cancel_token.is_cancelled() {
                        error = Some("任务已取消".to_string());
                        break;
                    }

                    let result = {
                        let conns = connections.lock().await;
                        match conns.get(&connection_id) {
                            Some(driver) => driver.get_table_data(&database, table_name, page, page_size).await,
                            None => { error = Some("连接已断开".to_string()); break; }
                        }
                    };
                    match result {
                        Ok(result) => {
                            if result.rows.is_empty() { break; }
                            let col_names: Vec<String> = result.columns.iter()
                                .map(|c| quote_ident(&c.name, &db_type))
                                .collect();
                            for row in &result.rows {
                                let values: Vec<String> = row.iter().map(|v| match v {
                                    serde_json::Value::Null => "NULL".to_string(),
                                    serde_json::Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
                                    serde_json::Value::Number(n) => n.to_string(),
                                    serde_json::Value::String(s) => format!("'{}'", s.replace('\'', "''")),
                                    _ => format!("'{}'", v.to_string().replace('\'', "''")),
                                }).collect();
                                sql_output.push_str(&format!(
                                    "INSERT INTO {} ({}) VALUES ({});\n",
                                    quoted_table, col_names.join(", "), values.join(", ")
                                ));
                            }
                            if result.rows.len() < page_size as usize { break; }
                            page += 1;
                        }
                        Err(_) => break,
                    }
                }
                if error.is_some() { break; }
                sql_output.push('\n');

                // Reset auto-increment / sequences after data for this table
                let schema_opt = if db_type == DbType::PostgreSQL { Some(_schema) } else { None };
                for stmt in reset_auto_increment_stmts(tbl, &db_type, schema_opt) {
                    sql_output.push_str(&stmt);
                    sql_output.push('\n');
                }
                sql_output.push('\n');
            }

            // Update progress
            let progress = (i + 1) as f32 / total as f32;
            {
                let mut tasks = tasks_ref.lock().await;
                if let Some(task) = tasks.get_mut(&task_id_clone) {
                    task.progress = progress;
                    task.message = format!("已处理 {}/{} 表", i + 1, total);
                }
            }
            let _ = app_handle.emit("task-progress", serde_json::json!({
                "id": task_id_clone,
                "progress": progress,
                "message": format!("已处理 {}/{} 表", i + 1, total),
            }));
        }

        // Re-enable FK checks
        if error.is_none() {
            match db_type {
                DbType::MySQL => sql_output.push_str("\nSET FOREIGN_KEY_CHECKS = 1;\n"),
                DbType::SQLite => sql_output.push_str("\nPRAGMA foreign_keys = ON;\n"),
                _ => {}
            }
        }

        // Finalize
        let (status, message) = if let Some(err) = error {
            if cancel_token.is_cancelled() {
                (crate::tasks::TaskStatus::Cancelled, err)
            } else {
                (crate::tasks::TaskStatus::Failed, err)
            }
        } else {
            match std::fs::write(&options.output_path, sql_output) {
                Ok(_) => (crate::tasks::TaskStatus::Completed, format!("备份完成: {} 个表", total)),
                Err(e) => (crate::tasks::TaskStatus::Failed, format!("写入文件失败: {}", e)),
            }
        };

        {
            let mut tasks = tasks_ref.lock().await;
            if let Some(task) = tasks.get_mut(&task_id_clone) {
                task.status = status;
                task.progress = 1.0;
                task.message = message;
                task.completed_at = Some(chrono::Utc::now());
            }
        }
        // Cleanup cancellation token
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

/// Parse "schema.table" or just "table" (defaults to "public" for schema).
fn parse_table_ref(table: &str) -> (&str, &str) {
    if let Some(dot) = table.find('.') {
        (&table[..dot], &table[dot + 1..])
    } else {
        ("public", table)
    }
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

    // Split by semicolons (basic splitting, skip comments and empty)
    let statements: Vec<String> = sql_content
        .split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && !s.starts_with("--"))
        .collect();

    let total = statements.len();
    let task_id = uuid::Uuid::new_v4().to_string();
    let cancel_token = CancellationToken::new();

    // Register task
    {
        let mut tasks = state.task_manager.tasks.lock().await;
        tasks.insert(task_id.clone(), crate::tasks::BackgroundTask {
            id: task_id.clone(),
            name: format!("恢复 {}", database),
            status: crate::tasks::TaskStatus::Running,
            progress: 0.0,
            message: "开始恢复...".to_string(),
            created_at: chrono::Utc::now(),
            completed_at: None,
        });
    }
    state.task_manager.register_token(&task_id, cancel_token.clone()).await;

    let connections = state.connections.clone();
    let tasks_ref = state.task_manager.tasks.clone();
    let tokens_ref = state.task_manager.cancellation_tokens.clone();
    let task_id_clone = task_id.clone();

    tokio::spawn(async move {
        let mut success_count = 0u64;
        let mut error_count = 0u64;
        let mut cancelled = false;

        for (i, stmt) in statements.iter().enumerate() {
            if cancel_token.is_cancelled() {
                cancelled = true;
                break;
            }

            let result = {
                let conns = connections.lock().await;
                match conns.get(&connection_id) {
                    Some(driver) => driver.execute_query(&database, stmt).await,
                    None => {
                        let mut tasks = tasks_ref.lock().await;
                        if let Some(task) = tasks.get_mut(&task_id_clone) {
                            task.status = crate::tasks::TaskStatus::Failed;
                            task.message = "连接已断开".to_string();
                            task.completed_at = Some(chrono::Utc::now());
                        }
                        let _ = app_handle.emit("task-complete", serde_json::json!({"id": task_id_clone}));
                        let mut tokens = tokens_ref.lock().await;
                        tokens.remove(&task_id_clone);
                        return;
                    }
                }
            };

            match result {
                Ok(_) => success_count += 1,
                Err(e) => {
                    error_count += 1;
                    log::warn!("Restore statement failed: {}", e);
                }
            }

            let progress = (i + 1) as f32 / total as f32;
            {
                let mut tasks = tasks_ref.lock().await;
                if let Some(task) = tasks.get_mut(&task_id_clone) {
                    task.progress = progress;
                    task.message = format!("已执行 {}/{} 语句 (失败: {})", i + 1, total, error_count);
                }
            }
            let _ = app_handle.emit("task-progress", serde_json::json!({
                "id": task_id_clone,
                "progress": progress,
                "message": format!("已执行 {}/{} 语句 (失败: {})", i + 1, total, error_count),
            }));
        }

        // Mark complete
        {
            let mut tasks = tasks_ref.lock().await;
            if let Some(task) = tasks.get_mut(&task_id_clone) {
                if cancelled {
                    task.status = crate::tasks::TaskStatus::Cancelled;
                    task.message = "已取消".to_string();
                } else {
                    task.status = crate::tasks::TaskStatus::Completed;
                    task.message = format!("恢复完成: 成功 {} / 失败 {}", success_count, error_count);
                }
                task.progress = 1.0;
                task.completed_at = Some(chrono::Utc::now());
            }
        }
        {
            let mut tokens = tokens_ref.lock().await;
            tokens.remove(&task_id_clone);
        }
        let _ = app_handle.emit("task-complete", serde_json::json!({"id": task_id_clone}));
    });

    Ok(task_id)
}
