use async_trait::async_trait;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions, SqliteRow};
use sqlx::{Column, Row, SqliteConnection, TypeInfo, ValueRef};
use std::time::Instant;

use super::{bytes_to_hex, f64_to_json, ColumnInfo, ConnectionConfig, DatabaseDriver, DbType, ForeignKeyInfo, IndexInfo, QueryResult, StatementResult, TableInfo, TriggerInfo, ViewInfo};

/// Decode the `i`-th column of a SQLite row into a JSON value, dispatching on the
/// value's actual storage class. SQLite is dynamically typed — a column's declared
/// type is only an affinity hint and each value may be any class — so we read the
/// real per-value type (INTEGER / REAL / TEXT / BLOB) rather than coercing (e.g.
/// `try_get::<i64>` on a TEXT value would silently coerce to 0).
fn sqlite_value_to_json(row: &SqliteRow, i: usize) -> serde_json::Value {
    use serde_json::Value as J;

    let storage = match row.try_get_raw(i) {
        Ok(raw) if !raw.is_null() => raw.type_info().name().to_string(),
        _ => return J::Null,
    };

    match storage.as_str() {
        "INTEGER" => row
            .try_get::<i64, _>(i)
            .map(|v| J::Number(v.into()))
            .unwrap_or(J::Null),
        "REAL" => row.try_get::<f64, _>(i).map(f64_to_json).unwrap_or(J::Null),
        "BLOB" => row
            .try_get::<Vec<u8>, _>(i)
            .map(|b| J::String(bytes_to_hex(&b, "0x")))
            .unwrap_or(J::Null),
        // TEXT and any fallback: decode as text — dates, decimals, JSON, etc. are
        // all stored as TEXT in SQLite and should display verbatim.
        _ => row.try_get::<String, _>(i).map(J::String).unwrap_or(J::Null),
    }
}

