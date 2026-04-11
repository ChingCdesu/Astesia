use tauri::State;

use crate::db::QueryResult;
use crate::state::AppState;

#[tauri::command]
pub async fn execute_query(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    sql: String,
) -> Result<QueryResult, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or_else(|| "连接不存在".to_string())?;
    driver
        .execute_query(&database, &sql)
        .await
        .map_err(|e| format!("查询失败: {}", e))
}

#[tauri::command]
pub async fn get_table_data(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
    page: u32,
    page_size: u32,
) -> Result<QueryResult, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or_else(|| "连接不存在".to_string())?;
    driver
        .get_table_data(&database, &table, page, page_size)
        .await
        .map_err(|e| format!("获取数据失败: {}", e))
}
