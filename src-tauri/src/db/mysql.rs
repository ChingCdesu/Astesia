use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions, MySqlRow};
use sqlx::types::BigDecimal;
use sqlx::{Column, Executor, MySqlConnection, Row, TypeInfo};
use std::time::Instant;

use super::{
    bytes_to_hex, f32_to_json, f64_to_json, ColumnInfo, ConnectionConfig, DatabaseDriver, DbType,
    ForeignKeyInfo, FunctionInfo, IndexInfo, ProcedureInfo, QueryResult, SqlDialect,
    StatementResult, TableInfo, TableRef, TriggerInfo, UserInfo, ViewInfo,
};

/// Decode the `i`-th column of a MySQL row into a JSON value, dispatching on the
/// (upper-cased) MySQL type name. Covers all built-in scalar types; unknown types
/// (e.g. GEOMETRY) fall back to text, then raw bytes as hex, else NULL.
fn mysql_value_to_json(row: &MySqlRow, i: usize, type_name: &str) -> serde_json::Value {
    use serde_json::Value as J;

    macro_rules! get {
        ($ty:ty, $f:expr) => {
            row.try_get::<$ty, _>(i).map($f).unwrap_or(J::Null)
        };
    }

    match type_name {
        // tinyint(1) reports as BOOLEAN; keep it numeric (MySQL stores 0/1) like the CLI.
        "BOOLEAN" | "TINYINT" => get!(i8, |v| J::Number(v.into())),
        "TINYINT UNSIGNED" => get!(u8, |v| J::Number(v.into())),
        "SMALLINT" => get!(i16, |v| J::Number(v.into())),
        "SMALLINT UNSIGNED" | "YEAR" => get!(u16, |v| J::Number(v.into())),
        "INT" | "MEDIUMINT" => get!(i32, |v| J::Number(v.into())),
        "INT UNSIGNED" | "MEDIUMINT UNSIGNED" => get!(u32, |v| J::Number(v.into())),
        "BIGINT" => get!(i64, |v| J::Number(v.into())),
        "BIGINT UNSIGNED" | "BIT" => get!(u64, |v| J::Number(v.into())),
        "FLOAT" => get!(f32, f32_to_json),
        "DOUBLE" => get!(f64, f64_to_json),
        "DECIMAL" => get!(BigDecimal, |v: BigDecimal| J::String(v.to_string())),
        "DATE" => get!(NaiveDate, |v: NaiveDate| J::String(v.to_string())),
        "TIME" => get!(NaiveTime, |v: NaiveTime| J::String(v.to_string())),
        "DATETIME" => get!(NaiveDateTime, |v: NaiveDateTime| J::String(v.to_string())),
        "TIMESTAMP" => get!(DateTime<Utc>, |v| J::String(v.to_rfc3339())),
        "JSON" => row.try_get::<J, _>(i).unwrap_or(J::Null),
        "CHAR" | "VARCHAR" | "TEXT" | "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM" | "SET" => {
            get!(String, J::String)
        }
        "BINARY" | "VARBINARY" | "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" => {
            get!(Vec<u8>, |b: Vec<u8>| J::String(bytes_to_hex(&b, "0x")))
        }
        _ => row
            .try_get::<String, _>(i)
            .map(J::String)
            .or_else(|_| {
                row.try_get::<Vec<u8>, _>(i)
                    .map(|b| J::String(bytes_to_hex(&b, "0x")))
            })
            .unwrap_or(J::Null),
    }
}

