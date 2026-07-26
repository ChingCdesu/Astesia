use serde_json::Value;
use tauri::State;
use crate::db::DbType;
use crate::state::AppState;

/// Quote an identifier (column name, etc.) using the appropriate syntax for the database type.
fn quote_identifier(name: &str, db_type: &DbType) -> String {
    match db_type {
        DbType::MySQL => format!("`{}`", name.replace('`', "``")),
        DbType::ClickHouse => {
            let escaped = name.replace('\\', "\\\\").replace('`', "\\`");
            format!("`{escaped}`")
        }
        DbType::PostgreSQL => format!("\"{}\"", name),
        DbType::SQLite => format!("\"{}\"", name),
        DbType::SQLServer => format!("[{}]", name),
        _ => name.to_string(),
    }
}

/// Quote a table reference using the appropriate syntax for the database type.
/// For PostgreSQL, handles the "schema.table" convention.
fn quote_table(table: &str, db_type: &DbType) -> String {
    match db_type {
        DbType::PostgreSQL => {
            if let Some(dot) = table.find('.') {
                let schema = &table[..dot];
                let tbl = &table[dot + 1..];
                format!("\"{}\".\"{}\"", schema, tbl)
            } else {
                format!("\"{}\"", table)
            }
        }
        DbType::MySQL => format!("`{}`", table.replace('`', "``")),
        DbType::ClickHouse => {
            let escaped = table.replace('\\', "\\\\").replace('`', "\\`");
            format!("`{escaped}`")
        }
        DbType::SQLServer => format!("[{}]", table),
        DbType::SQLite => format!("\"{}\"", table),
        _ => format!("\"{}\"", table),
    }
}

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
    let db_type = driver.db_type();

    let pk_val = value_to_sql(&primary_key_value, &db_type);
    let new_val = value_to_sql(&new_value, &db_type);
    let tbl = quote_table(&table, &db_type);
    let col = quote_identifier(&column, &db_type);
    let pk_col = quote_identifier(&primary_key_column, &db_type);
    let sql = if db_type == DbType::ClickHouse {
        format!(
            "ALTER TABLE {} UPDATE {} = {} WHERE {} = {} SETTINGS mutations_sync = 1",
            tbl, col, new_val, pk_col, pk_val
        )
    } else {
        format!(
            "UPDATE {} SET {} = {} WHERE {} = {}",
            tbl, col, new_val, pk_col, pk_val
        )
    };
    log::info!("Executing UPDATE SQL: {}", sql);
    let result = driver.execute_query(&database, &sql).await.map_err(|e| {
        log::error!("UPDATE failed: {}", e);
        e.to_string()
    })?;
    log::info!("UPDATE affected {} rows", result.affected_rows);
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
    let db_type = driver.db_type();

    let vals: Vec<String> = primary_key_values
        .iter()
        .map(|value| value_to_sql(value, &db_type))
        .collect();
    let tbl = quote_table(&table, &db_type);
    let pk_col = quote_identifier(&primary_key_column, &db_type);
    let sql = if db_type == DbType::ClickHouse {
        format!(
            "ALTER TABLE {} DELETE WHERE {} IN ({}) SETTINGS mutations_sync = 1",
            tbl,
            pk_col,
            vals.join(", ")
        )
    } else {
        format!(
            "DELETE FROM {} WHERE {} IN ({})",
            tbl,
            pk_col,
            vals.join(", ")
        )
    };
    log::info!("Executing DELETE SQL: {}", sql);
    let result = driver.execute_query(&database, &sql).await.map_err(|e| {
        log::error!("DELETE failed: {}", e);
        e.to_string()
    })?;
    log::info!("DELETE affected {} rows", result.affected_rows);
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
    let db_type = driver.db_type();

    let cols = columns.iter().map(|c| quote_identifier(c, &db_type)).collect::<Vec<_>>().join(", ");
    let vals = values
        .iter()
        .map(|value| value_to_sql(value, &db_type))
        .collect::<Vec<_>>()
        .join(", ");
    let tbl = quote_table(&table, &db_type);
    let sql = format!("INSERT INTO {} ({}) VALUES ({})", tbl, cols, vals);
    log::info!("Executing INSERT SQL: {}", sql);
    let result = driver.execute_query(&database, &sql).await.map_err(|e| {
        log::error!("INSERT failed: {}", e);
        e.to_string()
    })?;
    log::info!("INSERT affected {} rows", result.affected_rows);
    Ok(result.affected_rows)
}

fn value_to_sql(value: &Value, db_type: &DbType) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => quote_string_value(s, db_type),
        _ => quote_string_value(&value.to_string(), db_type),
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

#[tauri::command]
pub async fn redis_set_key(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    key: String,
    value: String,
    ttl: Option<i64>,
) -> Result<String, String> {
    let connections = state.connections.lock().await;
    let driver = connections.get(&connection_id).ok_or("连接不存在")?;

    let mut cmd = format!("SET {} {}", key, value);
    if let Some(t) = ttl {
        if t > 0 { cmd = format!("SET {} {} EX {}", key, value, t); }
    }
    driver.execute_query(&database, &cmd).await.map_err(|e| e.to_string())?;
    Ok("OK".to_string())
}

#[tauri::command]
pub async fn redis_delete_key(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    key: String,
) -> Result<u64, String> {
    let connections = state.connections.lock().await;
    let driver = connections.get(&connection_id).ok_or("连接不存在")?;
    let cmd = format!("DEL {}", key);
    let result = driver.execute_query(&database, &cmd).await.map_err(|e| e.to_string())?;
    Ok(result.affected_rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_clickhouse_string_and_identifier_literals() {
        assert_eq!(
            value_to_sql(&Value::String("it's\\ready".to_string()), &DbType::ClickHouse),
            "'it\\'s\\\\ready'"
        );
        assert_eq!(quote_identifier(r"odd\`name", &DbType::ClickHouse), r"`odd\\\`name`");
    }
}
