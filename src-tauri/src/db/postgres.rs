use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use sqlx::postgres::types::{
    Oid, PgBox, PgCircle, PgInterval, PgLSeg, PgLine, PgMoney, PgPath, PgPoint, PgPolygon, PgRange,
    PgTimeTz,
};
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::types::ipnetwork::IpNetwork;
use sqlx::types::mac_address::MacAddress;
use sqlx::types::{BigDecimal, BitVec, Uuid};
use sqlx::{Column, PgConnection, Row, TypeInfo, ValueRef};
use std::time::Instant;

use super::{bytes_to_hex, f32_to_json, f64_to_json, ColumnInfo, ConnectionConfig, DatabaseDriver, DbType, ForeignKeyInfo, FunctionInfo, IndexInfo, ProcedureInfo, QueryResult, StatementResult, TableInfo, TriggerInfo, UserInfo, ViewInfo};

/// Render a `BitVec` (BIT / VARBIT) as a string of '0'/'1' characters.
fn bitvec_to_string(bits: &BitVec) -> String {
    bits.iter().map(|b| if b { '1' } else { '0' }).collect()
}

fn pg_point_to_string(p: &PgPoint) -> String {
    format!("({},{})", p.x, p.y)
}

/// Format a PostgreSQL INTERVAL roughly like psql does
/// (e.g. "1 year 2 mons 3 days 04:05:06").
fn interval_to_string(iv: &PgInterval) -> String {
    let mut parts: Vec<String> = Vec::new();
    let years = iv.months / 12;
    let mons = iv.months % 12;
    if years != 0 {
        parts.push(format!("{} year{}", years, if years.abs() == 1 { "" } else { "s" }));
    }
    if mons != 0 {
        parts.push(format!("{} mon{}", mons, if mons.abs() == 1 { "" } else { "s" }));
    }
    if iv.days != 0 {
        parts.push(format!("{} day{}", iv.days, if iv.days.abs() == 1 { "" } else { "s" }));
    }
    if iv.microseconds != 0 || parts.is_empty() {
        let neg = iv.microseconds < 0;
        let abs = iv.microseconds.unsigned_abs();
        let secs = abs / 1_000_000;
        let micros = abs % 1_000_000;
        let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
        let sign = if neg { "-" } else { "" };
        if micros != 0 {
            parts.push(format!("{sign}{h:02}:{m:02}:{s:02}.{micros:06}"));
        } else {
            parts.push(format!("{sign}{h:02}:{m:02}:{s:02}"));
        }
    }
    parts.join(" ")
}

