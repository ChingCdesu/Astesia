use async_trait::async_trait;
use std::time::Instant;
use tiberius::{AuthMethod, Client, Config};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use super::{ColumnInfo, ConnectionConfig, DatabaseDriver, DbType, ForeignKeyInfo, FunctionInfo, IndexInfo, ProcedureInfo, QueryResult, TableInfo, TriggerInfo, UserInfo, ViewInfo};

pub struct SqlServerDriver {
    config: ConnectionConfig,
    client: Option<Mutex<Client<tokio_util::compat::Compat<TcpStream>>>>,
}

impl SqlServerDriver {
    pub fn new(config: ConnectionConfig) -> Self {
        Self { config, client: None }
    }

    fn tiberius_config(&self) -> anyhow::Result<Config> {
        let mut config = Config::new();
        config.host(&self.config.host);
        config.port(self.config.port);
        config.authentication(AuthMethod::sql_server(&self.config.username, &self.config.password));
        config.trust_cert();
        Ok(config)
    }

    async fn create_client(&self) -> anyhow::Result<Client<tokio_util::compat::Compat<TcpStream>>> {
        let config = self.tiberius_config()?;
        let tcp = TcpStream::connect(config.get_addr()).await?;
        tcp.set_nodelay(true)?;
        let client = Client::connect(config, tcp.compat_write()).await?;
        Ok(client)
    }
}

