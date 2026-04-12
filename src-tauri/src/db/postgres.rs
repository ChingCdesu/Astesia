use async_trait::async_trait;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::{Column, Row};
use std::time::Instant;

use super::{ColumnInfo, ConnectionConfig, DatabaseDriver, DbType, ForeignKeyInfo, FunctionInfo, IndexInfo, ProcedureInfo, QueryResult, TableInfo, TriggerInfo, UserInfo, ViewInfo};

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
        let start = Instant::now();
        let trimmed = sql.trim().to_uppercase();

        if trimmed.starts_with("SELECT") || trimmed.starts_with("SHOW") || trimmed.starts_with("EXPLAIN") {
            let rows: Vec<PgRow> = sqlx::query(sql).fetch_all(&pool).await?;
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
                    data_type: format!("{:?}", c.type_info()),
                    nullable: true,
                    is_primary_key: false,
                    default_value: None,
                    comment: None,
                })
                .collect();

            let data_rows: Vec<Vec<serde_json::Value>> = rows
                .iter()
                .map(|row| {
                    row.columns()
                        .iter()
                        .enumerate()
                        .map(|(i, _)| {
                            // Try JSON/JSONB first — returns as parsed serde_json::Value
                            row.try_get::<serde_json::Value, _>(i)
                                .ok()
                                .filter(|v| !v.is_string()) // avoid capturing plain strings as JSON
                                .map(Ok)
                                .unwrap_or_else(||
                                    row.try_get::<NaiveDateTime, _>(i)
                                        .map(|v| serde_json::Value::String(v.to_string()))
                                        .or_else(|_| row.try_get::<NaiveDate, _>(i)
                                            .map(|v| serde_json::Value::String(v.to_string())))
                                        .or_else(|_| row.try_get::<NaiveTime, _>(i)
                                            .map(|v| serde_json::Value::String(v.to_string())))
                                        .or_else(|_| row.try_get::<String, _>(i)
                                            .map(serde_json::Value::String))
                                        .or_else(|_| row.try_get::<i64, _>(i).map(|v| serde_json::Value::Number(v.into())))
                                        .or_else(|_| row.try_get::<i32, _>(i).map(|v| serde_json::Value::Number(v.into())))
                                        .or_else(|_| row.try_get::<f64, _>(i).map(|v| {
                                            serde_json::Number::from_f64(v)
                                                .map(serde_json::Value::Number)
                                                .unwrap_or(serde_json::Value::Null)
                                        }))
                                        .or_else(|_| row.try_get::<bool, _>(i).map(serde_json::Value::Bool))
                                )
                                .unwrap_or(serde_json::Value::Null)
                        })
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
            let result = sqlx::query(sql).execute(&pool).await?;
            let elapsed = start.elapsed().as_millis() as u64;
            Ok(QueryResult {
                affected_rows: result.rows_affected(),
                execution_time_ms: elapsed,
                ..Default::default()
            })
        }
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
