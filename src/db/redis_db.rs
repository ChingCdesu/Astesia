use async_trait::async_trait;
use redis::{AsyncCommands, ConnectionAddr, ConnectionInfo, RedisConnectionInfo};
use std::collections::HashSet;
use std::time::Instant;

use super::{
    ColumnInfo, ConnectionConfig, DatabaseDriver, DbType, QueryResult, RedisKeySnapshot,
    RedisListSide, RedisMutation, RedisPageCursor, RedisValue, TableInfo, TableRef,
};

const SCAN_BATCH_SIZE: usize = 500;
pub(crate) const VALUE_PAGE_SIZE: u64 = 256;

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

    /// Build typed connection info so a password with special characters
    /// (`/ # ? @ :`, spaces, …) is handled by the driver instead of being
    /// string-interpolated into a URL, which previously mis-parsed and produced
    /// errors like "invalid port number".
    fn connection_info(&self) -> ConnectionInfo {
        ConnectionInfo {
            addr: ConnectionAddr::Tcp(self.config.host.clone(), self.config.port),
            redis: RedisConnectionInfo {
                password: if self.config.password.is_empty() {
                    None
                } else {
                    Some(self.config.password.clone())
                },
                ..Default::default()
            },
        }
    }

    fn conn(&self) -> anyhow::Result<&redis::aio::MultiplexedConnection> {
        self.connection
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))
    }

    async fn selected_connection(
        &self,
        database: &str,
    ) -> anyhow::Result<redis::aio::MultiplexedConnection> {
        let mut connection = self.conn()?.clone();
        select_database_command(database)?
            .query_async::<()>(&mut connection)
            .await?;
        Ok(connection)
    }
}

