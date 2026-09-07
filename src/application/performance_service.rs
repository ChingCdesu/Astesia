use crate::db::{DatabaseDriver, DbType, PerformanceMode, SqlDialect};

use super::{
    connections::ConnectionManager, ClickHouseMetrics, MongoMetrics, MySqlMetrics,
    PerformanceSnapshot, PostgresMetrics, QueryTarget, RedisMetrics, SqlServerMetrics,
    SqliteMetrics,
};

#[derive(Clone)]
pub struct PerformanceService {
    manager: ConnectionManager,
}

impl PerformanceService {
    pub(super) fn new(manager: ConnectionManager) -> Self {
        Self { manager }
    }

    pub async fn metrics(&self, target: &QueryTarget) -> Result<PerformanceSnapshot, String> {
        let (handle, generation) = self.manager.driver_session(&target.connection_id).await?;
        if generation != target.session_generation {
            return Err(format!(
                "Database Session changed before performance refresh (expected {}, found {generation})",
                target.session_generation
            ));
        }
        let driver = handle.lock_active().await?;
        let driver = &*driver;
        let db_type = driver.db_type();
        if db_type != target.db_type {
            return Err("Database Session engine changed before performance refresh".to_string());
        }
        if db_type.capabilities().performance == PerformanceMode::Unavailable {
            return Ok(PerformanceSnapshot::Unavailable { engine: db_type });
        }

        match db_type {
            DbType::MySQL => get_mysql_metrics(driver, &target.database)
                .await
                .map(PerformanceSnapshot::MySql),
            DbType::PostgreSQL => get_postgres_metrics(driver, &target.database)
                .await
                .map(PerformanceSnapshot::PostgreSql),
            DbType::SQLite => get_sqlite_metrics(driver, &target.database)
                .await
                .map(PerformanceSnapshot::SQLite),
            DbType::SQLServer => get_sqlserver_metrics(driver, &target.database)
                .await
                .map(PerformanceSnapshot::SqlServer),
            DbType::MongoDB => get_mongodb_metrics(driver, &target.database)
                .await
                .map(PerformanceSnapshot::MongoDB),
            DbType::Redis => get_redis_metrics(driver, &target.database)
                .await
                .map(PerformanceSnapshot::Redis),
            DbType::ClickHouse => get_clickhouse_metrics(driver, &target.database)
                .await
                .map(PerformanceSnapshot::ClickHouse),
        }
    }
}

async fn get_mongodb_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<MongoMetrics, String> {
    let status = driver
        .get_mongodb_server_status(database)
        .await
        .map_err(|error| error.to_string())?;
    Ok(mongodb_metrics(&status))
}

fn mongodb_metrics(status: &serde_json::Value) -> MongoMetrics {
    MongoMetrics {
        connections: nested_u64(status, &["connections", "current"]),
        resident_memory_mb: nested_f64(status, &["mem", "resident"]),
        virtual_memory_mb: nested_f64(status, &["mem", "virtual"]),
        network_bytes_in: nested_u64(status, &["network", "bytesIn"]),
        network_bytes_out: nested_u64(status, &["network", "bytesOut"]),
        insert_operations: nested_u64(status, &["opcounters", "insert"]),
        query_operations: nested_u64(status, &["opcounters", "query"]),
        update_operations: nested_u64(status, &["opcounters", "update"]),
        delete_operations: nested_u64(status, &["opcounters", "delete"]),
        uptime_seconds: nested_u64(status, &["uptime"]),
    }
}

fn nested_u64(value: &serde_json::Value, path: &[&str]) -> u64 {
    nested_number(value, path)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as u64)
        .unwrap_or(0)
}

fn nested_f64(value: &serde_json::Value, path: &[&str]) -> f64 {
    nested_number(value, path).unwrap_or(0.0)
}

fn nested_number(value: &serde_json::Value, path: &[&str]) -> Option<f64> {
    let value = path.iter().try_fold(value, |value, key| value.get(*key))?;
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        .or_else(|| {
            ["$numberInt", "$numberLong", "$numberDouble"]
                .iter()
                .find_map(|key| value.get(*key)?.as_str()?.parse().ok())
        })
}

