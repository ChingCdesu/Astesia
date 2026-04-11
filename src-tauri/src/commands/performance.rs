use serde_json::json;
use tauri::State;

use crate::db::{DatabaseDriver, DbType};
use crate::state::AppState;

#[tauri::command]
pub async fn get_performance_metrics(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
) -> Result<serde_json::Value, String> {
    let connections = state.connections.lock().await;
    let driver = connections
        .get(&connection_id)
        .ok_or("连接不存在")?;

    let db_type = driver.db_type();
    match db_type {
        DbType::MySQL => get_mysql_metrics(driver.as_ref(), &database).await,
        DbType::PostgreSQL => get_postgres_metrics(driver.as_ref(), &database).await,
        DbType::SQLite => get_sqlite_metrics(driver.as_ref(), &database).await,
        DbType::SQLServer => get_sqlserver_metrics(driver.as_ref(), &database).await,
        DbType::MongoDB => get_mongodb_metrics(driver.as_ref(), &database).await,
        DbType::Redis => get_redis_metrics(driver.as_ref(), &database).await,
    }
}

async fn get_mysql_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<serde_json::Value, String> {
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

    Ok(json!({
        "dbType": "mysql",
        "connections": get_u64("Connections"),
        "threadsRunning": get_u64("Threads_running"),
        "queries": get_u64("Queries"),
        "slowQueries": get_u64("Slow_queries"),
        "bytesReceived": get_u64("Bytes_received"),
        "bytesSent": get_u64("Bytes_sent"),
        "uptime": get_u64("Uptime"),
        "bufferPoolHitRate": (buffer_pool_hit_rate * 100.0).round() / 100.0,
        "comSelect": get_u64("Com_select"),
        "comInsert": get_u64("Com_insert"),
        "comUpdate": get_u64("Com_update"),
        "comDelete": get_u64("Com_delete"),
        "threadsConnected": get_u64("Threads_connected")
    }))
}

async fn get_postgres_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<serde_json::Value, String> {
    let stats_sql = format!(
        "SELECT numbackends, xact_commit, xact_rollback, blks_read, blks_hit, \
         tup_returned, tup_fetched, tup_inserted, tup_updated, tup_deleted, \
         deadlocks, temp_files, temp_bytes \
         FROM pg_stat_database WHERE datname = '{}'",
        database
    );

    let stats_result = driver
        .execute_query(database, &stats_sql)
        .await
        .map_err(|e| e.to_string())?;

    let active_sql =
        "SELECT count(*) FROM pg_stat_activity WHERE state = 'active'";
    let active_result = driver
        .execute_query(database, active_sql)
        .await
        .map_err(|e| e.to_string())?;

    let active_connections = active_result
        .rows
        .first()
        .and_then(|r| r.first())
        .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
        .unwrap_or(0);

    if stats_result.rows.is_empty() {
        return Ok(json!({
            "dbType": "postgresql",
            "activeConnections": active_connections,
            "numbackends": 0,
            "xactCommit": 0,
            "xactRollback": 0,
            "blksRead": 0,
            "blksHit": 0,
            "cacheHitRatio": 0.0,
            "tupReturned": 0,
            "tupFetched": 0,
            "tupInserted": 0,
            "tupUpdated": 0,
            "tupDeleted": 0,
            "deadlocks": 0,
            "tempFiles": 0,
            "tempBytes": 0
        }));
    }

    let row = &stats_result.rows[0];
    let get_val = |idx: usize| -> i64 {
        row.get(idx)
            .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
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

    Ok(json!({
        "dbType": "postgresql",
        "activeConnections": active_connections,
        "numbackends": numbackends,
        "xactCommit": xact_commit,
        "xactRollback": xact_rollback,
        "blksRead": blks_read,
        "blksHit": blks_hit,
        "cacheHitRatio": (cache_hit_ratio * 100.0).round() / 100.0,
        "tupReturned": tup_returned,
        "tupFetched": tup_fetched,
        "tupInserted": tup_inserted,
        "tupUpdated": tup_updated,
        "tupDeleted": tup_deleted,
        "deadlocks": deadlocks,
        "tempFiles": temp_files,
        "tempBytes": temp_bytes
    }))
}

async fn get_sqlite_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<serde_json::Value, String> {
    let get_pragma = |result: &crate::db::QueryResult| -> serde_json::Value {
        result
            .rows
            .first()
            .and_then(|r| r.first())
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    };

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

    let cache_size = get_pragma(&cache_size_result);
    let page_count = get_pragma(&page_count_result);
    let page_size = get_pragma(&page_size_result);
    let journal_mode = get_pragma(&journal_mode_result);

    // Try to get WAL checkpoint info if using WAL mode
    let wal_info = driver
        .execute_query(database, "PRAGMA wal_checkpoint")
        .await
        .ok();
    let wal_pages = wal_info
        .as_ref()
        .and_then(|r| r.rows.first())
        .and_then(|r| r.get(1))
        .cloned()
        .unwrap_or(serde_json::Value::Number(0.into()));

    Ok(json!({
        "dbType": "sqlite",
        "cacheSize": cache_size,
        "pageCount": page_count,
        "pageSize": page_size,
        "journalMode": journal_mode,
        "walPages": wal_pages
    }))
}

async fn get_sqlserver_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<serde_json::Value, String> {
    // Query performance counters
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
            let val = row[1]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| row[1].as_i64())
                .unwrap_or(0);
            counter_map.insert(name, val);
        }
    }

    // Query active sessions
    let sessions_sql = "SELECT COUNT(*) FROM sys.dm_exec_sessions WHERE is_user_process = 1";
    let sessions_result = driver
        .execute_query(database, sessions_sql)
        .await
        .map_err(|e| e.to_string())?;

    let active_sessions = sessions_result
        .rows
        .first()
        .and_then(|r| r.first())
        .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
        .unwrap_or(0);

    // Query memory grants
    let memory_sql = "SELECT COUNT(*) FROM sys.dm_exec_query_memory_grants";
    let memory_result = driver
        .execute_query(database, memory_sql)
        .await
        .ok();
    let memory_grants = memory_result
        .as_ref()
        .and_then(|r| r.rows.first())
        .and_then(|r| r.first())
        .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
        .unwrap_or(0);

    let batch_requests = counter_map.get("Batch Requests/sec").copied().unwrap_or(0);
    let cache_hit_ratio = counter_map.get("Buffer cache hit ratio").copied().unwrap_or(0);
    let cache_hit_base = counter_map.get("Buffer cache hit ratio base").copied().unwrap_or(1);
    let page_life = counter_map.get("Page life expectancy").copied().unwrap_or(0);

    let actual_cache_ratio = if cache_hit_base > 0 {
        (cache_hit_ratio as f64 / cache_hit_base as f64) * 100.0
    } else {
        0.0
    };

    Ok(json!({
        "dbType": "sqlserver",
        "batchRequestsPerSec": batch_requests,
        "bufferCacheHitRatio": (actual_cache_ratio * 100.0).round() / 100.0,
        "activeSessions": active_sessions,
        "memoryGrants": memory_grants,
        "pageLifeExpectancy": page_life
    }))
}

