use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::connection_repository::{ConnectionRepositoryError, SharedConnectionRepository};
use crate::db::{
    mongo::MongoDriver, mysql::MySqlDriver, postgres::PostgresDriver, redis_db::RedisDriver,
    sqlite::SqliteDriver, sqlserver::SqlServerDriver, ConnectionConfig, DatabaseDriver, DbType,
};
use crate::tasks::TaskManager;

pub struct AppState {
    pub connections: Arc<Mutex<HashMap<String, Box<dyn DatabaseDriver>>>>,
    /// Revisions for drivers opened from the shared connection repository.
    ///
    /// HTTP-mirrored MCP drivers deliberately have no entry here, so repository
    /// reconciliation cannot remove session-scoped HTTP connections.
    pub shared_driver_revisions: Arc<Mutex<HashMap<String, i64>>>,
    /// Serializes shared-driver install/removal with repository snapshots and
    /// local profile mutations. Code taking the driver maps must always acquire
    /// this coordinator first, then `connections`, then
    /// `shared_driver_revisions`.
    pub shared_driver_coordinator: Arc<Mutex<()>>,
    pub connection_repository: SharedConnectionRepository,
    pub task_manager: TaskManager,
    pub app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
}

impl AppState {
    pub fn new() -> Result<Self, ConnectionRepositoryError> {
        Ok(Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
            shared_driver_revisions: Arc::new(Mutex::new(HashMap::new())),
            shared_driver_coordinator: Arc::new(Mutex::new(())),
            connection_repository: SharedConnectionRepository::new_default()?,
            task_manager: TaskManager::new(),
            app_handle: Arc::new(Mutex::new(None)),
        })
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