async fn get_clickhouse_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<ClickHouseMetrics, String> {
    let current = driver
        .execute_query(
            database,
            "SELECT metric, value FROM system.metrics \
             WHERE metric IN (\
               'Query', 'Merge', 'PartMutation', 'TCPConnection', \
               'HTTPConnection', 'MemoryTracking'\
             )",
        )
        .await
        .map_err(|e| e.to_string())?;
    let events = driver
        .execute_query(
            database,
            "SELECT event, value FROM system.events \
             WHERE event IN (\
               'Query', 'FailedQuery', 'SelectQuery', 'InsertQuery', \
               'SelectedRows', 'InsertedRows', 'SelectedBytes', 'InsertedBytes'\
             )",
        )
        .await
        .map_err(|e| e.to_string())?;
    let asynchronous = driver
        .execute_query(
            database,
            "SELECT metric, value FROM system.asynchronous_metrics \
             WHERE metric IN ('Uptime', 'NumberOfDatabases', 'NumberOfTables')",
        )
        .await
        .map_err(|e| e.to_string())?;

    let current = metric_rows(&current);
    let events = metric_rows(&events);
    let asynchronous = metric_rows(&asynchronous);
    let metric = |values: &std::collections::HashMap<String, f64>, name: &str| {
        values.get(name).copied().unwrap_or(0.0)
    };

    Ok(ClickHouseMetrics {
        active_queries: metric(&current, "Query"),
        active_merges: metric(&current, "Merge"),
        active_mutations: metric(&current, "PartMutation"),
        connections: metric(&current, "TCPConnection") + metric(&current, "HTTPConnection"),
        memory_usage: metric(&current, "MemoryTracking"),
        total_queries: metric(&events, "Query"),
        failed_queries: metric(&events, "FailedQuery"),
        select_queries: metric(&events, "SelectQuery"),
        insert_queries: metric(&events, "InsertQuery"),
        selected_rows: metric(&events, "SelectedRows"),
        inserted_rows: metric(&events, "InsertedRows"),
        selected_bytes: metric(&events, "SelectedBytes"),
        inserted_bytes: metric(&events, "InsertedBytes"),
        uptime: metric(&asynchronous, "Uptime"),
        database_count: metric(&asynchronous, "NumberOfDatabases"),
        table_count: metric(&asynchronous, "NumberOfTables"),
    })
}

fn metric_rows(result: &crate::db::QueryResult) -> std::collections::HashMap<String, f64> {
    result
        .rows
        .iter()
        .filter_map(|row| {
            let name = row.first()?.as_str()?.to_string();
            let value = row.get(1).and_then(|value| {
                value
                    .as_f64()
                    .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
            })?;
            Some((name, value))
        })
        .collect()
}

async fn get_mysql_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<MySqlMetrics, String> {
    let result = driver
        .execute_query(database, "SHOW GLOBAL STATUS")
        .await
        .map_err(|e| e.to_string())?;

    let mut status_map = std::collections::HashMap::new();
    for row in &result.rows {
        if row.len() >= 2 {
            let key = row[0].as_str().unwrap_or("").to_string();
            let val = row[1].as_str().unwrap_or("0").to_string();
            status_map.insert(key, val);
        }
    }

    let get_u64 = |key: &str| -> u64 {
        status_map
            .get(key)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };

    let get_f64 = |key: &str| -> f64 {
        status_map
            .get(key)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };

    let pool_read_requests = get_f64("Innodb_buffer_pool_read_requests");
    let pool_reads = get_f64("Innodb_buffer_pool_reads");
    let buffer_pool_hit_rate = if pool_read_requests > 0.0 {
        ((pool_read_requests - pool_reads) / pool_read_requests) * 100.0
    } else {
        0.0
    };

    Ok(MySqlMetrics {
        connections: get_u64("Connections"),
        threads_running: get_u64("Threads_running"),
        queries: get_u64("Queries"),
        slow_queries: get_u64("Slow_queries"),
        bytes_received: get_u64("Bytes_received"),
        bytes_sent: get_u64("Bytes_sent"),
        uptime: get_u64("Uptime"),
        buffer_pool_hit_rate: (buffer_pool_hit_rate * 100.0).round() / 100.0,
        selects: get_u64("Com_select"),
        inserts: get_u64("Com_insert"),
        updates: get_u64("Com_update"),
        deletes: get_u64("Com_delete"),
        threads_connected: get_u64("Threads_connected"),
    })
}

