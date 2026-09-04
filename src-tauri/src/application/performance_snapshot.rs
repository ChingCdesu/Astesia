use crate::db::DbType;

#[derive(Debug, Clone, PartialEq)]
pub enum PerformanceSnapshot {
    MySql(MySqlMetrics),
    PostgreSql(PostgresMetrics),
    SQLite(SqliteMetrics),
    SqlServer(SqlServerMetrics),
    MongoDB(MongoMetrics),
    Redis(RedisMetrics),
    ClickHouse(ClickHouseMetrics),
    Unavailable { engine: DbType },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MySqlMetrics {
    pub connections: u64,
    pub threads_running: u64,
    pub queries: u64,
    pub slow_queries: u64,
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub uptime: u64,
    pub buffer_pool_hit_rate: f64,
    pub selects: u64,
    pub inserts: u64,
    pub updates: u64,
    pub deletes: u64,
    pub threads_connected: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PostgresMetrics {
    pub active_connections: i64,
    pub backends: i64,
    pub committed_transactions: i64,
    pub rolled_back_transactions: i64,
    pub blocks_read: i64,
    pub blocks_hit: i64,
    pub cache_hit_ratio: f64,
    pub tuples_returned: i64,
    pub tuples_fetched: i64,
    pub tuples_inserted: i64,
    pub tuples_updated: i64,
    pub tuples_deleted: i64,
    pub deadlocks: i64,
    pub temporary_files: i64,
    pub temporary_bytes: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqliteMetrics {
    pub cache_size: i64,
    pub page_count: i64,
    pub page_size: i64,
    pub journal_mode: String,
    pub wal_pages: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SqlServerMetrics {
    pub batch_requests_per_second: i64,
    pub buffer_cache_hit_ratio: f64,
    pub active_sessions: i64,
    pub memory_grants: i64,
    pub page_life_expectancy: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MongoMetrics {
    pub connections: u64,
    pub resident_memory_mb: f64,
    pub virtual_memory_mb: f64,
    pub network_bytes_in: u64,
    pub network_bytes_out: u64,
    pub insert_operations: u64,
    pub query_operations: u64,
    pub update_operations: u64,
    pub delete_operations: u64,
    pub uptime_seconds: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RedisMetrics {
    pub connected_clients: u64,
    pub used_memory_human: String,
    pub used_memory_peak_human: String,
    pub total_commands_processed: u64,
    pub keyspace_hits: u64,
    pub keyspace_misses: u64,
    pub hit_rate: f64,
    pub uptime_seconds: u64,
    pub evicted_keys: u64,
    pub used_memory: u64,
    pub used_memory_peak: u64,
    pub connected_replicas: u64,
    pub version: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClickHouseMetrics {
    pub active_queries: f64,
    pub active_merges: f64,
    pub active_mutations: f64,
    pub connections: f64,
    pub memory_usage: f64,
    pub total_queries: f64,
    pub failed_queries: f64,
    pub select_queries: f64,
    pub insert_queries: f64,
    pub selected_rows: f64,
    pub inserted_rows: f64,
    pub selected_bytes: f64,
    pub inserted_bytes: f64,
    pub uptime: f64,
    pub database_count: f64,
    pub table_count: f64,
}
