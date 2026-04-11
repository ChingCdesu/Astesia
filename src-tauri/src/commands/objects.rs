use tauri::State;

use crate::db::{ForeignKeyInfo, FunctionInfo, ProcedureInfo, TriggerInfo, UserInfo, ViewInfo};
use crate::state::AppState;

#[tauri::command]
pub async fn get_views(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
) -> Result<Vec<ViewInfo>, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or_else(|| "连接不存在".to_string())?;
    driver
        .get_views(&database)
        .await
        .map_err(|e| format!("获取视图失败: {}", e))
}

#[tauri::command]
pub async fn get_functions(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
) -> Result<Vec<FunctionInfo>, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or_else(|| "连接不存在".to_string())?;
    driver
        .get_functions(&database)
        .await
        .map_err(|e| format!("获取函数失败: {}", e))
}

#[tauri::command]
pub async fn get_procedures(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
) -> Result<Vec<ProcedureInfo>, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or_else(|| "连接不存在".to_string())?;
    driver
        .get_procedures(&database)
        .await
        .map_err(|e| format!("获取存储过程失败: {}", e))
}

#[tauri::command]
pub async fn get_triggers(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
) -> Result<Vec<TriggerInfo>, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or_else(|| "连接不存在".to_string())?;
    driver
        .get_triggers(&database)
        .await
        .map_err(|e| format!("获取触发器失败: {}", e))
}

#[tauri::command]
pub async fn get_foreign_keys(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
) -> Result<Vec<ForeignKeyInfo>, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or_else(|| "连接不存在".to_string())?;
    driver
        .get_foreign_keys(&database, &table)
        .await
        .map_err(|e| format!("获取外键失败: {}", e))
}

#[tauri::command]
pub async fn get_users(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Vec<UserInfo>, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or_else(|| "连接不存在".to_string())?;
    driver
        .get_users()
        .await
        .map_err(|e| format!("获取用户列表失败: {}", e))
}
