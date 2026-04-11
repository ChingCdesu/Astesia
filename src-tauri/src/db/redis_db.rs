use async_trait::async_trait;
use redis::AsyncCommands;
use std::time::Instant;

use super::{ColumnInfo, ConnectionConfig, DatabaseDriver, DbType, IndexInfo, QueryResult, TableInfo};

pub struct RedisDriver {
    config: ConnectionConfig,
    client: Option<redis::Client>,
    connection: Option<redis::aio::MultiplexedConnection>,
}

impl RedisDriver {
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            client: None,
            connection: None,
        }
    }

    fn connection_string(&self) -> String {
        if self.config.password.is_empty() {
            format!("redis://{}:{}", self.config.host, self.config.port)
        } else {
            format!(
                "redis://:{}@{}:{}",
                self.config.password, self.config.host, self.config.port
            )
        }
    }

    fn conn(&self) -> anyhow::Result<&redis::aio::MultiplexedConnection> {
        self.connection
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))
    }
}

#[async_trait]
impl DatabaseDriver for RedisDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let client = redis::Client::open(self.connection_string())?;
        let connection = client.get_multiplexed_async_connection().await?;
        self.client = Some(client);
        self.connection = Some(connection);
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        self.connection = None;
        self.client = None;
        Ok(())
    }

    async fn test_connection(&self) -> anyhow::Result<bool> {
        let client = redis::Client::open(self.connection_string())?;
        let mut conn = client.get_multiplexed_async_connection().await?;
        let pong: String = redis::cmd("PING").query_async(&mut conn).await?;
        Ok(pong == "PONG")
    }

    async fn get_databases(&self) -> anyhow::Result<Vec<String>> {
        // Redis has 16 databases by default (0-15)
        Ok((0..16).map(|i| format!("db{}", i)).collect())
    }

    async fn get_tables(&self, database: &str) -> anyhow::Result<Vec<TableInfo>> {
        let mut conn = self.conn()?.clone();
        // Switch to the specified database
        let db_num: u8 = database
            .strip_prefix("db")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let _: () = redis::cmd("SELECT")
            .arg(db_num)
            .query_async(&mut conn)
            .await?;
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg("*")
            .query_async(&mut conn)
            .await?;
        Ok(keys
            .into_iter()
            .map(|name| TableInfo {
                name,
                schema: None,
                row_count: None,
                comment: Some("key".to_string()),
            })
            .collect())
    }

    async fn get_columns(&self, _database: &str, _table: &str) -> anyhow::Result<Vec<ColumnInfo>> {
        Ok(vec![
            ColumnInfo {
                name: "key".to_string(),
                data_type: "String".to_string(),
                nullable: false,
                is_primary_key: true,
                default_value: None,
                comment: None,
            },
            ColumnInfo {
                name: "value".to_string(),
                data_type: "String".to_string(),
                nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
            },
            ColumnInfo {
                name: "type".to_string(),
                data_type: "String".to_string(),
                nullable: false,
                is_primary_key: false,
                default_value: None,
                comment: None,
            },
            ColumnInfo {
                name: "ttl".to_string(),
                data_type: "Integer".to_string(),
                nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
            },
        ])
    }

    async fn get_indexes(&self, _database: &str, _table: &str) -> anyhow::Result<Vec<IndexInfo>> {
        Ok(vec![])
    }

    async fn execute_query(&self, database: &str, command: &str) -> anyhow::Result<QueryResult> {
        let mut conn = self.conn()?.clone();
        let db_num: u8 = database
            .strip_prefix("db")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let _: () = redis::cmd("SELECT")
            .arg(db_num)
            .query_async(&mut conn)
            .await?;

        let start = Instant::now();
        let parts: Vec<&str> = command.trim().split_whitespace().collect();
        if parts.is_empty() {
            return Ok(QueryResult::default());
        }

        let mut cmd = redis::cmd(parts[0]);
        for part in &parts[1..] {
            cmd.arg(*part);
        }
        let result: redis::Value = cmd.query_async(&mut conn).await?;
        let elapsed = start.elapsed().as_millis() as u64;

        let columns = vec![ColumnInfo {
            name: "result".to_string(),
            data_type: "String".to_string(),
            nullable: true,
            is_primary_key: false,
            default_value: None,
            comment: None,
        }];

        let rows = vec![vec![redis_value_to_json(&result)]];

        Ok(QueryResult {
            columns,
            rows,
            affected_rows: 0,
            execution_time_ms: elapsed,
        })
    }

    async fn get_table_data(
        &self,
        database: &str,
        key: &str,
        _page: u32,
        _page_size: u32,
    ) -> anyhow::Result<QueryResult> {
        let mut conn = self.conn()?.clone();
        let db_num: u8 = database
            .strip_prefix("db")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let _: () = redis::cmd("SELECT")
            .arg(db_num)
            .query_async(&mut conn)
            .await?;

        let key_type: String = redis::cmd("TYPE")
            .arg(key)
            .query_async(&mut conn)
            .await?;
        let ttl: i64 = conn.ttl(key).await?;

        let start = Instant::now();
        let value = match key_type.as_str() {
            "string" => {
                let val: String = conn.get(key).await?;
                serde_json::Value::String(val)
            }
            "list" => {
                let vals: Vec<String> = conn.lrange(key, 0, -1).await?;
                serde_json::Value::Array(vals.into_iter().map(serde_json::Value::String).collect())
            }
            "set" => {
                let vals: Vec<String> = conn.smembers(key).await?;
                serde_json::Value::Array(vals.into_iter().map(serde_json::Value::String).collect())
            }
            "hash" => {
                let vals: Vec<(String, String)> = conn.hgetall(key).await?;
                let map: serde_json::Map<String, serde_json::Value> = vals
                    .into_iter()
                    .map(|(k, v)| (k, serde_json::Value::String(v)))
                    .collect();
                serde_json::Value::Object(map)
            }
            "zset" => {
                let vals: Vec<(String, f64)> = conn.zrangebyscore_withscores(key, "-inf", "+inf").await?;
                serde_json::Value::Array(
                    vals.into_iter()
                        .map(|(member, score)| {
                            serde_json::json!({"member": member, "score": score})
                        })
                        .collect(),
                )
            }
            _ => serde_json::Value::String(format!("Unsupported type: {}", key_type)),
        };
        let elapsed = start.elapsed().as_millis() as u64;

        let columns = vec![
            ColumnInfo { name: "key".to_string(), data_type: "String".to_string(), nullable: false, is_primary_key: true, default_value: None, comment: None },
            ColumnInfo { name: "value".to_string(), data_type: key_type.clone(), nullable: true, is_primary_key: false, default_value: None, comment: None },
            ColumnInfo { name: "type".to_string(), data_type: "String".to_string(), nullable: false, is_primary_key: false, default_value: None, comment: None },
            ColumnInfo { name: "ttl".to_string(), data_type: "Integer".to_string(), nullable: true, is_primary_key: false, default_value: None, comment: None },
        ];

        let rows = vec![vec![
            serde_json::Value::String(key.to_string()),
            value,
            serde_json::Value::String(key_type),
            serde_json::Value::Number(ttl.into()),
        ]];

        Ok(QueryResult {
            columns,
            rows,
            affected_rows: 0,
            execution_time_ms: elapsed,
        })
    }

    fn db_type(&self) -> DbType {
        DbType::Redis
    }
}

fn redis_value_to_json(value: &redis::Value) -> serde_json::Value {
    match value {
        redis::Value::Nil => serde_json::Value::Null,
        redis::Value::Int(v) => serde_json::Value::Number((*v).into()),
        redis::Value::BulkString(v) => {
            serde_json::Value::String(String::from_utf8_lossy(v).to_string())
        }
        redis::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(redis_value_to_json).collect())
        }
        redis::Value::SimpleString(s) => serde_json::Value::String(s.clone()),
        redis::Value::Okay => serde_json::Value::String("OK".to_string()),
        _ => serde_json::Value::String(format!("{:?}", value)),
    }
}