async fn run_sqlite_query(conn: &mut SqliteConnection, sql: &str) -> anyhow::Result<QueryResult> {
    let start = Instant::now();
    let trimmed = sql.trim().to_uppercase();

    if trimmed.starts_with("SELECT")
        || trimmed.starts_with("PRAGMA")
        || trimmed.starts_with("EXPLAIN")
        || trimmed.starts_with("WITH ")
        || trimmed.starts_with("VALUES")
    {
        let rows: Vec<SqliteRow> = sqlx::query(sql).fetch_all(&mut *conn).await?;
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

        let data_rows: Vec<Vec<serde_json::Value>> = rows
            .iter()
            .map(|row| {
                (0..row.columns().len())
                    .map(|i| sqlite_value_to_json(row, i))
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

pub struct SqliteDriver {
    config: ConnectionConfig,
    pool: Option<SqlitePool>,
}

impl SqliteDriver {
    pub fn new(config: ConnectionConfig) -> Self {
        Self { config, pool: None }
    }

    fn connection_string(&self) -> String {
        format!("sqlite:{}", self.config.host)
    }

    fn pool(&self) -> anyhow::Result<&SqlitePool> {
        self.pool.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))
    }
}

#[async_trait]
impl DatabaseDriver for SqliteDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&self.connection_string())
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
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&self.connection_string())
            .await?;
        let _: (i32,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await?;
        pool.close().await;
        Ok(true)
    }

    async fn get_databases(&self) -> anyhow::Result<Vec<String>> {
        Ok(vec!["main".to_string()])
    }

    async fn get_tables(&self, _database: &str) -> anyhow::Result<Vec<TableInfo>> {
        let pool = self.pool()?;
        let rows: Vec<SqliteRow> = sqlx::query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .fetch_all(pool)
        .await?;
        let tables = rows
            .iter()
            .map(|row| TableInfo {
                name: row.get::<String, _>("name"),
                schema: None,
                row_count: None,
                comment: None,
            })
            .collect();
        Ok(tables)
    }

    async fn get_columns(&self, _database: &str, table: &str) -> anyhow::Result<Vec<ColumnInfo>> {
        let pool = self.pool()?;
        let sql = format!("PRAGMA table_info('{}')", table);
        let rows: Vec<SqliteRow> = sqlx::query(&sql).fetch_all(pool).await?;
        let columns = rows
            .iter()
            .map(|row| ColumnInfo {
                name: row.get::<String, _>("name"),
                data_type: row.get::<String, _>("type"),
                nullable: row.get::<i32, _>("notnull") == 0,
                is_primary_key: row.get::<i32, _>("pk") > 0,
                default_value: row.try_get::<String, _>("dflt_value").ok(),
                comment: None,
            })
            .collect();
        Ok(columns)
    }

    async fn get_indexes(&self, _database: &str, table: &str) -> anyhow::Result<Vec<IndexInfo>> {
        let pool = self.pool()?;
        let sql = format!("PRAGMA index_list('{}')", table);
        let rows: Vec<SqliteRow> = sqlx::query(&sql).fetch_all(pool).await?;
        let mut indexes = Vec::new();
        for row in &rows {
            let name: String = row.get("name");
            let unique: i32 = row.get("unique");
            let info_sql = format!("PRAGMA index_info('{}')", name);
            let info_rows: Vec<SqliteRow> = sqlx::query(&info_sql).fetch_all(pool).await?;
            let columns: Vec<String> = info_rows.iter().map(|r| r.get::<String, _>("name")).collect();
            indexes.push(IndexInfo {
                name,
                columns,
                is_unique: unique == 1,
                is_primary: false,
            });
        }
        Ok(indexes)
    }

    async fn execute_query(&self, _database: &str, sql: &str) -> anyhow::Result<QueryResult> {
        let pool = self.pool()?;
        let mut conn = pool.acquire().await?;
        run_sqlite_query(&mut *conn, sql).await
    }

    async fn execute_statements(
        &self,
        _database: &str,
        statements: Vec<String>,
    ) -> anyhow::Result<Vec<StatementResult>> {
        let pool = self.pool()?;
        let mut conn = pool.acquire().await?;
        let mut results = Vec::with_capacity(statements.len());
        for sql in statements {
            let start = Instant::now();
            match run_sqlite_query(&mut *conn, &sql).await {
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
        let offset = (page - 1) * page_size;
        let sql = format!(
            "SELECT * FROM \"{}\" LIMIT {} OFFSET {}",
            table, page_size, offset
        );
        self.execute_query(database, &sql).await
    }

    async fn get_views(&self, _database: &str) -> anyhow::Result<Vec<ViewInfo>> {
        let pool = self.pool()?;
        let rows: Vec<SqliteRow> = sqlx::query(
            "SELECT name, sql FROM sqlite_master WHERE type = 'view'"
        )
        .fetch_all(pool)
        .await?;
        let views = rows
            .iter()
            .map(|row| ViewInfo {
                name: row.get::<String, _>("name"),
                definition: row.try_get::<String, _>("sql").ok(),
            })
            .collect();
        Ok(views)
    }

    async fn get_triggers(&self, _database: &str) -> anyhow::Result<Vec<TriggerInfo>> {
        let pool = self.pool()?;
        let rows: Vec<SqliteRow> = sqlx::query(
            "SELECT name, tbl_name, sql FROM sqlite_master WHERE type = 'trigger'"
        )
        .fetch_all(pool)
        .await?;
        let triggers = rows
            .iter()
            .map(|row| {
                let name: String = row.get("name");
                let table: String = row.get("tbl_name");
                let sql: String = row.try_get::<String, _>("sql").unwrap_or_default();
                let upper = sql.to_uppercase();
                let timing = if upper.contains("BEFORE") {
                    "BEFORE"
                } else if upper.contains("AFTER") {
                    "AFTER"
                } else if upper.contains("INSTEAD OF") {
                    "INSTEAD OF"
                } else {
                    "UNKNOWN"
                };
                let event = if upper.contains("INSERT") {
                    "INSERT"
                } else if upper.contains("UPDATE") {
                    "UPDATE"
                } else if upper.contains("DELETE") {
                    "DELETE"
                } else {
                    "UNKNOWN"
                };
                TriggerInfo {
                    name,
                    event: event.to_string(),
                    table,
                    timing: timing.to_string(),
                }
            })
            .collect();
        Ok(triggers)
    }

    async fn get_foreign_keys(&self, _database: &str, table: &str) -> anyhow::Result<Vec<ForeignKeyInfo>> {
        let pool = self.pool()?;
        let sql = format!("PRAGMA foreign_key_list('{}')", table);
        let rows: Vec<SqliteRow> = sqlx::query(&sql).fetch_all(pool).await?;
        let mut fk_map: std::collections::HashMap<i32, ForeignKeyInfo> = std::collections::HashMap::new();
        for row in &rows {
            let id: i32 = row.get("id");
            let ref_table: String = row.get("table");
            let from_col: String = row.get("from");
            let to_col: String = row.get("to");
            let entry = fk_map.entry(id).or_insert_with(|| ForeignKeyInfo {
                name: format!("fk_{}_{}", table, id),
                from_table: table.to_string(),
                from_columns: vec![],
                to_table: ref_table.clone(),
                to_columns: vec![],
            });
            entry.from_columns.push(from_col);
            entry.to_columns.push(to_col);
        }
        Ok(fk_map.into_values().collect())
    }

    async fn get_create_table_sql(&self, _database: &str, table: &str) -> anyhow::Result<String> {
        let pool = self.pool()?;
        let sql = format!("SELECT sql FROM sqlite_master WHERE type='table' AND name='{}'", table);
        let rows: Vec<SqliteRow> = sqlx::query(&sql).fetch_all(pool).await?;
        rows.first().and_then(|r| r.try_get::<String, _>("sql").ok())
            .ok_or_else(|| anyhow::anyhow!("Table not found"))
    }

    fn db_type(&self) -> DbType {
        DbType::SQLite
    }
}
