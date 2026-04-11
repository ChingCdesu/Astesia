use async_trait::async_trait;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions, SqliteRow};
use sqlx::{Column, Row};
use std::time::Instant;

use super::{ColumnInfo, ConnectionConfig, DatabaseDriver, DbType, ForeignKeyInfo, IndexInfo, QueryResult, TableInfo, TriggerInfo, ViewInfo};

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
        let start = Instant::now();
        let trimmed = sql.trim().to_uppercase();

        if trimmed.starts_with("SELECT") || trimmed.starts_with("PRAGMA") || trimmed.starts_with("EXPLAIN") {
            let rows: Vec<SqliteRow> = sqlx::query(sql).fetch_all(pool).await?;
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
                            row.try_get::<String, _>(i)
                                .map(serde_json::Value::String)
                                .or_else(|_| row.try_get::<i64, _>(i).map(|v| serde_json::Value::Number(v.into())))
                                .or_else(|_| row.try_get::<f64, _>(i).map(|v| {
                                    serde_json::Number::from_f64(v)
                                        .map(serde_json::Value::Number)
                                        .unwrap_or(serde_json::Value::Null)
                                }))
                                .or_else(|_| row.try_get::<bool, _>(i).map(serde_json::Value::Bool))
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
            let result = sqlx::query(sql).execute(pool).await?;
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