async fn get_postgres_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<PostgresMetrics, String> {
    let database_name = SqlDialect::new(DbType::PostgreSQL)
        .literal(&serde_json::Value::String(database.to_string()))
        .map_err(String::from)?;
    let stats_sql = format!(
        "SELECT numbackends, xact_commit, xact_rollback, blks_read, blks_hit, \
         tup_returned, tup_fetched, tup_inserted, tup_updated, tup_deleted, \
         deadlocks, temp_files, temp_bytes \
         FROM pg_stat_database WHERE datname = {database_name}"
    );

    let stats_result = driver
        .execute_query(database, &stats_sql)
        .await
        .map_err(|e| e.to_string())?;

    let active_sql = "SELECT count(*) FROM pg_stat_activity WHERE state = 'active'";
    let active_result = driver
        .execute_query(database, active_sql)
        .await
        .map_err(|e| e.to_string())?;

    let active_connections = active_result
        .rows
        .first()
        .and_then(|r| r.first())
        .and_then(|v| {
            v.as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| v.as_i64())
        })
        .unwrap_or(0);

    let get_val = |idx: usize| -> i64 {
        stats_result
            .rows
            .first()
            .and_then(|row| row.get(idx))
            .and_then(|v| {
                v.as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                    .or_else(|| v.as_i64())
            })
            .unwrap_or(0)
    };

    let numbackends = get_val(0);
    let xact_commit = get_val(1);
    let xact_rollback = get_val(2);
    let blks_read = get_val(3);
    let blks_hit = get_val(4);
    let tup_returned = get_val(5);
    let tup_fetched = get_val(6);
    let tup_inserted = get_val(7);
    let tup_updated = get_val(8);
    let tup_deleted = get_val(9);
    let deadlocks = get_val(10);
    let temp_files = get_val(11);
    let temp_bytes = get_val(12);

    let cache_hit_ratio = if blks_hit + blks_read > 0 {
        (blks_hit as f64 / (blks_hit + blks_read) as f64) * 100.0
    } else {
        0.0
    };

    Ok(PostgresMetrics {
        active_connections,
        backends: numbackends,
        committed_transactions: xact_commit,
        rolled_back_transactions: xact_rollback,
        blocks_read: blks_read,
        blocks_hit: blks_hit,
        cache_hit_ratio: (cache_hit_ratio * 100.0).round() / 100.0,
        tuples_returned: tup_returned,
        tuples_fetched: tup_fetched,
        tuples_inserted: tup_inserted,
        tuples_updated: tup_updated,
        tuples_deleted: tup_deleted,
        deadlocks,
        temporary_files: temp_files,
        temporary_bytes: temp_bytes,
    })
}

async fn get_sqlite_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<SqliteMetrics, String> {
    let cache_size_result = driver
        .execute_query(database, "PRAGMA cache_size")
        .await
        .map_err(|e| e.to_string())?;
    let page_count_result = driver
        .execute_query(database, "PRAGMA page_count")
        .await
        .map_err(|e| e.to_string())?;
    let page_size_result = driver
        .execute_query(database, "PRAGMA page_size")
        .await
        .map_err(|e| e.to_string())?;
    let journal_mode_result = driver
        .execute_query(database, "PRAGMA journal_mode")
        .await
        .map_err(|e| e.to_string())?;

    let wal_info = driver
        .execute_query(database, "PRAGMA wal_checkpoint")
        .await
        .ok();
    let wal_pages = wal_info
        .as_ref()
        .and_then(|r| r.rows.first())
        .and_then(|r| r.get(1))
        .and_then(value_i64)
        .unwrap_or(0);

    Ok(SqliteMetrics {
        cache_size: first_i64(&cache_size_result),
        page_count: first_i64(&page_count_result),
        page_size: first_i64(&page_size_result),
        journal_mode: first_string(&journal_mode_result),
        wal_pages,
    })
}

fn first_i64(result: &crate::db::QueryResult) -> i64 {
    result
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(value_i64)
        .unwrap_or(0)
}

fn first_string(result: &crate::db::QueryResult) -> String {
    result
        .rows
        .first()
        .and_then(|row| row.first())
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string())
        })
        .unwrap_or_default()
}

fn value_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

