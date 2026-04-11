use serde_json::Value;
use tauri::State;
use crate::state::AppState;

#[tauri::command]
pub async fn update_row(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
    primary_key_column: String,
    primary_key_value: Value,
    column: String,
    new_value: Value,
) -> Result<u64, String> {
    let connections = state.connections.lock().await;
    let driver = connections.get(&connection_id).ok_or("连接不存在")?;

    let pk_val = value_to_sql(&primary_key_value);
    let new_val = value_to_sql(&new_value);
    let sql = format!(
        "UPDATE `{}` SET `{}` = {} WHERE `{}` = {}",
        table, column, new_val, primary_key_column, pk_val
    );
    let result = driver.execute_query(&database, &sql).await.map_err(|e| e.to_string())?;
    Ok(result.affected_rows)
}

#[tauri::command]
pub async fn delete_rows(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
    primary_key_column: String,
    primary_key_values: Vec<Value>,
) -> Result<u64, String> {
    let connections = state.connections.lock().await;
    let driver = connections.get(&connection_id).ok_or("连接不存在")?;

    let vals: Vec<String> = primary_key_values.iter().map(value_to_sql).collect();
    let sql = format!(
        "DELETE FROM `{}` WHERE `{}` IN ({})",
        table, primary_key_column, vals.join(", ")
    );
    let result = driver.execute_query(&database, &sql).await.map_err(|e| e.to_string())?;
    Ok(result.affected_rows)
}

#[tauri::command]
pub async fn insert_row(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
    columns: Vec<String>,
    values: Vec<Value>,
) -> Result<u64, String> {
    let connections = state.connections.lock().await;
    let driver = connections.get(&connection_id).ok_or("连接不存在")?;

    let cols = columns.iter().map(|c| format!("`{}`", c)).collect::<Vec<_>>().join(", ");
    let vals = values.iter().map(value_to_sql).collect::<Vec<_>>().join(", ");
    let sql = format!("INSERT INTO `{}` ({}) VALUES ({})", table, cols, vals);
    let result = driver.execute_query(&database, &sql).await.map_err(|e| e.to_string())?;
    Ok(result.affected_rows)
}

fn value_to_sql(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        _ => format!("'{}'", value.to_string().replace('\'', "''")),
    }
}