async fn run_mysql_query(conn: &mut MySqlConnection, sql: &str) -> anyhow::Result<QueryResult> {
    let start = Instant::now();
    let trimmed = sql.trim().to_uppercase();
    if trimmed.starts_with("SELECT")
        || trimmed.starts_with("SHOW")
        || trimmed.starts_with("DESCRIBE")
        || trimmed.starts_with("DESC ")
        || trimmed.starts_with("EXPLAIN")
        || trimmed.starts_with("WITH ")
    {
        let rows: Vec<MySqlRow> = sqlx::query(sql).fetch_all(&mut *conn).await?;
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

        // Pre-compute each column's (upper-cased) MySQL type name once;
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
                    .map(|i| mysql_value_to_json(row, i, &type_names[i]))
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

fn use_database_sql(database: &str) -> anyhow::Result<String> {
    let database = SqlDialect::new(DbType::MySQL).quote_identifier(database)?;
    Ok(format!("USE {database}"))
}

async fn select_database(conn: &mut MySqlConnection, database: &str) -> anyhow::Result<()> {
    // MySQL rejects `USE` through the prepared-statement protocol.
    let sql = use_database_sql(database)?;
    conn.execute(sql.as_str()).await?;
    Ok(())
}

fn table_data_sql(
    database: &str,
    table: &TableRef,
    page: u32,
    page_size: u32,
) -> anyhow::Result<String> {
    let dialect = SqlDialect::new(DbType::MySQL);
    let database = dialect.quote_identifier(table.schema().unwrap_or(database))?;
    let table = dialect.quote_identifier(table.name())?;
    let offset = u64::from(page.saturating_sub(1)) * u64::from(page_size);
    Ok(format!(
        "SELECT * FROM {database}.{table} LIMIT {page_size} OFFSET {offset}"
    ))
}

pub struct MySqlDriver {
    config: ConnectionConfig,
    pool: Option<MySqlPool>,
}

impl MySqlDriver {
    pub fn new(config: ConnectionConfig) -> Self {
        Self { config, pool: None }
    }

    /// Build typed connect options so credentials/host with special characters
    /// (`/ # ? @ :`, spaces, …) are handled by the driver instead of being
    /// string-interpolated into a URL, which previously mis-parsed and produced
    /// errors like "invalid port number".
    fn connect_options(&self) -> MySqlConnectOptions {
        let mut opts = MySqlConnectOptions::new()
            .host(&self.config.host)
            .port(self.config.port)
            .username(&self.config.username)
            .password(&self.config.password);
        if let Some(db) = self.config.database.as_deref().filter(|d| !d.is_empty()) {
            opts = opts.database(db);
        }
        opts
    }

    fn pool(&self) -> anyhow::Result<&MySqlPool> {
        self.pool
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))
    }
}