async fn get_sqlserver_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<SqlServerMetrics, String> {
    let counters_sql = "SELECT counter_name, cntr_value \
         FROM sys.dm_os_performance_counters \
         WHERE counter_name IN ('Batch Requests/sec', 'Buffer cache hit ratio', 'Buffer cache hit ratio base', 'Page life expectancy') \
         AND object_name LIKE '%Buffer Manager%' OR object_name LIKE '%SQL Statistics%'";

    let counters_result = driver
        .execute_query(database, counters_sql)
        .await
        .map_err(|e| e.to_string())?;

    let mut counter_map = std::collections::HashMap::new();
    for row in &counters_result.rows {
        if row.len() >= 2 {
            let name = row[0].as_str().unwrap_or("").trim().to_string();
            let val = value_i64(&row[1]).unwrap_or(0);
            counter_map.insert(name, val);
        }
    }

    let sessions_sql = "SELECT COUNT(*) FROM sys.dm_exec_sessions WHERE is_user_process = 1";
    let sessions_result = driver
        .execute_query(database, sessions_sql)
        .await
        .map_err(|e| e.to_string())?;

    let active_sessions = sessions_result
        .rows
        .first()
        .and_then(|r| r.first())
        .and_then(value_i64)
        .unwrap_or(0);

    let memory_sql = "SELECT COUNT(*) FROM sys.dm_exec_query_memory_grants";
    let memory_result = driver.execute_query(database, memory_sql).await.ok();
    let memory_grants = memory_result
        .as_ref()
        .and_then(|r| r.rows.first())
        .and_then(|r| r.first())
        .and_then(value_i64)
        .unwrap_or(0);

    let batch_requests = counter_map.get("Batch Requests/sec").copied().unwrap_or(0);
    let cache_hit_ratio = counter_map
        .get("Buffer cache hit ratio")
        .copied()
        .unwrap_or(0);
    let cache_hit_base = counter_map
        .get("Buffer cache hit ratio base")
        .copied()
        .unwrap_or(1);
    let page_life = counter_map
        .get("Page life expectancy")
        .copied()
        .unwrap_or(0);

    let actual_cache_ratio = if cache_hit_base > 0 {
        (cache_hit_ratio as f64 / cache_hit_base as f64) * 100.0
    } else {
        0.0
    };

    Ok(SqlServerMetrics {
        batch_requests_per_second: batch_requests,
        buffer_cache_hit_ratio: (actual_cache_ratio * 100.0).round() / 100.0,
        active_sessions,
        memory_grants,
        page_life_expectancy: page_life,
    })
}

async fn get_redis_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<RedisMetrics, String> {
    let result = driver
        .execute_query(database, "INFO")
        .await
        .map_err(|e| e.to_string())?;

    let info_text = result
        .rows
        .first()
        .and_then(|r| r.first())
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut info_map = std::collections::HashMap::new();
    for line in info_text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            info_map.insert(key.to_string(), value.to_string());
        }
    }

    let get_str = |key: &str| -> String { info_map.get(key).cloned().unwrap_or_default() };

    let get_u64 = |key: &str| -> u64 {
        info_map
            .get(key)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };

    let keyspace_hits = get_u64("keyspace_hits");
    let keyspace_misses = get_u64("keyspace_misses");
    let hit_rate = if keyspace_hits + keyspace_misses > 0 {
        (keyspace_hits as f64 / (keyspace_hits + keyspace_misses) as f64) * 100.0
    } else {
        0.0
    };

    Ok(RedisMetrics {
        connected_clients: get_u64("connected_clients"),
        used_memory_human: get_str("used_memory_human"),
        used_memory_peak_human: get_str("used_memory_peak_human"),
        total_commands_processed: get_u64("total_commands_processed"),
        keyspace_hits,
        keyspace_misses,
        hit_rate: (hit_rate * 100.0).round() / 100.0,
        uptime_seconds: get_u64("uptime_in_seconds"),
        evicted_keys: get_u64("evicted_keys"),
        used_memory: get_u64("used_memory"),
        used_memory_peak: get_u64("used_memory_peak"),
        connected_replicas: get_u64("connected_slaves"),
        version: get_str("redis_version"),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn mongodb_server_status_maps_native_and_extended_numbers() {
        let metrics = mongodb_metrics(&json!({
            "connections": { "current": 12 },
            "mem": { "resident": 84.5, "virtual": 512 },
            "network": {
                "bytesIn": { "$numberLong": "2048" },
                "bytesOut": 4096
            },
            "opcounters": {
                "insert": 7,
                "query": { "$numberInt": "11" },
                "update": 3,
                "delete": 2
            },
            "uptime": { "$numberDouble": "3600" }
        }));

        assert_eq!(metrics.connections, 12);
        assert_eq!(metrics.resident_memory_mb, 84.5);
        assert_eq!(metrics.virtual_memory_mb, 512.0);
        assert_eq!(metrics.network_bytes_in, 2048);
        assert_eq!(metrics.network_bytes_out, 4096);
        assert_eq!(metrics.insert_operations, 7);
        assert_eq!(metrics.query_operations, 11);
        assert_eq!(metrics.update_operations, 3);
        assert_eq!(metrics.delete_operations, 2);
        assert_eq!(metrics.uptime_seconds, 3600);
    }

    #[test]
    fn mongodb_server_status_defaults_missing_or_invalid_metrics_to_zero() {
        let metrics = mongodb_metrics(&json!({
            "connections": { "current": -1 },
            "mem": { "resident": "not-a-number" }
        }));

        assert_eq!(metrics.connections, 0);
        assert_eq!(metrics.resident_memory_mb, 0.0);
        assert_eq!(metrics.network_bytes_in, 0);
        assert_eq!(metrics.query_operations, 0);
    }
}
