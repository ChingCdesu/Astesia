use async_trait::async_trait;
use sqlx::mysql::{MySqlPool, MySqlPoolOptions, MySqlRow};
use sqlx::{Column, Row};
use std::time::Instant;

use super::{ColumnInfo, ConnectionConfig, DatabaseDriver, DbType, ForeignKeyInfo, FunctionInfo, IndexInfo, ProcedureInfo, QueryResult, TableInfo, TriggerInfo, UserInfo, ViewInfo};

pub struct MySqlDriver {
    config: ConnectionConfig,
    pool: Option<MySqlPool>,
}

impl MySqlDriver {
    pub fn new(config: ConnectionConfig) -> Self {
        Self { config, pool: None }
    }

    fn connection_string(&self) -> String {
        let db = self.config.database.as_deref().unwrap_or("");
        format!(
            "mysql://{}:{}@{}:{}/{}",
            self.config.username, self.config.password, self.config.host, self.config.port, db
        )
    }

    fn pool(&self) -> anyhow::Result<&MySqlPool> {
        self.pool.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))
    }
}

#[async_trait]
impl DatabaseDriver for MySqlDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let pool = MySqlPoolOptions::new()
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
        let url = self.connection_string();
        let pool = MySqlPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await?;
        let _: (i32,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await?;
        pool.close().await;
        Ok(true)
    }

    async fn get_databases(&self) -> anyhow::Result<Vec<String>> {
        let pool = self.pool()?;
        let rows: Vec<(String,)> = sqlx::query_as("SHOW DATABASES")
            .fetch_all(pool)
            .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn get_tables(&self, database: &str) -> anyhow::Result<Vec<TableInfo>> {
        let pool = self.pool()?;
        let sql = format!(
            "SELECT TABLE_NAME, TABLE_ROWS, TABLE_COMMENT FROM information_schema.TABLES WHERE TABLE_SCHEMA = '{}'",
            database
        );
        let rows: Vec<MySqlRow> = sqlx::query(&sql).fetch_all(pool).await?;
        let tables = rows
            .iter()
            .map(|row| {
                TableInfo {
                    name: row.get::<String, _>("TABLE_NAME"),
                    schema: Some(database.to_string()),
                    row_count: row.try_get::<i64, _>("TABLE_ROWS").ok(),
                    comment: row.try_get::<String, _>("TABLE_COMMENT").ok(),
                }
            })
            .collect();
        Ok(tables)
    }

    async fn get_columns(&self, database: &str, table: &str) -> anyhow::Result<Vec<ColumnInfo>> {
        let pool = self.pool()?;
        let sql = format!(
            "SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE, COLUMN_KEY, COLUMN_DEFAULT, COLUMN_COMMENT \
             FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = '{}' AND TABLE_NAME = '{}' ORDER BY ORDINAL_POSITION",
            database, table
        );
        let rows: Vec<MySqlRow> = sqlx::query(&sql).fetch_all(pool).await?;
        let columns = rows
            .iter()
            .map(|row| {
                ColumnInfo {
                    name: row.get::<String, _>("COLUMN_NAME"),
                    data_type: row.get::<String, _>("DATA_TYPE"),
                    nullable: row.get::<String, _>("IS_NULLABLE") == "YES",
                    is_primary_key: row.get::<String, _>("COLUMN_KEY") == "PRI",
                    default_value: row.try_get::<String, _>("COLUMN_DEFAULT").ok(),
                    comment: row.try_get::<String, _>("COLUMN_COMMENT").ok(),
                }
            })
            .collect();
        Ok(columns)
    }

    async fn get_indexes(&self, database: &str, table: &str) -> anyhow::Result<Vec<IndexInfo>> {
        let pool = self.pool()?;
        let sql = format!(
            "SELECT INDEX_NAME, COLUMN_NAME, NON_UNIQUE \
             FROM information_schema.STATISTICS \
             WHERE TABLE_SCHEMA = '{}' AND TABLE_NAME = '{}' \
             ORDER BY INDEX_NAME, SEQ_IN_INDEX",
            database, table
        );
        let rows: Vec<MySqlRow> = sqlx::query(&sql).fetch_all(pool).await?;
        let mut indexes: std::collections::HashMap<String, IndexInfo> = std::collections::HashMap::new();
        for row in &rows {
            let name: String = row.get("INDEX_NAME");
            let column: String = row.get("COLUMN_NAME");
            let non_unique: i32 = row.get("NON_UNIQUE");
            let entry = indexes.entry(name.clone()).or_insert_with(|| IndexInfo {
                name: name.clone(),
                columns: vec![],
                is_unique: non_unique == 0,
                is_primary: name == "PRIMARY",
            });
            entry.columns.push(column);
        }
        Ok(indexes.into_values().collect())
    }

    async fn execute_query(&self, database: &str, sql: &str) -> anyhow::Result<QueryResult> {
        let pool = self.pool()?;
        let _ = sqlx::query(&format!("USE `{}`", database))
            .execute(pool)
            .await;

        let start = Instant::now();
        let trimmed = sql.trim().to_uppercase();
        if trimmed.starts_with("SELECT") || trimmed.starts_with("SHOW") || trimmed.starts_with("DESCRIBE") || trimmed.starts_with("EXPLAIN") {
            let rows: Vec<MySqlRow> = sqlx::query(sql).fetch_all(pool).await?;
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
            "SELECT * FROM `{}`.`{}` LIMIT {} OFFSET {}",
            database, table, page_size, offset
        );
        self.execute_query(database, &sql).await
    }

    async fn get_views(&self, database: &str) -> anyhow::Result<Vec<ViewInfo>> {
        let pool = self.pool()?;
        let sql = format!(
            "SELECT TABLE_NAME, VIEW_DEFINITION FROM information_schema.VIEWS WHERE TABLE_SCHEMA = '{}'",
            database
        );
        let rows: Vec<MySqlRow> = sqlx::query(&sql).fetch_all(pool).await?;
        let views = rows
            .iter()
            .map(|row| ViewInfo {
                name: row.get::<String, _>("TABLE_NAME"),
                definition: row.try_get::<String, _>("VIEW_DEFINITION").ok(),
            })
            .collect();
        Ok(views)
    }

    async fn get_functions(&self, database: &str) -> anyhow::Result<Vec<FunctionInfo>> {
        let pool = self.pool()?;
        let sql = format!(
            "SELECT ROUTINE_NAME, DTD_IDENTIFIER, ROUTINE_DEFINITION FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = '{}' AND ROUTINE_TYPE = 'FUNCTION'",
            database
        );
        let rows: Vec<MySqlRow> = sqlx::query(&sql).fetch_all(pool).await?;
        let functions = rows
            .iter()
            .map(|row| FunctionInfo {
                name: row.get::<String, _>("ROUTINE_NAME"),
                language: Some("SQL".to_string()),
                return_type: row.try_get::<String, _>("DTD_IDENTIFIER").ok(),
                definition: row.try_get::<String, _>("ROUTINE_DEFINITION").ok(),
            })
            .collect();
        Ok(functions)
    }

    async fn get_procedures(&self, database: &str) -> anyhow::Result<Vec<ProcedureInfo>> {
        let pool = self.pool()?;
        let sql = format!(
            "SELECT ROUTINE_NAME, ROUTINE_DEFINITION FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = '{}' AND ROUTINE_TYPE = 'PROCEDURE'",
            database
        );
        let rows: Vec<MySqlRow> = sqlx::query(&sql).fetch_all(pool).await?;
        let procedures = rows
            .iter()
            .map(|row| ProcedureInfo {
                name: row.get::<String, _>("ROUTINE_NAME"),
                language: Some("SQL".to_string()),
                definition: row.try_get::<String, _>("ROUTINE_DEFINITION").ok(),
            })
            .collect();
        Ok(procedures)
    }

    async fn get_triggers(&self, database: &str) -> anyhow::Result<Vec<TriggerInfo>> {
        let pool = self.pool()?;
        let sql = format!(
            "SELECT TRIGGER_NAME, EVENT_MANIPULATION, EVENT_OBJECT_TABLE, ACTION_TIMING FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA = '{}'",
            database
        );
        let rows: Vec<MySqlRow> = sqlx::query(&sql).fetch_all(pool).await?;
        let triggers = rows
            .iter()
            .map(|row| TriggerInfo {
                name: row.get::<String, _>("TRIGGER_NAME"),
                event: row.get::<String, _>("EVENT_MANIPULATION"),
                table: row.get::<String, _>("EVENT_OBJECT_TABLE"),
                timing: row.get::<String, _>("ACTION_TIMING"),
            })
            .collect();
        Ok(triggers)
    }

    async fn get_foreign_keys(&self, database: &str, table: &str) -> anyhow::Result<Vec<ForeignKeyInfo>> {
        let pool = self.pool()?;
        let sql = format!(
            "SELECT CONSTRAINT_NAME, TABLE_NAME, COLUMN_NAME, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
             FROM information_schema.KEY_COLUMN_USAGE \
             WHERE TABLE_SCHEMA = '{}' AND TABLE_NAME = '{}' AND REFERENCED_TABLE_NAME IS NOT NULL \
             ORDER BY CONSTRAINT_NAME, ORDINAL_POSITION",
            database, table
        );
        let rows: Vec<MySqlRow> = sqlx::query(&sql).fetch_all(pool).await?;
        let mut fk_map: std::collections::HashMap<String, ForeignKeyInfo> = std::collections::HashMap::new();
        for row in &rows {
            let name: String = row.get("CONSTRAINT_NAME");
            let from_col: String = row.get("COLUMN_NAME");
            let to_table: String = row.get("REFERENCED_TABLE_NAME");
            let to_col: String = row.get("REFERENCED_COLUMN_NAME");
            let entry = fk_map.entry(name.clone()).or_insert_with(|| ForeignKeyInfo {
                name: name.clone(),
                from_table: table.to_string(),
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
        let rows: Vec<MySqlRow> = sqlx::query("SELECT User, Host FROM mysql.user")
            .fetch_all(pool)
            .await?;
        let users = rows
            .iter()
            .map(|row| UserInfo {
                name: row.get::<String, _>("User"),
                host: row.try_get::<String, _>("Host").ok(),
            })
            .collect();
        Ok(users)
    }

    async fn get_create_table_sql(&self, database: &str, table: &str) -> anyhow::Result<String> {
        let pool = self.pool()?;
        let sql = format!("SHOW CREATE TABLE `{}`.`{}`", database, table);
        let rows: Vec<MySqlRow> = sqlx::query(&sql).fetch_all(pool).await?;
        if let Some(row) = rows.first() {
            Ok(row.try_get::<String, _>(1).unwrap_or_default())
        } else {
            Err(anyhow::anyhow!("Table not found"))
        }
    }

    fn db_type(&self) -> DbType {
        DbType::MySQL
    }
}
