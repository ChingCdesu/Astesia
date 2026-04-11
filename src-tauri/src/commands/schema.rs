use tauri::State;

use crate::db::{ColumnInfo, IndexInfo, TableInfo};
use crate::state::AppState;

#[tauri::command]
pub async fn get_databases(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Vec<String>, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or_else(|| "连接不存在".to_string())?;
    driver
        .get_databases()
        .await
        .map_err(|e| format!("获取数据库列表失败: {}", e))
}

#[tauri::command]
pub async fn get_tables(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
) -> Result<Vec<TableInfo>, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or_else(|| "连接不存在".to_string())?;
    driver
        .get_tables(&database)
        .await
        .map_err(|e| format!("获取表列表失败: {}", e))
}

#[tauri::command]
pub async fn get_columns(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
) -> Result<Vec<ColumnInfo>, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or_else(|| "连接不存在".to_string())?;
    driver
        .get_columns(&database, &table)
        .await
        .map_err(|e| format!("获取列信息失败: {}", e))
}

#[tauri::command]
pub async fn get_indexes(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
) -> Result<Vec<IndexInfo>, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or_else(|| "连接不存在".to_string())?;
    driver
        .get_indexes(&database, &table)
        .await
        .map_err(|e| format!("获取索引信息失败: {}", e))
}