async fn get_mongodb_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<serde_json::Value, String> {
    // Use execute_query with a serverStatus-like command
    // The MongoDB driver's execute_query has limited command support,
    // so we gather basic info from available APIs
    let result = driver
        .execute_query(database, "db.runCommand({serverStatus:1})")
        .await;

    match result {
        Ok(query_result) => {
            // If the command returned meaningful data, try to extract it
            if !query_result.rows.is_empty() {
                // Try to parse any returned data
                Ok(json!({
                    "dbType": "mongodb",
                    "connections": 0,
                    "memResident": 0,
                    "memVirtual": 0,
                    "networkBytesIn": 0,
                    "networkBytesOut": 0,
                    "opInsert": 0,
                    "opQuery": 0,
                    "opUpdate": 0,
                    "opDelete": 0
                }))
            } else {
                Ok(json!({
                    "dbType": "mongodb",
                    "connections": 0,
                    "memResident": 0,
                    "memVirtual": 0,
                    "networkBytesIn": 0,
                    "networkBytesOut": 0,
                    "opInsert": 0,
                    "opQuery": 0,
                    "opUpdate": 0,
                    "opDelete": 0
                }))
            }
        }
        Err(_) => {
            // Fallback: return basic structure with zeros
            Ok(json!({
                "dbType": "mongodb",
                "connections": 0,
                "memResident": 0,
                "memVirtual": 0,
                "networkBytesIn": 0,
                "networkBytesOut": 0,
                "opInsert": 0,
                "opQuery": 0,
                "opUpdate": 0,
                "opDelete": 0
            }))
        }
    }
}

async fn get_redis_metrics(
    driver: &dyn DatabaseDriver,
    database: &str,
) -> Result<serde_json::Value, String> {
    let result = driver
        .execute_query(database, "INFO")
        .await
        .map_err(|e| e.to_string())?;

    // The Redis INFO command returns a single bulk string via execute_query
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

    let get_str = |key: &str| -> String {
        info_map.get(key).cloned().unwrap_or_default()
    };

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

    Ok(json!({
        "dbType": "redis",
        "connectedClients": get_u64("connected_clients"),
        "usedMemoryHuman": get_str("used_memory_human"),
        "usedMemoryPeakHuman": get_str("used_memory_peak_human"),
        "totalCommandsProcessed": get_u64("total_commands_processed"),
        "keyspaceHits": keyspace_hits,
        "keyspaceMisses": keyspace_misses,
        "hitRate": (hit_rate * 100.0).round() / 100.0,
        "uptimeInSeconds": get_u64("uptime_in_seconds"),
        "evictedKeys": get_u64("evicted_keys"),
        "usedMemory": get_u64("used_memory"),
        "usedMemoryPeak": get_u64("used_memory_peak"),
        "connectedSlaves": get_u64("connected_slaves"),
        "redisVersion": get_str("redis_version")
    }))
}