#[async_trait]
impl DatabaseDriver for SqlServerDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let client = self.create_client().await?;
        self.client = Some(Mutex::new(client));
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        self.client = None;
        Ok(())
    }

    async fn test_connection(&self) -> anyhow::Result<bool> {
        let mut client = self.create_client().await?;
        let _stream = client.query("SELECT 1", &[]).await?;
        Ok(true)
    }

    async fn get_databases(&self) -> anyhow::Result<Vec<String>> {
        let mutex = self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let stream = client.query("SELECT name FROM sys.databases ORDER BY name", &[]).await?;
        let rows = stream.into_first_result().await?;
        let databases: Vec<String> = rows
            .iter()
            .filter_map(|row| row.try_get::<&str, _>(0).ok().flatten().map(|s| s.to_string()))
            .collect();
        Ok(databases)
    }

    async fn get_tables(&self, database: &str) -> anyhow::Result<Vec<TableInfo>> {
        let mutex = self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = format!(
            "USE [{}]; SELECT TABLE_NAME, TABLE_SCHEMA FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME",
            database
        );
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let tables: Vec<TableInfo> = rows
            .iter()
            .map(|row| TableInfo {
                name: row.try_get::<&str, _>(0).ok().flatten().unwrap_or("").to_string(),
                schema: row.try_get::<&str, _>(1).ok().flatten().map(|s| s.to_string()),
                row_count: None,
                comment: None,
            })
            .collect();
        Ok(tables)
    }

    async fn get_columns(&self, database: &str, table: &str) -> anyhow::Result<Vec<ColumnInfo>> {
        let mutex = self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = format!(
            "USE [{}]; SELECT c.COLUMN_NAME, c.DATA_TYPE, c.IS_NULLABLE, c.COLUMN_DEFAULT, \
             CASE WHEN pk.COLUMN_NAME IS NOT NULL THEN 1 ELSE 0 END as IS_PK \
             FROM INFORMATION_SCHEMA.COLUMNS c \
             LEFT JOIN (SELECT ku.TABLE_NAME, ku.COLUMN_NAME FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc \
             JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE ku ON tc.CONSTRAINT_NAME = ku.CONSTRAINT_NAME \
             WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY') pk ON c.TABLE_NAME = pk.TABLE_NAME AND c.COLUMN_NAME = pk.COLUMN_NAME \
             WHERE c.TABLE_NAME = '{}' ORDER BY c.ORDINAL_POSITION",
            database, table
        );
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let columns: Vec<ColumnInfo> = rows
            .iter()
            .map(|row| ColumnInfo {
                name: row.try_get::<&str, _>(0).ok().flatten().unwrap_or("").to_string(),
                data_type: row.try_get::<&str, _>(1).ok().flatten().unwrap_or("").to_string(),
                nullable: row.try_get::<&str, _>(2).ok().flatten().unwrap_or("YES") == "YES",
                is_primary_key: row.try_get::<i32, _>(4).ok().flatten().unwrap_or(0) == 1,
                default_value: row.try_get::<&str, _>(3).ok().flatten().map(|s| s.to_string()),
                comment: None,
            })
            .collect();
        Ok(columns)
    }

    async fn get_indexes(&self, database: &str, table: &str) -> anyhow::Result<Vec<IndexInfo>> {
        let mutex = self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = format!(
            "USE [{}]; SELECT i.name AS index_name, COL_NAME(ic.object_id, ic.column_id) AS column_name, \
             i.is_unique, i.is_primary_key \
             FROM sys.indexes i \
             JOIN sys.index_columns ic ON i.object_id = ic.object_id AND i.index_id = ic.index_id \
             WHERE OBJECT_NAME(i.object_id) = '{}' ORDER BY i.name, ic.key_ordinal",
            database, table
        );
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let mut indexes: std::collections::HashMap<String, IndexInfo> = std::collections::HashMap::new();
        for row in &rows {
            let name = row.try_get::<&str, _>(0).ok().flatten().unwrap_or("").to_string();
            let column = row.try_get::<&str, _>(1).ok().flatten().unwrap_or("").to_string();
            let is_unique = row.try_get::<bool, _>(2).ok().flatten().unwrap_or(false);
            let is_primary = row.try_get::<bool, _>(3).ok().flatten().unwrap_or(false);
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
        let mutex = self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;

        let full_sql = format!("USE [{}]; {}", database, sql);
        let start = Instant::now();
        let stream = client.query(full_sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
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
                data_type: format!("{:?}", c.column_type()),
                nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
            })
            .collect();

        let data_rows: Vec<Vec<serde_json::Value>> = rows
            .iter()
            .map(|row| {
                (0..columns.len())
                    .map(|i| {
                        if let Some(val) = row.try_get::<&str, _>(i).ok().flatten() {
                            serde_json::Value::String(val.to_string())
                        } else if let Some(val) = row.try_get::<i32, _>(i).ok().flatten() {
                            serde_json::Value::Number(val.into())
                        } else if let Some(val) = row.try_get::<i64, _>(i).ok().flatten() {
                            serde_json::Value::Number(val.into())
                        } else if let Some(val) = row.try_get::<f64, _>(i).ok().flatten() {
                            serde_json::Number::from_f64(val)
                                .map(serde_json::Value::Number)
                                .unwrap_or(serde_json::Value::Null)
                        } else if let Some(val) = row.try_get::<bool, _>(i).ok().flatten() {
                            serde_json::Value::Bool(val)
                        } else {
                            serde_json::Value::Null
                        }
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
            "SELECT * FROM [{}] ORDER BY (SELECT NULL) OFFSET {} ROWS FETCH NEXT {} ROWS ONLY",
            table, offset, page_size
        );
        self.execute_query(database, &sql).await
    }

    async fn get_views(&self, database: &str) -> anyhow::Result<Vec<ViewInfo>> {
        let mutex = self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = format!(
            "USE [{}]; SELECT name, OBJECT_DEFINITION(object_id) AS definition FROM sys.views",
            database
        );
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let views: Vec<ViewInfo> = rows
            .iter()
            .map(|row| ViewInfo {
                name: row.try_get::<&str, _>(0).ok().flatten().unwrap_or("").to_string(),
                definition: row.try_get::<&str, _>(1).ok().flatten().map(|s| s.to_string()),
            })
            .collect();
        Ok(views)
    }

    async fn get_functions(&self, database: &str) -> anyhow::Result<Vec<FunctionInfo>> {
        let mutex = self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = format!(
            "USE [{}]; SELECT name, OBJECT_DEFINITION(object_id) AS definition, type_desc FROM sys.objects WHERE type IN ('FN', 'IF', 'TF')",
            database
        );
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let functions: Vec<FunctionInfo> = rows
            .iter()
            .map(|row| FunctionInfo {
                name: row.try_get::<&str, _>(0).ok().flatten().unwrap_or("").to_string(),
                language: Some("T-SQL".to_string()),
                return_type: row.try_get::<&str, _>(2).ok().flatten().map(|s| s.to_string()),
                definition: row.try_get::<&str, _>(1).ok().flatten().map(|s| s.to_string()),
            })
            .collect();
        Ok(functions)
    }

    async fn get_procedures(&self, database: &str) -> anyhow::Result<Vec<ProcedureInfo>> {
        let mutex = self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = format!(
            "USE [{}]; SELECT name, OBJECT_DEFINITION(object_id) AS definition FROM sys.procedures",
            database
        );
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let procedures: Vec<ProcedureInfo> = rows
            .iter()
            .map(|row| ProcedureInfo {
                name: row.try_get::<&str, _>(0).ok().flatten().unwrap_or("").to_string(),
                language: Some("T-SQL".to_string()),
                definition: row.try_get::<&str, _>(1).ok().flatten().map(|s| s.to_string()),
            })
            .collect();
        Ok(procedures)
    }

    async fn get_triggers(&self, database: &str) -> anyhow::Result<Vec<TriggerInfo>> {
        let mutex = self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = format!(
            "USE [{}]; SELECT t.name, te.type_desc, OBJECT_NAME(t.parent_id) AS table_name \
             FROM sys.triggers t \
             JOIN sys.trigger_events te ON t.object_id = te.object_id",
            database
        );
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let triggers: Vec<TriggerInfo> = rows
            .iter()
            .map(|row| {
                let name = row.try_get::<&str, _>(0).ok().flatten().unwrap_or("").to_string();
                let event = row.try_get::<&str, _>(1).ok().flatten().unwrap_or("").to_string();
                let table = row.try_get::<&str, _>(2).ok().flatten().unwrap_or("").to_string();
                TriggerInfo {
                    name,
                    event: event.clone(),
                    table,
                    timing: if event.contains("AFTER") { "AFTER".to_string() } else { "INSTEAD OF".to_string() },
                }
            })
            .collect();
        Ok(triggers)
    }

    async fn get_foreign_keys(&self, database: &str, table: &str) -> anyhow::Result<Vec<ForeignKeyInfo>> {
        let mutex = self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = format!(
            "USE [{}]; SELECT fk.name AS constraint_name, \
             COL_NAME(fkc.parent_object_id, fkc.parent_column_id) AS from_column, \
             OBJECT_NAME(fkc.referenced_object_id) AS to_table, \
             COL_NAME(fkc.referenced_object_id, fkc.referenced_column_id) AS to_column \
             FROM sys.foreign_keys fk \
             JOIN sys.foreign_key_columns fkc ON fk.object_id = fkc.constraint_object_id \
             WHERE OBJECT_NAME(fk.parent_object_id) = '{}'",
            database, table
        );
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let mut fk_map: std::collections::HashMap<String, ForeignKeyInfo> = std::collections::HashMap::new();
        for row in &rows {
            let name = row.try_get::<&str, _>(0).ok().flatten().unwrap_or("").to_string();
            let from_col = row.try_get::<&str, _>(1).ok().flatten().unwrap_or("").to_string();
            let to_table = row.try_get::<&str, _>(2).ok().flatten().unwrap_or("").to_string();
            let to_col = row.try_get::<&str, _>(3).ok().flatten().unwrap_or("").to_string();
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
        let mutex = self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let stream = client.query(
            "SELECT name FROM sys.database_principals WHERE type IN ('S','U')",
            &[],
        ).await?;
        let rows = stream.into_first_result().await?;
        let users: Vec<UserInfo> = rows
            .iter()
            .map(|row| UserInfo {
                name: row.try_get::<&str, _>(0).ok().flatten().unwrap_or("").to_string(),
                host: None,
            })
            .collect();
        Ok(users)
    }

    async fn get_create_table_sql(&self, database: &str, table: &str) -> anyhow::Result<String> {
        let mutex = self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = format!(
            "USE [{}]; SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE, COLUMN_DEFAULT, CHARACTER_MAXIMUM_LENGTH FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = '{}' ORDER BY ORDINAL_POSITION",
            database, table
        );
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let mut ddl = format!("CREATE TABLE [{}] (\n", table);
        let col_defs: Vec<String> = rows.iter().map(|row| {
            let name = row.try_get::<&str, _>(0).ok().flatten().unwrap_or("");
            let dtype = row.try_get::<&str, _>(1).ok().flatten().unwrap_or("");
            let nullable = row.try_get::<&str, _>(2).ok().flatten().unwrap_or("YES");
            let null_str = if nullable == "NO" { " NOT NULL" } else { "" };
            format!("  [{}] {}{}", name, dtype, null_str)
        }).collect();
        ddl.push_str(&col_defs.join(",\n"));
        ddl.push_str("\n);");
        Ok(ddl)
    }

    fn db_type(&self) -> DbType {
        DbType::SQLServer
    }
}
