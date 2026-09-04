pub mod clickhouse;
mod engine;
#[cfg(test)]
mod engine_smoke_tests;
pub mod mongo;
pub mod mysql;
pub mod postgres;
pub mod redis_db;
mod sql_render;
mod sql_script;
pub mod sqlite;
pub mod sqlserver;

pub use engine::{
    EngineCapabilities, EngineProfileSpec, EnumMode, ExplainMode, IndexMode, PerformanceMode,
    RowMutationMode, TableCopyMode,
};
pub use sql_render::TableRef;
pub(crate) use sql_render::{SqlDialect, SqlRenderError, SqlRenderResult};
pub(crate) use sql_script::SqlScript;

use std::{error::Error, fmt};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub(crate) fn bytes_to_hex(bytes: &[u8], prefix: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(prefix.len() + bytes.len() * 2);
    s.push_str(prefix);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

pub(crate) fn f64_to_json(v: f64) -> serde_json::Value {
    serde_json::Number::from_f64(v)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

// The shortest decimal representation avoids exposing f32-to-f64 noise in JSON.
pub(crate) fn f32_to_json(v: f32) -> serde_json::Value {
    v.to_string()
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum DbType {
    MySQL,
    PostgreSQL,
    SQLite,
    SQLServer,
    MongoDB,
    Redis,
    ClickHouse,
}

pub(crate) fn create_driver(config: &ConnectionConfig) -> Box<dyn DatabaseDriver> {
    match config.db_type {
        DbType::MySQL => Box::new(mysql::MySqlDriver::new(config.clone())),
        DbType::PostgreSQL => Box::new(postgres::PostgresDriver::new(config.clone())),
        DbType::SQLite => Box::new(sqlite::SqliteDriver::new(config.clone())),
        DbType::SQLServer => Box::new(sqlserver::SqlServerDriver::new(config.clone())),
        DbType::MongoDB => Box::new(mongo::MongoDriver::new(config.clone())),
        DbType::Redis => Box::new(redis_db::RedisDriver::new(config.clone())),
        DbType::ClickHouse => Box::new(clickhouse::ClickHouseDriver::new(config.clone())),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub affected_rows: u64,
    pub execution_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatementResult {
    pub sql: String,
    pub success: bool,
    pub error: Option<String>,
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub affected_rows: u64,
    pub execution_time_ms: u64,
}

impl StatementResult {
    pub fn from_query_result(sql: String, qr: QueryResult) -> Self {
        Self {
            sql,
            success: true,
            error: None,
            columns: qr.columns,
            rows: qr.rows,
            affected_rows: qr.affected_rows,
            execution_time_ms: qr.execution_time_ms,
        }
    }

    pub fn from_error(sql: String, err: impl ToString, elapsed_ms: u64) -> Self {
        Self {
            sql,
            success: false,
            error: Some(err.to_string()),
            columns: vec![],
            rows: vec![],
            affected_rows: 0,
            execution_time_ms: elapsed_ms,
        }
    }
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
    #[serde(flatten)]
    pub reference: TableRef,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstraintKind {
    PrimaryKey,
    Unique,
    Check,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintInfo {
    pub name: String,
    pub kind: ConstraintKind,
    pub columns: Vec<String>,
    pub definition: Option<String>,
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

#[derive(Debug, Clone, Serialize)]
pub struct ForeignKeyInfo {
    pub name: String,
    #[serde(serialize_with = "serialize_table_ref_name")]
    pub from_table: TableRef,
    pub from_columns: Vec<String>,
    #[serde(serialize_with = "serialize_table_ref_name")]
    pub to_table: TableRef,
    pub to_columns: Vec<String>,
}

fn serialize_table_ref_name<S>(table: &TableRef, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(table.name())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub name: String,
    pub host: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DocumentPage {
    pub(crate) documents: Vec<serde_json::Value>,
    pub(crate) total_documents: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RedisListSide {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RedisValue {
    Missing,
    String(String),
    Hash(Vec<(String, String)>),
    List(Vec<String>),
    Set(Vec<String>),
    SortedSet(Vec<(String, f64)>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RedisKeySnapshot {
    pub(crate) ttl_seconds: Option<u64>,
    pub(crate) value: RedisValue,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RedisMutation {
    SetString {
        value: String,
        ttl_seconds: Option<u64>,
    },
    HashSet {
        field: String,
        value: String,
    },
    HashDelete {
        field: String,
    },
    ListPush {
        side: RedisListSide,
        value: String,
    },
    ListRemove {
        count: i64,
        value: String,
    },
    SetAdd {
        member: String,
    },
    SetRemove {
        member: String,
    },
    SortedSetAdd {
        member: String,
        score: f64,
    },
    SortedSetRemove {
        member: String,
    },
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedFeature {
    pub engine: DbType,
    pub feature: &'static str,
}

impl UnsupportedFeature {
    pub const fn new(engine: DbType, feature: &'static str) -> Self {
        Self { engine, feature }
    }
}

impl fmt::Display for UnsupportedFeature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} is not supported for {:?}",
            self.feature, self.engine
        )
    }
}

impl Error for UnsupportedFeature {}

#[async_trait]
pub trait DatabaseDriver: Send + Sync {
    async fn connect(&mut self) -> anyhow::Result<()>;
    async fn disconnect(&mut self) -> anyhow::Result<()>;
    async fn test_connection(&self) -> anyhow::Result<bool>;
    async fn get_databases(&self) -> anyhow::Result<Vec<String>>;
    async fn get_tables(&self, database: &str) -> anyhow::Result<Vec<TableInfo>>;
    async fn get_columns(
        &self,
        database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<ColumnInfo>>;
    async fn get_indexes(
        &self,
        _database: &str,
        _table: &TableRef,
    ) -> anyhow::Result<Vec<IndexInfo>> {
        Err(UnsupportedFeature::new(self.db_type(), "indexes").into())
    }
    async fn get_constraints(
        &self,
        _database: &str,
        _table: &TableRef,
    ) -> anyhow::Result<Vec<ConstraintInfo>> {
        Err(UnsupportedFeature::new(self.db_type(), "constraints").into())
    }
    async fn execute_query(&self, database: &str, sql: &str) -> anyhow::Result<QueryResult>;
    /// Transactional drivers override this to keep the batch on one connection.
    async fn execute_statements(
        &self,
        database: &str,
        statements: Vec<String>,
    ) -> anyhow::Result<Vec<StatementResult>> {
        let mut results = Vec::with_capacity(statements.len());
        for sql in statements {
            let start = std::time::Instant::now();
            match self.execute_query(database, &sql).await {
                Ok(qr) => results.push(StatementResult::from_query_result(sql, qr)),
                Err(e) => {
                    let elapsed = start.elapsed().as_millis() as u64;
                    results.push(StatementResult::from_error(sql, e, elapsed));
                    break;
                }
            }
        }
        Ok(results)
    }
    async fn execute_mutation_batch(
        &self,
        _database: &str,
        _statements: Vec<String>,
    ) -> anyhow::Result<Vec<StatementResult>> {
        Err(UnsupportedFeature::new(self.db_type(), "transactional mutation batches").into())
    }
    async fn explain(&self, database: &str, statement: &str) -> anyhow::Result<QueryResult> {
        let sql = SqlDialect::new(self.db_type()).build_explain_statement(statement)?;
        self.execute_query(database, &sql).await
    }
    async fn get_table_data(
        &self,
        database: &str,
        table: &TableRef,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<QueryResult>;
    async fn set_key(
        &self,
        _database: &str,
        _key: &str,
        _value: &str,
        _ttl_seconds: Option<u64>,
    ) -> anyhow::Result<()> {
        Err(UnsupportedFeature::new(self.db_type(), "set key").into())
    }
    async fn delete_key(&self, _database: &str, _key: &str) -> anyhow::Result<u64> {
        Err(UnsupportedFeature::new(self.db_type(), "delete key").into())
    }
    async fn get_documents(
        &self,
        _database: &str,
        _collection: &TableRef,
        _filter: Option<serde_json::Value>,
        _page: u32,
        _page_size: u32,
    ) -> anyhow::Result<DocumentPage> {
        Err(UnsupportedFeature::new(self.db_type(), "document browsing").into())
    }
    async fn get_mongodb_server_status(
        &self,
        _database: &str,
    ) -> anyhow::Result<serde_json::Value> {
        Err(UnsupportedFeature::new(self.db_type(), "MongoDB server status").into())
    }
    async fn scan_redis_keys(
        &self,
        _database: &str,
        _pattern: &str,
    ) -> anyhow::Result<Vec<String>> {
        Err(UnsupportedFeature::new(self.db_type(), "Redis key scanning").into())
    }
    async fn get_redis_key(&self, _database: &str, _key: &str) -> anyhow::Result<RedisKeySnapshot> {
        Err(UnsupportedFeature::new(self.db_type(), "Redis key inspection").into())
    }
    async fn mutate_redis_key(
        &self,
        _database: &str,
        _key: &str,
        _mutation: RedisMutation,
    ) -> anyhow::Result<u64> {
        Err(UnsupportedFeature::new(self.db_type(), "Redis key mutation").into())
    }
    async fn execute_redis_command(
        &self,
        _database: &str,
        _arguments: Vec<String>,
    ) -> anyhow::Result<QueryResult> {
        Err(UnsupportedFeature::new(self.db_type(), "Redis commands").into())
    }
    fn db_type(&self) -> DbType;
    async fn get_views(&self, _database: &str) -> anyhow::Result<Vec<ViewInfo>> {
        Err(UnsupportedFeature::new(self.db_type(), "views").into())
    }
    async fn get_functions(&self, _database: &str) -> anyhow::Result<Vec<FunctionInfo>> {
        Err(UnsupportedFeature::new(self.db_type(), "functions").into())
    }
    async fn get_procedures(&self, _database: &str) -> anyhow::Result<Vec<ProcedureInfo>> {
        Err(UnsupportedFeature::new(self.db_type(), "procedures").into())
    }
    async fn get_triggers(&self, _database: &str) -> anyhow::Result<Vec<TriggerInfo>> {
        Err(UnsupportedFeature::new(self.db_type(), "triggers").into())
    }
    async fn get_foreign_keys(
        &self,
        _database: &str,
        _table: &TableRef,
    ) -> anyhow::Result<Vec<ForeignKeyInfo>> {
        Err(UnsupportedFeature::new(self.db_type(), "foreign keys").into())
    }
    async fn get_users(&self) -> anyhow::Result<Vec<UserInfo>> {
        Err(UnsupportedFeature::new(self.db_type(), "users").into())
    }

    async fn get_enum_values(
        &self,
        _database: &str,
        _enum_type: &str,
    ) -> anyhow::Result<Vec<String>> {
        Err(UnsupportedFeature::new(self.db_type(), "enum values").into())
    }

    async fn get_schemas(&self, _database: &str) -> anyhow::Result<Vec<String>> {
        Err(UnsupportedFeature::new(self.db_type(), "schemas").into())
    }

    async fn get_create_table_sql(
        &self,
        _database: &str,
        _table: &TableRef,
    ) -> anyhow::Result<String> {
        Err(UnsupportedFeature::new(self.db_type(), "create table SQL").into())
    }
}

#[cfg(test)]
mod table_identity_tests {
    use serde_json::json;

    use super::{ForeignKeyInfo, TableInfo, TableRef};

    #[test]
    fn table_and_foreign_key_wire_shapes_remain_compatible() {
        let table = TableRef::qualified("billing.v2", "account.history");
        let table_info = TableInfo {
            reference: table.clone(),
            row_count: None,
            comment: None,
        };
        assert_eq!(
            serde_json::to_value(table_info).unwrap(),
            json!({
                "name": "account.history",
                "schema": "billing.v2",
                "row_count": null,
                "comment": null,
            })
        );

        let foreign_key = ForeignKeyInfo {
            name: "fk_account".to_string(),
            from_table: table,
            from_columns: vec!["parent_id".to_string()],
            to_table: TableRef::qualified("billing.v2", "parent.table"),
            to_columns: vec!["id".to_string()],
        };
        let serialized = serde_json::to_value(foreign_key).unwrap();
        assert_eq!(serialized["from_table"], "account.history");
        assert_eq!(serialized["to_table"], "parent.table");
    }
}
