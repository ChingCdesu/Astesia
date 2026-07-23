use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::{Mutex, RwLock};

use crate::db::{ConnectionConfig, DatabaseDriver};
use crate::state::create_driver;

pub(super) type DriverHandle = Arc<Mutex<Box<dyn DatabaseDriver>>>;

#[derive(Clone)]
pub(super) struct ConnectionProfile {
    pub config: ConnectionConfig,
    pub password_env: Option<String>,
}

impl ConnectionProfile {
    pub fn resolved_config(&self) -> Result<ConnectionConfig, String> {
        let mut config = self.config.clone();
        config.password = match self.password_env.as_deref() {
            Some(variable) => std::env::var(variable)
                .map_err(|_| format!("环境变量 {variable} 未设置，无法读取数据库凭据"))?,
            None => String::new(),
        };
        Ok(config)
    }
}

#[derive(Clone, Serialize)]
pub(super) struct SavedQuery {
    pub id: String,
    pub name: String,
    pub connection_id: String,
    pub database: String,
    pub sql: String,
}

#[derive(Clone, Default)]
pub(super) struct Catalog {
    profiles: Arc<RwLock<HashMap<String, ConnectionProfile>>>,
    drivers: Arc<RwLock<HashMap<String, DriverHandle>>>,
    queries: Arc<RwLock<HashMap<String, SavedQuery>>>,
    update_approvals: Arc<RwLock<HashSet<(String, String)>>>,
}

impl Catalog {
    pub async fn insert_profile(&self, profile: ConnectionProfile) -> Result<(), String> {
        let id = profile.config.id.clone();
        let mut profiles = self.profiles.write().await;
        if profiles.contains_key(&id) {
            return Err(format!("连接 {id} 已存在"));
        }
        profiles.insert(id, profile);
        Ok(())
    }

    pub async fn profiles(&self) -> Vec<ConnectionProfile> {
        let mut profiles = self
            .profiles
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        profiles.sort_by(|a, b| a.config.name.cmp(&b.config.name));
        profiles
    }

    pub async fn profile(&self, connection_id: &str) -> Result<ConnectionProfile, String> {
        self.profiles
            .read()
            .await
            .get(connection_id)
            .cloned()
            .ok_or_else(|| format!("连接 {connection_id} 不存在"))
    }

    pub async fn is_connected(&self, connection_id: &str) -> bool {
        self.drivers.read().await.contains_key(connection_id)
    }

    pub async fn test_connection(&self, connection_id: &str) -> Result<(), String> {
        let profile = self.profile(connection_id).await?;
        let config = profile.resolved_config()?;
        create_driver(&config)
            .test_connection()
            .await
            .map(|_| ())
            .map_err(|error| format!("测试连接失败: {error}"))
    }

    pub async fn connect(&self, connection_id: &str) -> Result<bool, String> {
        if self.is_connected(connection_id).await {
            return Ok(false);
        }

        let profile = self.profile(connection_id).await?;
        let config = profile.resolved_config()?;
        let mut driver = create_driver(&config);
        driver
            .connect()
            .await
            .map_err(|error| format!("连接失败: {error}"))?;

        let mut drivers = self.drivers.write().await;
        if drivers.contains_key(connection_id) {
            drop(drivers);
            let _ = driver.disconnect().await;
            return Ok(false);
        }
        drivers.insert(connection_id.to_string(), Arc::new(Mutex::new(driver)));
        Ok(true)
    }

    pub async fn disconnect(&self, connection_id: &str) -> Result<bool, String> {
        let driver = self.drivers.write().await.remove(connection_id);
        self.update_approvals
            .write()
            .await
            .retain(|(approved_connection, _)| approved_connection != connection_id);
        match driver {
            Some(driver) => {
                driver
                    .lock()
                    .await
                    .disconnect()
                    .await
                    .map_err(|error| format!("断开连接失败: {error}"))?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub async fn remove_profile(&self, connection_id: &str) -> Result<(), String> {
        self.disconnect(connection_id).await?;
        self.profiles
            .write()
            .await
            .remove(connection_id)
            .ok_or_else(|| format!("连接 {connection_id} 不存在"))?;
        Ok(())
    }

    pub async fn driver(&self, connection_id: &str) -> Result<DriverHandle, String> {
        self.drivers
            .read()
            .await
            .get(connection_id)
            .cloned()
            .ok_or_else(|| format!("连接 {connection_id} 尚未访问，请先调用 connect_connection"))
    }

    pub async fn insert_query(&self, query: SavedQuery) -> Result<(), String> {
        let profiles = self.profiles.read().await;
        if !profiles.contains_key(&query.connection_id) {
            return Err(format!("连接 {} 不存在", query.connection_id));
        }
        let mut queries = self.queries.write().await;
        if queries.contains_key(&query.id) {
            return Err(format!("查询 {} 已存在", query.id));
        }
        queries.insert(query.id.clone(), query);
        Ok(())
    }

    pub async fn queries(&self) -> Vec<SavedQuery> {
        let mut queries = self
            .queries
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        queries.sort_by(|a, b| a.name.cmp(&b.name));
        queries
    }

    pub async fn query(&self, query_id: &str) -> Result<SavedQuery, String> {
        self.queries
            .read()
            .await
            .get(query_id)
            .cloned()
            .ok_or_else(|| format!("查询 {query_id} 不存在"))
    }

    pub async fn remove_query(&self, query_id: &str) -> Result<SavedQuery, String> {
        self.queries
            .write()
            .await
            .remove(query_id)
            .ok_or_else(|| format!("查询 {query_id} 不存在"))
    }

    pub async fn query_count_for_connection(&self, connection_id: &str) -> usize {
        self.queries
            .read()
            .await
            .values()
            .filter(|query| query.connection_id == connection_id)
            .count()
    }

    pub async fn remove_queries_for_connection(&self, connection_id: &str) -> usize {
        let mut queries = self.queries.write().await;
        let before = queries.len();
        queries.retain(|_, query| query.connection_id != connection_id);
        before - queries.len()
    }

    pub async fn updates_are_approved(&self, connection_id: &str, database: &str) -> bool {
        self.update_approvals
            .read()
            .await
            .contains(&(connection_id.to_string(), database.to_string()))
    }

    pub async fn approve_updates(&self, connection_id: &str, database: &str) {
        self.update_approvals
            .write()
            .await
            .insert((connection_id.to_string(), database.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn update_approval_is_scoped_to_connection_and_database() {
        let catalog = Catalog::default();
        catalog.approve_updates("connection-a", "database-a").await;

        assert!(
            catalog
                .updates_are_approved("connection-a", "database-a")
                .await
        );
        assert!(
            !catalog
                .updates_are_approved("connection-a", "database-b")
                .await
        );
        assert!(
            !catalog
                .updates_are_approved("connection-b", "database-a")
                .await
        );

        catalog
            .disconnect("connection-a")
            .await
            .expect("disconnect without a live driver");
        assert!(
            !catalog
                .updates_are_approved("connection-a", "database-a")
                .await
        );
    }
}