#[async_trait]
impl DatabaseDriver for RedisDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let client = redis::Client::open(self.connection_info())?;
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
        let client = redis::Client::open(self.connection_info())?;
        let mut conn = client.get_multiplexed_async_connection().await?;
        let pong: String = redis::cmd("PING").query_async(&mut conn).await?;
        Ok(pong == "PONG")
    }

    async fn get_databases(&self) -> anyhow::Result<Vec<String>> {
        // Redis has 16 databases by default (0-15)
        Ok((0..16).map(|i| format!("db{}", i)).collect())
    }

    async fn get_tables(&self, database: &str) -> anyhow::Result<Vec<TableInfo>> {
        let mut conn = self.selected_connection(database).await?;
        let keys = scan_keys(&mut conn, "*").await?;
        Ok(keys
            .into_iter()
            .map(|name| TableInfo {
                reference: TableRef::unqualified(name),
                row_count: None,
                comment: Some("key".to_string()),
            })
            .collect())
    }

    async fn get_columns(
        &self,
        _database: &str,
        _table: &TableRef,
    ) -> anyhow::Result<Vec<ColumnInfo>> {
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

    async fn execute_query(&self, database: &str, command: &str) -> anyhow::Result<QueryResult> {
        let mut conn = self.selected_connection(database).await?;

        let start = Instant::now();
        let parts: Vec<&str> = command.split_whitespace().collect();
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

    async fn set_key(
        &self,
        database: &str,
        key: &str,
        value: &str,
        ttl_seconds: Option<u64>,
    ) -> anyhow::Result<()> {
        let mut connection = self.selected_connection(database).await?;
        set_key_command(key, value, ttl_seconds)
            .query_async::<()>(&mut connection)
            .await?;
        Ok(())
    }

    async fn delete_key(&self, database: &str, key: &str) -> anyhow::Result<u64> {
        let mut connection = self.selected_connection(database).await?;
        let deleted = delete_key_command(key)
            .query_async::<u64>(&mut connection)
            .await?;
        Ok(deleted)
    }

    async fn scan_redis_keys(&self, database: &str, pattern: &str) -> anyhow::Result<Vec<String>> {
        let mut conn = self.selected_connection(database).await?;
        scan_keys(&mut conn, pattern).await
    }

    async fn get_redis_key_page(
        &self,
        database: &str,
        key: &str,
        cursor: Option<RedisPageCursor>,
    ) -> anyhow::Result<RedisKeySnapshot> {
        let mut connection = self.selected_connection(database).await?;
        let key_type: String = redis::cmd("TYPE")
            .arg(key)
            .query_async(&mut connection)
            .await?;
        if key_type == "none" {
            return Ok(RedisKeySnapshot {
                ttl_seconds: None,
                value: RedisValue::Missing,
                total_entries: 0,
                offset: 0,
                next_page: None,
            });
        }
        let position = page_position(&key_type, cursor)?;
        let ttl: i64 = redis::cmd("TTL")
            .arg(key)
            .query_async(&mut connection)
            .await?;
        let ttl_seconds = u64::try_from(ttl).ok();
        let mut offset = 0;
        let (value, total_entries, next_page) = match key_type.as_str() {
            "string" => (
                RedisValue::String(
                    redis::cmd("GET")
                        .arg(key)
                        .query_async(&mut connection)
                        .await?,
                ),
                1,
                None,
            ),
            "hash" => {
                let total = redis::cmd("HLEN")
                    .arg(key)
                    .query_async(&mut connection)
                    .await?;
                // COUNT is a work hint; retaining every returned entry preserves scan coverage.
                let (next, mut values): (u64, Vec<(String, String)>) = redis::cmd("HSCAN")
                    .arg(key)
                    .arg(position)
                    .arg("COUNT")
                    .arg(VALUE_PAGE_SIZE)
                    .query_async(&mut connection)
                    .await?;
                values.sort_unstable_by(|left, right| left.0.cmp(&right.0));
                values.dedup_by(|left, right| left.0 == right.0);
                (
                    RedisValue::Hash(values),
                    total,
                    (next != 0).then_some(RedisPageCursor::Hash(next)),
                )
            }
            "list" => {
                offset = position;
                let total = redis::cmd("LLEN")
                    .arg(key)
                    .query_async(&mut connection)
                    .await?;
                let values = redis::cmd("LRANGE")
                    .arg(key)
                    .arg(position)
                    .arg(range_end(position)?)
                    .query_async(&mut connection)
                    .await?;
                (
                    RedisValue::List(values),
                    total,
                    next_range_page(position, total).map(RedisPageCursor::List),
                )
            }
            "set" => {
                let total = redis::cmd("SCARD")
                    .arg(key)
                    .query_async(&mut connection)
                    .await?;
                let (next, mut values): (u64, Vec<String>) = redis::cmd("SSCAN")
                    .arg(key)
                    .arg(position)
                    .arg("COUNT")
                    .arg(VALUE_PAGE_SIZE)
                    .query_async(&mut connection)
                    .await?;
                values.sort_unstable();
                values.dedup();
                (
                    RedisValue::Set(values),
                    total,
                    (next != 0).then_some(RedisPageCursor::Set(next)),
                )
            }
            "zset" => {
                offset = position;
                let total = redis::cmd("ZCARD")
                    .arg(key)
                    .query_async(&mut connection)
                    .await?;
                let values = redis::cmd("ZRANGE")
                    .arg(key)
                    .arg(position)
                    .arg(range_end(position)?)
                    .arg("WITHSCORES")
                    .query_async(&mut connection)
                    .await?;
                (
                    RedisValue::SortedSet(values),
                    total,
                    next_range_page(position, total).map(RedisPageCursor::SortedSet),
                )
            }
            other => anyhow::bail!("Unsupported Redis key type {other}"),
        };
        Ok(RedisKeySnapshot {
            ttl_seconds,
            value,
            total_entries,
            offset,
            next_page,
        })
    }

    async fn mutate_redis_key(
        &self,
        database: &str,
        key: &str,
        mutation: RedisMutation,
    ) -> anyhow::Result<u64> {
        let mut connection = self.selected_connection(database).await?;
        match mutation {
            RedisMutation::SetString { value, ttl_seconds } => {
                set_key_command(key, &value, ttl_seconds)
                    .query_async::<()>(&mut connection)
                    .await?;
                Ok(1)
            }
            RedisMutation::HashSet { field, value } => redis::cmd("HSET")
                .arg(key)
                .arg(field)
                .arg(value)
                .query_async(&mut connection)
                .await
                .map_err(Into::into),
            RedisMutation::HashDelete { field } => redis::cmd("HDEL")
                .arg(key)
                .arg(field)
                .query_async(&mut connection)
                .await
                .map_err(Into::into),
            RedisMutation::ListPush { side, value } => redis::cmd(match side {
                RedisListSide::Left => "LPUSH",
                RedisListSide::Right => "RPUSH",
            })
            .arg(key)
            .arg(value)
            .query_async(&mut connection)
            .await
            .map_err(Into::into),
            RedisMutation::ListRemove { count, value } => redis::cmd("LREM")
                .arg(key)
                .arg(count)
                .arg(value)
                .query_async(&mut connection)
                .await
                .map_err(Into::into),
            RedisMutation::SetAdd { member } => redis::cmd("SADD")
                .arg(key)
                .arg(member)
                .query_async(&mut connection)
                .await
                .map_err(Into::into),
            RedisMutation::SetRemove { member } => redis::cmd("SREM")
                .arg(key)
                .arg(member)
                .query_async(&mut connection)
                .await
                .map_err(Into::into),
            RedisMutation::SortedSetAdd { member, score } => redis::cmd("ZADD")
                .arg(key)
                .arg(score)
                .arg(member)
                .query_async(&mut connection)
                .await
                .map_err(Into::into),
            RedisMutation::SortedSetRemove { member } => redis::cmd("ZREM")
                .arg(key)
                .arg(member)
                .query_async(&mut connection)
                .await
                .map_err(Into::into),
            RedisMutation::Delete => delete_key_command(key)
                .query_async(&mut connection)
                .await
                .map_err(Into::into),
        }
    }

    async fn execute_redis_command(
        &self,
        database: &str,
        arguments: Vec<String>,
    ) -> anyhow::Result<QueryResult> {
        let mut connection = self.selected_connection(database).await?;
        let Some((name, arguments)) = arguments.split_first() else {
            return Ok(QueryResult::default());
        };
        let started = Instant::now();
        let mut command = redis::cmd(name);
        command.arg(arguments);
        let value: redis::Value = command.query_async(&mut connection).await?;
        Ok(redis_command_result(
            value,
            started.elapsed().as_millis() as u64,
        ))
    }

    async fn get_table_data(
        &self,
        database: &str,
        key: &TableRef,
        _page: u32,
        _page_size: u32,
    ) -> anyhow::Result<QueryResult> {
        let mut conn = self.selected_connection(database).await?;
        let key = key.name();

        let key_type: String = redis::cmd("TYPE").arg(key).query_async(&mut conn).await?;
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
                let vals: Vec<(String, f64)> =
                    conn.zrangebyscore_withscores(key, "-inf", "+inf").await?;
                serde_json::Value::Array(
                    vals.into_iter()
                        .map(
                            |(member, score)| serde_json::json!({"member": member, "score": score}),
                        )
                        .collect(),
                )
            }
            _ => serde_json::Value::String(format!("Unsupported type: {}", key_type)),
        };
        let elapsed = start.elapsed().as_millis() as u64;

        let columns = vec![
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
                data_type: key_type.clone(),
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

fn page_position(key_type: &str, cursor: Option<RedisPageCursor>) -> anyhow::Result<u64> {
    match (key_type, cursor) {
        (_, None) => Ok(0),
        ("hash", Some(RedisPageCursor::Hash(position)))
        | ("list", Some(RedisPageCursor::List(position)))
        | ("set", Some(RedisPageCursor::Set(position)))
        | ("zset", Some(RedisPageCursor::SortedSet(position))) => Ok(position),
        _ => anyhow::bail!("Redis key type changed. Refresh to restart browsing."),
    }
}

fn range_end(offset: u64) -> anyhow::Result<u64> {
    offset
        .checked_add(VALUE_PAGE_SIZE - 1)
        .filter(|end| *end <= i64::MAX as u64)
        .ok_or_else(|| anyhow::anyhow!("Redis page offset is outside the supported range"))
}

fn next_range_page(offset: u64, total: u64) -> Option<u64> {
    offset
        .checked_add(VALUE_PAGE_SIZE)
        .filter(|next| *next < total)
}

fn parse_database_selector(database: &str) -> anyhow::Result<u8> {
    let number = database.strip_prefix("db").ok_or_else(|| {
        anyhow::anyhow!("Invalid Redis database selector {database:?}; expected db<N>")
    })?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        anyhow::bail!("Invalid Redis database selector {database:?}; expected db<N>");
    }
    let database_number = number.parse::<u8>().map_err(|_| {
        anyhow::anyhow!(
            "Invalid Redis database selector {database:?}; database number must be between 0 and 255"
        )
    })?;
    if database != format!("db{database_number}") {
        anyhow::bail!("Invalid Redis database selector {database:?}; expected db<N>");
    }
    Ok(database_number)
}

fn select_database_command(database: &str) -> anyhow::Result<redis::Cmd> {
    let mut command = redis::cmd("SELECT");
    command.arg(parse_database_selector(database)?);
    Ok(command)
}

fn set_key_command(key: &str, value: &str, ttl_seconds: Option<u64>) -> redis::Cmd {
    let mut command = redis::cmd("SET");
    command.arg(key).arg(value);
    if let Some(ttl_seconds) = ttl_seconds {
        command.arg("EX").arg(ttl_seconds);
    }
    command
}

fn delete_key_command(key: &str) -> redis::Cmd {
    let mut command = redis::cmd("DEL");
    command.arg(key);
    command
}

async fn scan_keys(
    connection: &mut redis::aio::MultiplexedConnection,
    pattern: &str,
) -> anyhow::Result<Vec<String>> {
    let mut keys = HashSet::new();
    let mut cursor = 0_u64;
    loop {
        let (next_cursor, batch): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(SCAN_BATCH_SIZE)
            .query_async(connection)
            .await?;
        keys.extend(batch);
        if next_cursor == 0 {
            break;
        }
        cursor = next_cursor;
    }
    let mut keys = keys.into_iter().collect::<Vec<_>>();
    keys.sort_unstable();
    Ok(keys)
}

fn redis_command_result(value: redis::Value, execution_time_ms: u64) -> QueryResult {
    QueryResult {
        columns: vec![ColumnInfo {
            name: "result".to_string(),
            data_type: "Redis".to_string(),
            nullable: true,
            is_primary_key: false,
            default_value: None,
            comment: None,
        }],
        rows: vec![vec![redis_value_to_json(&value)]],
        affected_rows: 0,
        execution_time_ms,
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

#[cfg(test)]
mod tests {
    use super::{
        delete_key_command, parse_database_selector, select_database_command, set_key_command,
    };

    #[test]
    fn page_tokens_reject_type_changes_and_range_overflow() {
        assert_eq!(
            super::page_position("list", Some(super::RedisPageCursor::List(256))).unwrap(),
            256
        );
        assert!(super::page_position("set", Some(super::RedisPageCursor::Hash(17))).is_err());
        assert!(super::page_position("string", Some(super::RedisPageCursor::List(256))).is_err());
        assert_eq!(super::range_end(0).unwrap(), 255);
        assert!(super::range_end(i64::MAX as u64).is_err());
        assert!(super::range_end(u64::MAX).is_err());
        assert_eq!(super::next_range_page(0, 256), None);
        assert_eq!(super::next_range_page(0, 257), Some(256));
    }

    #[tokio::test]
    #[ignore = "requires ASTESIA_REDIS_PAGING_PORT pointing to a disposable localhost Redis server"]
    async fn redis_collection_pages_preserve_coverage_order_and_mutation_identity() {
        use super::{
            ConnectionConfig, DatabaseDriver, DbType, RedisDriver, RedisMutation, RedisPageCursor,
            RedisValue, VALUE_PAGE_SIZE,
        };
        use std::collections::HashSet;

        let port = std::env::var("ASTESIA_REDIS_PAGING_PORT")
            .expect("disposable Redis port")
            .parse()
            .expect("Redis port number");
        let mut driver = RedisDriver::new(ConnectionConfig {
            id: "redis-paging-smoke".to_string(),
            name: "Disposable Redis paging".to_string(),
            db_type: DbType::Redis,
            host: "127.0.0.1".to_string(),
            port,
            username: String::new(),
            password: String::new(),
            database: Some("db0".to_string()),
            color: None,
        });
        driver.connect().await.unwrap();
        let mut connection = driver.selected_connection("db0").await.unwrap();
        let prefix = format!("astesia-paging-{}", uuid::Uuid::new_v4());
        let keys = ["hash", "list", "set", "zset"].map(|kind| format!("{prefix}-{kind}"));
        let entries = (0..700)
            .map(|index| format!("{index:04}-{}", "成员 空格".repeat(100)))
            .collect::<Vec<_>>();
        let mut fixtures = redis::pipe();
        for (index, member) in entries.iter().enumerate() {
            fixtures
                .cmd("HSET")
                .arg(&keys[0])
                .arg(member)
                .arg(format!("value-{index}"))
                .ignore();
            fixtures.cmd("RPUSH").arg(&keys[1]).arg(member).ignore();
            fixtures.cmd("SADD").arg(&keys[2]).arg(member).ignore();
            fixtures
                .cmd("ZADD")
                .arg(&keys[3])
                .arg(index)
                .arg(member)
                .ignore();
        }
        fixtures.query_async::<()>(&mut connection).await.unwrap();
        for (kind, key) in keys.iter().enumerate() {
            let mut cursor = None;
            let mut seen = HashSet::new();
            let mut pages = 0;
            loop {
                let page = driver.get_redis_key_page("db0", key, cursor).await.unwrap();
                assert_eq!(page.total_entries, 700);
                assert!(
                    page.value.entry_count() < 700,
                    "collection must not be fully loaded"
                );
                match page.value {
                    RedisValue::Hash(values) => {
                        for (member, value) in values {
                            let index = member[..4].parse::<usize>().unwrap();
                            assert_eq!(value, format!("value-{index}"));
                            seen.insert(member);
                        }
                    }
                    RedisValue::List(values) => {
                        assert!(values.len() <= VALUE_PAGE_SIZE as usize);
                        for (index, member) in values.into_iter().enumerate() {
                            assert_eq!(member, entries[page.offset as usize + index]);
                            seen.insert(member);
                        }
                    }
                    RedisValue::Set(values) => seen.extend(values),
                    RedisValue::SortedSet(values) => {
                        assert!(values.len() <= VALUE_PAGE_SIZE as usize);
                        for (index, (member, score)) in values.into_iter().enumerate() {
                            assert_eq!(member, entries[page.offset as usize + index]);
                            assert_eq!(score, (page.offset as usize + index) as f64);
                            seen.insert(member);
                        }
                    }
                    _ => panic!("unexpected Redis page type"),
                }
                pages += 1;
                assert!(pages < 20, "cursor traversal failed to terminate");
                cursor = page.next_page;
                if cursor.is_none() {
                    break;
                }
            }
            assert!(pages >= 3);
            assert_eq!(seen, entries.iter().cloned().collect());
            let identity = entries[350].clone();
            let mutation = match kind {
                0 => RedisMutation::HashDelete { field: identity },
                1 => RedisMutation::ListRemove {
                    count: 1,
                    value: identity,
                },
                2 => RedisMutation::SetRemove { member: identity },
                _ => RedisMutation::SortedSetRemove { member: identity },
            };
            assert_eq!(
                driver.mutate_redis_key("db0", key, mutation).await.unwrap(),
                1
            );
            assert_eq!(
                driver
                    .get_redis_key("db0", key)
                    .await
                    .unwrap()
                    .total_entries,
                699
            );
        }
        redis::cmd("LTRIM")
            .arg(&keys[1])
            .arg(0)
            .arg(0)
            .query_async::<()>(&mut connection)
            .await
            .unwrap();
        let empty = driver
            .get_redis_key_page("db0", &keys[1], Some(RedisPageCursor::List(256)))
            .await
            .unwrap();
        assert_eq!(empty.value.entry_count(), 0);
        assert_eq!(empty.next_page, None);
        let cursor = driver
            .get_redis_key("db0", &keys[0])
            .await
            .unwrap()
            .next_page;
        assert!(cursor.is_some());
        driver
            .set_key("db0", &keys[0], "replacement", Some(60))
            .await
            .unwrap();
        assert!(driver
            .get_redis_key_page("db0", &keys[0], cursor)
            .await
            .unwrap_err()
            .to_string()
            .contains("type changed"));
        let string = driver.get_redis_key("db0", &keys[0]).await.unwrap();
        assert_eq!(string.value, RedisValue::String("replacement".to_string()));
        assert!(string.ttl_seconds.is_some_and(|ttl| ttl > 0 && ttl <= 60));
        for key in &keys {
            driver.delete_key("db0", key).await.unwrap();
            let missing = driver.get_redis_key("db0", key).await.unwrap();
            assert_eq!(missing.value, RedisValue::Missing);
            assert_eq!(missing.next_page, None);
            assert_eq!(missing.total_entries, 0);
        }
        driver.disconnect().await.unwrap();
    }

    #[test]
    fn accepts_exact_redis_database_selectors() {
        for (selector, database_number) in [("db0", 0), ("db15", 15), ("db255", 255)] {
            assert_eq!(
                parse_database_selector(selector).expect("valid selector"),
                database_number
            );
            assert_eq!(
                select_database_command(selector)
                    .expect("valid SELECT")
                    .get_packed_command(),
                format!(
                    "*2\r\n$6\r\nSELECT\r\n${}\r\n{}\r\n",
                    selector.len() - 2,
                    database_number
                )
                .into_bytes()
            );
        }
    }

    #[test]
    fn rejects_noncanonical_redis_database_selectors() {
        for selector in [
            "", "0", "db", "DB0", "db-1", "db+1", "db 1", "db1x", " db1", "db1 ", "db01", "db256",
        ] {
            assert!(
                parse_database_selector(selector).is_err(),
                "accepted {selector:?}"
            );
        }
    }

    #[test]
    fn typed_set_preserves_whitespace_in_keys_and_values() {
        assert_eq!(
            set_key_command("key with spaces", "value with spaces", Some(30))
                .get_packed_command(),
            b"*5\r\n$3\r\nSET\r\n$15\r\nkey with spaces\r\n$17\r\nvalue with spaces\r\n$2\r\nEX\r\n$2\r\n30\r\n".to_vec()
        );
        assert_eq!(
            set_key_command("key with spaces", "value with spaces", None).get_packed_command(),
            b"*3\r\n$3\r\nSET\r\n$15\r\nkey with spaces\r\n$17\r\nvalue with spaces\r\n".to_vec()
        );
    }

    #[test]
    fn typed_delete_preserves_the_key_as_one_argument() {
        assert_eq!(
            delete_key_command("key with spaces").get_packed_command(),
            b"*2\r\n$3\r\nDEL\r\n$15\r\nkey with spaces\r\n".to_vec()
        );
    }
}