/// Decode the `i`-th column of `row` into a JSON value, dispatching on the
/// (upper-cased) PostgreSQL type name. Covers every built-in scalar type plus
/// arrays of the common element types; unknown / user-defined types (enums,
/// domains, …) fall back to their raw text representation so they still display.
fn pg_value_to_json(row: &PgRow, i: usize, type_name: &str) -> serde_json::Value {
    use serde_json::Value as J;

    // Decode column `i` as `$ty`, mapping the value with `$f`; any decode error → NULL.
    macro_rules! get {
        ($ty:ty, $f:expr) => {
            row.try_get::<$ty, _>(i).map($f).unwrap_or(J::Null)
        };
    }
    // Decode column `i` as `$ty` and render it via `Display`.
    macro_rules! get_str {
        ($ty:ty) => {
            row.try_get::<$ty, _>(i)
                .map(|v| J::String(v.to_string()))
                .unwrap_or(J::Null)
        };
    }

    match type_name {
        "BOOL" => get!(bool, J::Bool),

        "INT2" => get!(i16, |v| J::Number(v.into())),
        "INT4" => get!(i32, |v| J::Number(v.into())),
        "INT8" => get!(i64, |v| J::Number(v.into())),
        "OID" => get!(Oid, |v| J::Number(v.0.into())),

        "FLOAT4" => get!(f32, f32_to_json),
        "FLOAT8" => get!(f64, f64_to_json),
        "NUMERIC" => get_str!(BigDecimal),
        "MONEY" => get!(PgMoney, |v| J::String(v.to_bigdecimal(2).to_string())),

        "TEXT" | "VARCHAR" | "CHAR" | "NAME" | "UNKNOWN" => get!(String, J::String),

        "UUID" => get_str!(Uuid),
        "JSON" | "JSONB" => row.try_get::<J, _>(i).unwrap_or(J::Null),
        "BYTEA" => get!(Vec<u8>, |b: Vec<u8>| J::String(bytes_to_hex(&b, "\\x"))),

        "DATE" => get_str!(NaiveDate),
        "TIME" => get_str!(NaiveTime),
        "TIMESTAMP" => get_str!(NaiveDateTime),
        "TIMESTAMPTZ" => get!(DateTime<Utc>, |v| J::String(v.to_rfc3339())),
        "TIMETZ" => get!(PgTimeTz, |v| J::String(format!("{}{}", v.time, v.offset))),
        "INTERVAL" => get!(PgInterval, |v| J::String(interval_to_string(&v))),

        "INET" | "CIDR" => get_str!(IpNetwork),
        "MACADDR" => get_str!(MacAddress),
        "BIT" | "VARBIT" => get!(BitVec, |v: BitVec| J::String(bitvec_to_string(&v))),

        "POINT" => get!(PgPoint, |p| J::String(pg_point_to_string(&p))),
        "LINE" => get!(PgLine, |l| J::String(format!("{{{},{},{}}}", l.a, l.b, l.c))),
        "LSEG" => get!(PgLSeg, |s| J::String(format!(
            "[({},{}),({},{})]",
            s.start_x, s.start_y, s.end_x, s.end_y
        ))),
        "BOX" => get!(PgBox, |b| J::String(format!(
            "({},{}),({},{})",
            b.upper_right_x, b.upper_right_y, b.lower_left_x, b.lower_left_y
        ))),
        "PATH" => get!(PgPath, |p| {
            let pts: Vec<String> = p.points.iter().map(pg_point_to_string).collect();
            J::String(if p.closed {
                format!("({})", pts.join(","))
            } else {
                format!("[{}]", pts.join(","))
            })
        }),
        "POLYGON" => get!(PgPolygon, |p| {
            let pts: Vec<String> = p.points.iter().map(pg_point_to_string).collect();
            J::String(format!("({})", pts.join(",")))
        }),
        "CIRCLE" => get!(PgCircle, |c| J::String(format!("<({},{}),{}>", c.x, c.y, c.radius))),

        "INT4RANGE" => get_str!(PgRange<i32>),
        "INT8RANGE" => get_str!(PgRange<i64>),
        "NUMRANGE" => get_str!(PgRange<BigDecimal>),
        "DATERANGE" => get_str!(PgRange<NaiveDate>),
        "TSRANGE" => get_str!(PgRange<NaiveDateTime>),
        "TSTZRANGE" => get_str!(PgRange<DateTime<Utc>>),

        name if name.ends_with("[]") => pg_array_to_json(row, i, &name[..name.len() - 2]),

        // Unknown / user-defined (enum, domain, …): recover the raw text value
        // (Postgres transmits enum labels as text), else hex bytes, else NULL.
        _ => match row.try_get_raw(i) {
            Ok(raw) if !raw.is_null() => match raw.as_str() {
                Ok(s) => J::String(s.to_string()),
                Err(_) => raw
                    .as_bytes()
                    .map(|b| J::String(bytes_to_hex(b, "\\x")))
                    .unwrap_or(J::Null),
            },
            _ => J::Null,
        },
    }
}

