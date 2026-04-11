use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::{ConnectionConfig, DbType};
use crate::state::{create_driver, AppState};

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionResult {
    pub success: bool,
    pub message: String,
}

#[tauri::command]
pub async fn test_connection(config: ConnectionConfig) -> Result<ConnectionResult, String> {
    let driver = create_driver(&config);
    match driver.test_connection().await {
        Ok(_) => Ok(ConnectionResult {
            success: true,
            message: "连接成功".to_string(),
        }),
        Err(e) => Ok(ConnectionResult {
            success: false,
            message: format!("连接失败: {}", e),
        }),
    }
}

#[tauri::command]
pub async fn connect_database(
    state: State<'_, AppState>,
    config: ConnectionConfig,
) -> Result<ConnectionResult, String> {
    let mut driver = create_driver(&config);
    match driver.connect().await {
        Ok(_) => {
            let mut connections = state.connections.lock().await;
            connections.insert(config.id.clone(), driver);
            Ok(ConnectionResult {
                success: true,
                message: "连接成功".to_string(),
            })
        }
        Err(e) => Ok(ConnectionResult {
            success: false,
            message: format!("连接失败: {}", e),
        }),
    }
}

#[tauri::command]
pub async fn disconnect_database(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<ConnectionResult, String> {
    let mut connections = state.connections.lock().await;
    if let Some(mut driver) = connections.remove(&connection_id) {
        let _ = driver.disconnect().await;
        Ok(ConnectionResult {
            success: true,
            message: "已断开连接".to_string(),
        })
    } else {
        Ok(ConnectionResult {
            success: false,
            message: "连接不存在".to_string(),
        })
    }
}

#[tauri::command]
pub async fn get_default_port(db_type: DbType) -> Result<u16, String> {
    let port = match db_type {
        DbType::MySQL => 3306,
        DbType::PostgreSQL => 5432,
        DbType::SQLite => 0,
        DbType::SQLServer => 1433,
        DbType::MongoDB => 27017,
        DbType::Redis => 6379,
    };
    Ok(port)
}