#[async_trait]
impl DatabaseDriver for MySqlDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let pool = MySqlPoolOptions::new()
            .max_connections(5)
            .connect_with(self.connect_options())
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
        let pool = MySqlPoolOptions::new()
            .max_connections(1)
            .connect_with(self.connect_options())
            .await?;
        let _: (i32,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await?;
        pool.close().await;
        Ok(true)
    }

    async fn get_databases(&self) -> anyhow::Result<Vec<String>> {
        let pool = self.pool()?;
        let rows: Vec<MySqlRow> = sqlx::query("SHOW DATABASES").fetch_all(pool).await?;
        let databases = rows
            .iter()
            .filter_map(|row| {
                row.try_get::<String, _>(0)
                    .or_else(|_| {
                        // Some MySQL configs return VARBINARY instead of VARCHAR
                        row.try_get::<Vec<u8>, _>(0)
                            .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                    })
                    .ok()
            })
            .collect();
        Ok(databases)
    }

    async fn get_tables(&self, database: &str) -> anyhow::Result<Vec<TableInfo>> {
        let pool = self.pool()?;
        let rows: Vec<MySqlRow> = sqlx::query(
            "SELECT TABLE_NAME, TABLE_ROWS, TABLE_COMMENT FROM information_schema.TABLES WHERE TABLE_SCHEMA = ?",
        )
        .bind(database)
        .fetch_all(pool)
        .await?;
        let tables = rows
            .iter()
            .map(|row| -> anyhow::Result<_> {
                Ok(TableInfo {
                    reference: TableRef::qualified(database, row.get::<String, _>("TABLE_NAME")),
                    row_count: row.try_get::<i64, _>("TABLE_ROWS").ok(),
                    comment: row.try_get::<String, _>("TABLE_COMMENT").ok(),
                })
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(tables)
    }

    async fn get_columns(
        &self,
        database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<ColumnInfo>> {
        let pool = self.pool()?;
        let rows: Vec<MySqlRow> = sqlx::query(
            "SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE, COLUMN_KEY, COLUMN_DEFAULT, COLUMN_COMMENT \
             FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
        )
        .bind(table.schema().unwrap_or(database))
        .bind(table.name())
        .fetch_all(pool)
        .await?;
        let columns = rows
            .iter()
            .map(|row| ColumnInfo {
                name: row.get::<String, _>("COLUMN_NAME"),
                data_type: row.get::<String, _>("DATA_TYPE"),
                nullable: row.get::<String, _>("IS_NULLABLE") == "YES",
                is_primary_key: row.get::<String, _>("COLUMN_KEY") == "PRI",
                default_value: row.try_get::<String, _>("COLUMN_DEFAULT").ok(),
                comment: row.try_get::<String, _>("COLUMN_COMMENT").ok(),
            })
            .collect();
        Ok(columns)
    }

    async fn get_indexes(
        &self,
        database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<IndexInfo>> {
        let pool = self.pool()?;
        let rows: Vec<MySqlRow> = sqlx::query(
            "SELECT INDEX_NAME, COLUMN_NAME, NON_UNIQUE \
             FROM information_schema.STATISTICS \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
             ORDER BY INDEX_NAME, SEQ_IN_INDEX",
        )
        .bind(table.schema().unwrap_or(database))
        .bind(table.name())
        .fetch_all(pool)
        .await?;
        let mut indexes: std::collections::HashMap<String, IndexInfo> =
            std::collections::HashMap::new();
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
        let mut conn = pool.acquire().await?;
        select_database(&mut conn, database).await?;
        run_mysql_query(&mut conn, sql).await
    }

    async fn execute_statements(
        &self,
        database: &str,
        statements: Vec<String>,
    ) -> anyhow::Result<Vec<StatementResult>> {
        let pool = self.pool()?;
        let mut conn = pool.acquire().await?;
        select_database(&mut conn, database).await?;
        let mut results = Vec::with_capacity(statements.len());
        for sql in statements {
            let start = Instant::now();
            match run_mysql_query(&mut conn, &sql).await {
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
        table: &TableRef,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<QueryResult> {
        let sql = table_data_sql(database, table, page, page_size)?;
        self.execute_query(database, &sql).await
    }

    async fn get_views(&self, database: &str) -> anyhow::Result<Vec<ViewInfo>> {
        let pool = self.pool()?;
        let rows: Vec<MySqlRow> = sqlx::query(
            "SELECT TABLE_NAME, VIEW_DEFINITION FROM information_schema.VIEWS WHERE TABLE_SCHEMA = ?",
        )
        .bind(database)
        .fetch_all(pool)
        .await?;
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
        let rows: Vec<MySqlRow> = sqlx::query(
            "SELECT ROUTINE_NAME, DTD_IDENTIFIER, ROUTINE_DEFINITION FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = ? AND ROUTINE_TYPE = 'FUNCTION'",
        )
        .bind(database)
        .fetch_all(pool)
        .await?;
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
        let rows: Vec<MySqlRow> = sqlx::query(
            "SELECT ROUTINE_NAME, ROUTINE_DEFINITION FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = ? AND ROUTINE_TYPE = 'PROCEDURE'",
        )
        .bind(database)
        .fetch_all(pool)
        .await?;
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
        let rows: Vec<MySqlRow> = sqlx::query(
            "SELECT TRIGGER_NAME, EVENT_MANIPULATION, EVENT_OBJECT_TABLE, ACTION_TIMING FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA = ?",
        )
        .bind(database)
        .fetch_all(pool)
        .await?;
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

    async fn get_foreign_keys(
        &self,
        database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<ForeignKeyInfo>> {
        let pool = self.pool()?;
        let rows: Vec<MySqlRow> = sqlx::query(
            "SELECT CONSTRAINT_NAME, TABLE_NAME, COLUMN_NAME, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
             FROM information_schema.KEY_COLUMN_USAGE \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND REFERENCED_TABLE_NAME IS NOT NULL \
             ORDER BY CONSTRAINT_NAME, ORDINAL_POSITION",
        )
        .bind(table.schema().unwrap_or(database))
        .bind(table.name())
        .fetch_all(pool)
        .await?;
        let mut fk_map: std::collections::HashMap<String, ForeignKeyInfo> =
            std::collections::HashMap::new();
        for row in &rows {
            let name: String = row.get("CONSTRAINT_NAME");
            let from_col: String = row.get("COLUMN_NAME");
            let to_table: String = row.get("REFERENCED_TABLE_NAME");
            let to_col: String = row.get("REFERENCED_COLUMN_NAME");
            let to_table =
                TableRef::qualified(table.schema().unwrap_or(database), to_table.clone());
            let entry = fk_map
                .entry(name.clone())
                .or_insert_with(|| ForeignKeyInfo {
                    name: name.clone(),
                    from_table: table.clone(),
                    from_columns: vec![],
                    to_table,
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

    async fn get_create_table_sql(
        &self,
        database: &str,
        table: &TableRef,
    ) -> anyhow::Result<String> {
        let pool = self.pool()?;
        let dialect = SqlDialect::new(DbType::MySQL);
        let sql = format!(
            "SHOW CREATE TABLE {}.{}",
            dialect.quote_identifier(table.schema().unwrap_or(database))?,
            dialect.quote_identifier(table.name())?
        );
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

#[cfg(test)]
mod tests {
    use super::{table_data_sql, use_database_sql};
    use crate::db::TableRef;

    #[test]
    fn pagination_and_database_selection_quote_mysql_delimiters() {
        assert_eq!(
            table_data_sql("odd`database", &TableRef::unqualified("odd`table"), 2, 25,).unwrap(),
            "SELECT * FROM `odd``database`.`odd``table` LIMIT 25 OFFSET 25"
        );
        assert_eq!(
            use_database_sql("odd`database").unwrap(),
            "USE `odd``database`"
        );
    }
}