/// Decode a PostgreSQL array column into a JSON array, dispatching on the
/// element type name. Element NULLs are preserved as JSON `null`.
fn pg_array_to_json(row: &PgRow, i: usize, elem: &str) -> serde_json::Value {
    use serde_json::Value as J;

    macro_rules! arr {
        ($ty:ty, $f:expr) => {
            row.try_get::<Vec<Option<$ty>>, _>(i)
                .map(|items| J::Array(items.into_iter().map(|o| o.map_or(J::Null, $f)).collect()))
                .unwrap_or(J::Null)
        };
    }
    macro_rules! arr_str {
        ($ty:ty) => {
            arr!($ty, |v: $ty| J::String(v.to_string()))
        };
    }

    match elem {
        "BOOL" => arr!(bool, J::Bool),
        "INT2" => arr!(i16, |v| J::Number(v.into())),
        "INT4" => arr!(i32, |v| J::Number(v.into())),
        "INT8" => arr!(i64, |v| J::Number(v.into())),
        "OID" => arr!(Oid, |v: Oid| J::Number(v.0.into())),
        "FLOAT4" => arr!(f32, f32_to_json),
        "FLOAT8" => arr!(f64, f64_to_json),
        "NUMERIC" => arr_str!(BigDecimal),
        "TEXT" | "VARCHAR" | "CHAR" | "NAME" => arr!(String, J::String),
        "UUID" => arr_str!(Uuid),
        "JSON" | "JSONB" => row
            .try_get::<Vec<Option<J>>, _>(i)
            .map(|items| J::Array(items.into_iter().map(|o| o.unwrap_or(J::Null)).collect()))
            .unwrap_or(J::Null),
        "BYTEA" => arr!(Vec<u8>, |b: Vec<u8>| J::String(bytes_to_hex(&b, "\\x"))),
        "DATE" => arr_str!(NaiveDate),
        "TIME" => arr_str!(NaiveTime),
        "TIMESTAMP" => arr_str!(NaiveDateTime),
        "TIMESTAMPTZ" => arr!(DateTime<Utc>, |v: DateTime<Utc>| J::String(v.to_rfc3339())),
        "INET" | "CIDR" => arr_str!(IpNetwork),
        "MACADDR" => arr_str!(MacAddress),
        // Less common element types: render the array as its text form if the
        // elements decode as text, else NULL.
        _ => row
            .try_get::<Vec<Option<String>>, _>(i)
            .map(|items| {
                J::Array(items.into_iter().map(|o| o.map_or(J::Null, J::String)).collect())
            })
            .unwrap_or(J::Null),
    }
}

async fn run_pg_query(conn: &mut PgConnection, sql: &str) -> anyhow::Result<QueryResult> {
    let start = Instant::now();
    let trimmed = sql.trim().to_uppercase();

    if trimmed.starts_with("SELECT")
        || trimmed.starts_with("SHOW")
        || trimmed.starts_with("EXPLAIN")
        || trimmed.starts_with("WITH ")
        || trimmed.starts_with("VALUES")
    {
        let rows: Vec<PgRow> = sqlx::query(sql).fetch_all(&mut *conn).await?;
        let elapsed = start.elapsed().as_millis() as u64;

        if rows.is_empty() {
            return Ok(QueryResult {
                execution_time_ms: elapsed,
                ..Default::default()
            });
        }

        let columns: Vec<ColumnInfo> = rows[0]
            .columns()
            .iter()
            .map(|c| ColumnInfo {
                name: c.name().to_string(),
                data_type: c.type_info().name().to_string(),
                nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
            })
            .collect();

        // Pre-compute each column's (upper-cased) PostgreSQL type name once;
        // value decoding then dispatches on it for every row.
        let type_names: Vec<String> = rows[0]
            .columns()
            .iter()
            .map(|c| c.type_info().name().to_ascii_uppercase())
            .collect();

        let data_rows: Vec<Vec<serde_json::Value>> = rows
            .iter()
            .map(|row| {
                (0..type_names.len())
                    .map(|i| pg_value_to_json(row, i, &type_names[i]))
                    .collect()
            })
            .collect();

        Ok(QueryResult {
            columns,
            rows: data_rows,
            affected_rows: 0,
            execution_time_ms: elapsed,
        })
    } else {
        let result = sqlx::query(sql).execute(&mut *conn).await?;
        let elapsed = start.elapsed().as_millis() as u64;
        Ok(QueryResult {
            affected_rows: result.rows_affected(),
            execution_time_ms: elapsed,
            ..Default::default()
        })
    }
}

