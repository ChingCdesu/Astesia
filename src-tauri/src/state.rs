use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::db::{
    mongo::MongoDriver, mysql::MySqlDriver, postgres::PostgresDriver, redis_db::RedisDriver,
    sqlite::SqliteDriver, sqlserver::SqlServerDriver, ConnectionConfig, DatabaseDriver, DbType,
};
use crate::tasks::TaskManager;

pub struct AppState {
    pub connections: Arc<Mutex<HashMap<String, Box<dyn DatabaseDriver>>>>,
    pub task_manager: TaskManager,
    pub app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
            task_manager: TaskManager::new(),
            app_handle: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_app_handle(&self, handle: tauri::AppHandle) {
        let mut app_handle = self.app_handle.lock().await;
        *app_handle = Some(handle);
    }
}

pub fn create_driver(config: &ConnectionConfig) -> Box<dyn DatabaseDriver> {
    match config.db_type {
        DbType::MySQL => Box::new(MySqlDriver::new(config.clone())),
        DbType::PostgreSQL => Box::new(PostgresDriver::new(config.clone())),
        DbType::SQLite => Box::new(SqliteDriver::new(config.clone())),
        DbType::SQLServer => Box::new(SqlServerDriver::new(config.clone())),
        DbType::MongoDB => Box::new(MongoDriver::new(config.clone())),
        DbType::Redis => Box::new(RedisDriver::new(config.clone())),
    }
}
