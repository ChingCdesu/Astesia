pub mod mysql;
pub mod postgres;
pub mod sqlite;
pub mod sqlserver;
pub mod mongo;
pub mod redis_db;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub db_type: DbType,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DbType {
    MySQL,
    PostgreSQL,
    SQLite,
    SQLServer,
    MongoDB,
    Redis,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub affected_rows: u64,
    pub execution_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub is_primary_key: bool,
    pub default_value: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    pub schema: Option<String>,
    pub row_count: Option<i64>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewInfo {
    pub name: String,
    pub definition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub name: String,
    pub language: Option<String>,
    pub return_type: Option<String>,
    pub definition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureInfo {
    pub name: String,
    pub language: Option<String>,
    pub definition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerInfo {
    pub name: String,
    pub event: String,
    pub table: String,
    pub timing: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyInfo {
    pub name: String,
    pub from_table: String,
    pub from_columns: Vec<String>,
    pub to_table: String,
    pub to_columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub name: String,
    pub host: Option<String>,
}

#[async_trait]
pub trait DatabaseDriver: Send + Sync {
    async fn connect(&mut self) -> anyhow::Result<()>;
    async fn disconnect(&mut self) -> anyhow::Result<()>;
    async fn test_connection(&self) -> anyhow::Result<bool>;
    async fn get_databases(&self) -> anyhow::Result<Vec<String>>;
    async fn get_tables(&self, database: &str) -> anyhow::Result<Vec<TableInfo>>;
    async fn get_columns(&self, database: &str, table: &str) -> anyhow::Result<Vec<ColumnInfo>>;
    async fn get_indexes(&self, database: &str, table: &str) -> anyhow::Result<Vec<IndexInfo>>;
    async fn execute_query(&self, database: &str, sql: &str) -> anyhow::Result<QueryResult>;
    async fn get_table_data(
        &self,
        database: &str,
        table: &str,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<QueryResult>;
    fn db_type(&self) -> DbType;

    async fn get_views(&self, _database: &str) -> anyhow::Result<Vec<ViewInfo>> { Ok(vec![]) }
    async fn get_functions(&self, _database: &str) -> anyhow::Result<Vec<FunctionInfo>> { Ok(vec![]) }
    async fn get_procedures(&self, _database: &str) -> anyhow::Result<Vec<ProcedureInfo>> { Ok(vec![]) }
    async fn get_triggers(&self, _database: &str) -> anyhow::Result<Vec<TriggerInfo>> { Ok(vec![]) }
    async fn get_foreign_keys(&self, _database: &str, _table: &str) -> anyhow::Result<Vec<ForeignKeyInfo>> { Ok(vec![]) }
    async fn get_users(&self) -> anyhow::Result<Vec<UserInfo>> { Ok(vec![]) }

    async fn get_enum_values(&self, _database: &str, _enum_type: &str) -> anyhow::Result<Vec<String>> { Ok(vec![]) }

    async fn get_create_table_sql(&self, _database: &str, _table: &str) -> anyhow::Result<String> {
        Err(anyhow::anyhow!("Not supported for this database type"))
    }
}

impl Default for QueryResult {
    fn default() -> Self {
        Self {
            columns: vec![],
            rows: vec![],
            affected_rows: 0,
            execution_time_ms: 0,
        }
    }
}