pub struct PostgresDriver {
    config: ConnectionConfig,
    pool: Option<PgPool>,
    /// Cache of per-database connection pools for cross-database queries
    db_pools: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, PgPool>>>,
}

impl PostgresDriver {
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            pool: None,
            db_pools: std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    fn connection_string(&self, database: Option<&str>) -> String {
        let db = database
            .or(self.config.database.as_deref())
            .unwrap_or("postgres");
        format!(
            "postgres://{}:{}@{}:{}/{}",
            self.config.username, self.config.password, self.config.host, self.config.port, db
        )
    }

    fn pool(&self) -> anyhow::Result<&PgPool> {
        self.pool.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))
    }

    /// Parse a table reference that may be schema-qualified (e.g. "myschema.mytable").
    /// Returns (schema, table_name). Defaults to "public" if no schema is specified.
    fn parse_table_ref(table: &str) -> (&str, &str) {
        if let Some(dot_pos) = table.find('.') {
            (&table[..dot_pos], &table[dot_pos + 1..])
        } else {
            ("public", table)
        }
    }

    /// Get a connection pool for a specific database. Returns the main pool
    /// if the database matches the connected one, or creates/reuses a cached pool.
    async fn pool_for_db(&self, database: &str) -> anyhow::Result<PgPool> {
        // Check if it's the same as the main connected database
        let main_db = self.config.database.as_deref().unwrap_or("postgres");
        if database == main_db {
            return self.pool().cloned();
        }
        // Check cache
        let mut cache = self.db_pools.lock().await;
        if let Some(pool) = cache.get(database) {
            return Ok(pool.clone());
        }
        // Create new pool for this database
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&self.connection_string(Some(database)))
            .await?;
        cache.insert(database.to_string(), pool.clone());
        Ok(pool)
    }
}

#[async_trait]
impl DatabaseDriver for PostgresDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&self.connection_string(None))
            .await?;
        self.pool = Some(pool);
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        if let Some(pool) = self.pool.take() {
            pool.close().await;
        }
        Ok(())
    }

    async fn test_connection(&self) -> anyhow::Result<bool> {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.connection_string(None))
            .await?;
        let _: (i32,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await?;
        pool.close().await;
        Ok(true)
    }

    async fn get_databases(&self) -> anyhow::Result<Vec<String>> {
        let pool = self.pool()?;
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn get_tables(&self, database: &str) -> anyhow::Result<Vec<TableInfo>> {
        let pool = self.pool_for_db(database).await?;
        let rows: Vec<PgRow> = sqlx::query(
            "SELECT tablename, schemaname FROM pg_tables \
             WHERE schemaname NOT IN ('pg_catalog', 'information_schema') \
             ORDER BY tablename",
        )
        .fetch_all(&pool)
        .await?;
        let tables = rows
            .iter()
            .map(|row| TableInfo {
                name: row.get::<String, _>("tablename"),
                schema: row.try_get::<String, _>("schemaname").ok(),
                row_count: None,
                comment: None,
            })
            .collect();
        Ok(tables)
    }

    async fn get_columns(&self, database: &str, table: &str) -> anyhow::Result<Vec<ColumnInfo>> {
        let pool = self.pool_for_db(database).await?;
        let (schema, tbl) = Self::parse_table_ref(table);
        let sql = format!(
            "SELECT c.column_name, c.data_type, c.udt_name, c.is_nullable, c.column_default, \
             CASE WHEN tc.constraint_type = 'PRIMARY KEY' THEN true ELSE false END as is_pk \
             FROM information_schema.columns c \
             LEFT JOIN information_schema.key_column_usage kcu \
               ON c.column_name = kcu.column_name AND c.table_name = kcu.table_name AND c.table_schema = kcu.table_schema \
             LEFT JOIN information_schema.table_constraints tc \
               ON kcu.constraint_name = tc.constraint_name AND tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = kcu.table_schema \
             WHERE c.table_name = '{}' AND c.table_schema = '{}' \
             ORDER BY c.ordinal_position",
            tbl, schema
        );
        let rows: Vec<PgRow> = sqlx::query(&sql).fetch_all(&pool).await?;
        let columns = rows
            .iter()
            .map(|row| ColumnInfo {
                name: row.get::<String, _>("column_name"),
                data_type: {
                    let dt: String = row.get("data_type");
                    if dt == "USER-DEFINED" {
                        row.try_get::<String, _>("udt_name").unwrap_or(dt)
                    } else {
                        dt
                    }
                },
                nullable: row.get::<String, _>("is_nullable") == "YES",
                is_primary_key: row.try_get::<bool, _>("is_pk").unwrap_or(false),
                default_value: row.try_get::<String, _>("column_default").ok(),
                comment: None,
            })
            .collect();
        Ok(columns)
    }

    async fn get_indexes(&self, database: &str, table: &str) -> anyhow::Result<Vec<IndexInfo>> {
        let pool = self.pool_for_db(database).await?;
        let (schema, tbl) = Self::parse_table_ref(table);
        let sql = format!(
            "SELECT i.relname as index_name, a.attname as column_name, ix.indisunique, ix.indisprimary \
             FROM pg_class t \
             JOIN pg_namespace n ON t.relnamespace = n.oid \
             JOIN pg_index ix ON t.oid = ix.indrelid \
             JOIN pg_class i ON i.oid = ix.indexrelid \
             JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey) \
             WHERE t.relkind = 'r' AND t.relname = '{}' AND n.nspname = '{}'",
            tbl, schema
        );
        let rows: Vec<PgRow> = sqlx::query(&sql).fetch_all(&pool).await?;
        let mut indexes: std::collections::HashMap<String, IndexInfo> = std::collections::HashMap::new();
        for row in &rows {
            let name: String = row.get("index_name");
            let column: String = row.get("column_name");
            let is_unique: bool = row.get("indisunique");
            let is_primary: bool = row.get("indisprimary");
            let entry = indexes.entry(name.clone()).or_insert_with(|| IndexInfo {
                name: name.clone(),
                columns: vec![],
                is_unique,
                is_primary,
            });
            entry.columns.push(column);
        }
        Ok(indexes.into_values().collect())
    }

    async fn execute_query(&self, database: &str, sql: &str) -> anyhow::Result<QueryResult> {
        let pool = self.pool_for_db(database).await?;
        let mut conn = pool.acquire().await?;
        run_pg_query(&mut *conn, sql).await
    }

    async fn execute_statements(
        &self,
        database: &str,
        statements: Vec<String>,
    ) -> anyhow::Result<Vec<StatementResult>> {
        let pool = self.pool_for_db(database).await?;
        let mut conn = pool.acquire().await?;
        let mut results = Vec::with_capacity(statements.len());
        for sql in statements {
            let start = Instant::now();
            match run_pg_query(&mut *conn, &sql).await {
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

    async fn get_table_data(
        &self,
        database: &str,
        table: &str,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<QueryResult> {
        let (schema, tbl) = Self::parse_table_ref(table);
        let offset = (page - 1) * page_size;
        let sql = format!(
            "SELECT * FROM \"{}\".\"{}\" LIMIT {} OFFSET {}",
            schema, tbl, page_size, offset
        );
        self.execute_query(database, &sql).await
    }

    async fn get_views(&self, database: &str) -> anyhow::Result<Vec<ViewInfo>> {
        let pool = self.pool_for_db(database).await?;
        let rows: Vec<PgRow> = sqlx::query(
            "SELECT viewname, definition, schemaname FROM pg_views \
             WHERE schemaname NOT IN ('pg_catalog', 'information_schema')"
        )
        .fetch_all(&pool)
        .await?;
        let views = rows
            .iter()
            .map(|row| {
                let schema: String = row.get("schemaname");
                let name: String = row.get("viewname");
                ViewInfo {
                    name: format!("{}.{}", schema, name),
                    definition: row.try_get::<String, _>("definition").ok(),
                }
            })
            .collect();
        Ok(views)
    }

    async fn get_functions(&self, database: &str) -> anyhow::Result<Vec<FunctionInfo>> {
        let pool = self.pool_for_db(database).await?;
        let rows: Vec<PgRow> = sqlx::query(
            "SELECT n.nspname, p.proname, l.lanname, pg_get_function_result(p.oid) as return_type, pg_get_functiondef(p.oid) as definition \
             FROM pg_proc p \
             JOIN pg_namespace n ON p.pronamespace = n.oid \
             JOIN pg_language l ON p.prolang = l.oid \
             WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') AND p.prokind = 'f'"
        )
        .fetch_all(&pool)
        .await?;
        let functions = rows
            .iter()
            .map(|row| {
                let schema: String = row.get("nspname");
                let name: String = row.get("proname");
                FunctionInfo {
                    name: format!("{}.{}", schema, name),
                    language: row.try_get::<String, _>("lanname").ok(),
                    return_type: row.try_get::<String, _>("return_type").ok(),
                    definition: row.try_get::<String, _>("definition").ok(),
                }
            })
            .collect();
        Ok(functions)
    }

    async fn get_procedures(&self, database: &str) -> anyhow::Result<Vec<ProcedureInfo>> {
        let pool = self.pool_for_db(database).await?;
        let rows: Vec<PgRow> = sqlx::query(
            "SELECT n.nspname, p.proname, l.lanname, pg_get_functiondef(p.oid) as definition \
             FROM pg_proc p \
             JOIN pg_namespace n ON p.pronamespace = n.oid \
             JOIN pg_language l ON p.prolang = l.oid \
             WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') AND p.prokind = 'p'"
        )
        .fetch_all(&pool)
        .await?;
        let procedures = rows
            .iter()
            .map(|row| {
                let schema: String = row.get("nspname");
                let name: String = row.get("proname");
                ProcedureInfo {
                    name: format!("{}.{}", schema, name),
                    language: row.try_get::<String, _>("lanname").ok(),
                    definition: row.try_get::<String, _>("definition").ok(),
                }
            })
            .collect();
        Ok(procedures)
    }

    async fn get_triggers(&self, database: &str) -> anyhow::Result<Vec<TriggerInfo>> {
        let pool = self.pool_for_db(database).await?;
        let rows: Vec<PgRow> = sqlx::query(
            "SELECT trigger_schema, trigger_name, event_manipulation, event_object_table, action_timing \
             FROM information_schema.triggers \
             WHERE trigger_schema NOT IN ('pg_catalog', 'information_schema')"
        )
        .fetch_all(&pool)
        .await?;
        let triggers = rows
            .iter()
            .map(|row| {
                let schema: String = row.get("trigger_schema");
                let name: String = row.get("trigger_name");
                TriggerInfo {
                    name: format!("{}.{}", schema, name),
                    event: row.get::<String, _>("event_manipulation"),
                    table: row.get::<String, _>("event_object_table"),
                    timing: row.get::<String, _>("action_timing"),
                }
            })
            .collect();
        Ok(triggers)
    }

    async fn get_foreign_keys(&self, database: &str, table: &str) -> anyhow::Result<Vec<ForeignKeyInfo>> {
        let pool = self.pool_for_db(database).await?;
        let (schema, tbl) = Self::parse_table_ref(table);
        let sql = format!(
            "SELECT tc.constraint_name, kcu.table_name, kcu.column_name, \
             ccu.table_name AS referenced_table, ccu.column_name AS referenced_column \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
             JOIN information_schema.constraint_column_usage ccu \
               ON ccu.constraint_name = tc.constraint_name AND ccu.table_schema = tc.table_schema \
             WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_name = '{}' AND tc.table_schema = '{}'",
            tbl, schema
        );
        let rows: Vec<PgRow> = sqlx::query(&sql).fetch_all(&pool).await?;
        let mut fk_map: std::collections::HashMap<String, ForeignKeyInfo> = std::collections::HashMap::new();
        for row in &rows {
            let name: String = row.get("constraint_name");
            let from_col: String = row.get("column_name");
            let to_table: String = row.get("referenced_table");
            let to_col: String = row.get("referenced_column");
            let entry = fk_map.entry(name.clone()).or_insert_with(|| ForeignKeyInfo {
                name: name.clone(),
                from_table: tbl.to_string(),
                from_columns: vec![],
                to_table: to_table.clone(),
                to_columns: vec![],
            });
            entry.from_columns.push(from_col);
            entry.to_columns.push(to_col);
        }
        Ok(fk_map.into_values().collect())
    }

    async fn get_users(&self) -> anyhow::Result<Vec<UserInfo>> {
        let pool = self.pool()?;
        let rows: Vec<PgRow> = sqlx::query(
            "SELECT rolname, rolcanlogin, rolsuper, rolcreaterole, rolcreatedb \
             FROM pg_roles WHERE rolname NOT LIKE 'pg_%' ORDER BY rolname"
        )
        .fetch_all(pool)
        .await?;
        let users = rows
            .iter()
            .map(|row| {
                let name: String = row.get("rolname");
                let can_login: bool = row.try_get::<bool, _>("rolcanlogin").unwrap_or(false);
                UserInfo {
                    name,
                    host: Some(if can_login { "user".to_string() } else { "group".to_string() }),
                }
            })
            .collect();
        Ok(users)
    }

    async fn get_schemas(&self, database: &str) -> anyhow::Result<Vec<String>> {
        let pool = self.pool_for_db(database).await?;
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT schema_name FROM information_schema.schemata ORDER BY schema_name"
        )
        .fetch_all(&pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn get_enum_values(&self, database: &str, enum_type: &str) -> anyhow::Result<Vec<String>> {
        let pool = self.pool_for_db(database).await?;
        let sql = format!(
            "SELECT e.enumlabel FROM pg_enum e JOIN pg_type t ON e.enumtypid = t.oid WHERE t.typname = '{}' ORDER BY e.enumsortorder",
            enum_type
        );
        let rows: Vec<PgRow> = sqlx::query(&sql).fetch_all(&pool).await?;
        Ok(rows.iter().map(|r| r.get::<String, _>("enumlabel")).collect())
    }

    async fn get_create_table_sql(&self, database: &str, table: &str) -> anyhow::Result<String> {
        let pool = self.pool_for_db(database).await?;
        let (schema, tbl) = Self::parse_table_ref(table);
        // Get columns
        let col_sql = format!(
            "SELECT column_name, data_type, is_nullable, column_default, character_maximum_length \
             FROM information_schema.columns \
             WHERE table_name = '{}' AND table_schema = '{}' \
             ORDER BY ordinal_position",
            tbl, schema
        );
        let col_rows: Vec<PgRow> = sqlx::query(&col_sql).fetch_all(&pool).await?;

        let mut ddl = format!("CREATE TABLE \"{}\".\"{}\" (\n", schema, tbl);
        let mut col_defs = Vec::new();
        for row in &col_rows {
            let name: String = row.get("column_name");
            let dtype: String = row.get("data_type");
            let nullable: String = row.get("is_nullable");
            let default: Option<String> = row.try_get("column_default").ok();
            let mut col_def = format!("  \"{}\" {}", name, dtype);
            if nullable == "NO" { col_def.push_str(" NOT NULL"); }
            if let Some(def) = default { col_def.push_str(&format!(" DEFAULT {}", def)); }
            col_defs.push(col_def);
        }
        // Get primary key
        let pk_sql = format!(
            "SELECT kcu.column_name \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
             WHERE tc.table_name = '{}' AND tc.table_schema = '{}' AND tc.constraint_type = 'PRIMARY KEY'",
            tbl, schema
        );
        let pk_rows: Vec<PgRow> = sqlx::query(&pk_sql).fetch_all(&pool).await?;
        if !pk_rows.is_empty() {
            let pk_cols: Vec<String> = pk_rows.iter().map(|r| format!("\"{}\"", r.get::<String, _>("column_name"))).collect();
            col_defs.push(format!("  PRIMARY KEY ({})", pk_cols.join(", ")));
        }
        ddl.push_str(&col_defs.join(",\n"));
        ddl.push_str("\n);");
        Ok(ddl)
    }

    fn db_type(&self) -> DbType {
        DbType::PostgreSQL
    }
}
