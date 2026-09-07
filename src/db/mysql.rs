use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use futures::TryStreamExt;
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions, MySqlRow};
use sqlx::types::BigDecimal;
use sqlx::{Column, Executor, MySqlConnection, Row, TypeInfo};
use std::time::Instant;

use super::{
    bytes_to_hex, f32_to_json, f64_to_json, ColumnInfo, ConnectionConfig, ConstraintInfo,
    ConstraintKind, DatabaseDriver, DbType, ForeignKeyInfo, FunctionInfo, IndexInfo, ProcedureInfo,
    QueryResult, QueryRowCollector, QueryRowSink, SqlDialect, StatementResult, TableInfo, TableRef,
    TriggerInfo, UserInfo, ViewInfo,
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

fn mysql_catalog_text(row: &MySqlRow, column: &str) -> anyhow::Result<String> {
    row.try_get::<String, _>(column)
        .or_else(|_| {
            row.try_get::<Vec<u8>, _>(column)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        })
        .map_err(Into::into)
}

fn mysql_optional_catalog_text(row: &MySqlRow, column: &str) -> anyhow::Result<Option<String>> {
    if let Ok(value) = row.try_get::<Option<String>, _>(column) {
        return Ok(value);
    }
    row.try_get::<Option<Vec<u8>>, _>(column)
        .map(|value| value.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
        .map_err(Into::into)
}

pub(super) async fn run_mysql_query(
    conn: &mut MySqlConnection,
    sql: &str,
) -> anyhow::Result<QueryResult> {
    let mut collector = QueryRowCollector::new(None);
    let result = stream_mysql_query(conn, sql, &mut collector).await?;
    Ok(collector.finish(result))
}

pub(super) async fn stream_mysql_query(
    conn: &mut MySqlConnection,
    sql: &str,
    sink: &mut dyn QueryRowSink,
) -> anyhow::Result<QueryResult> {
    let start = Instant::now();
    let trimmed = sql.trim().to_uppercase();
    if trimmed.starts_with("SELECT")
        || trimmed.starts_with("SHOW")
        || trimmed.starts_with("DESCRIBE")
        || trimmed.starts_with("DESC ")
        || trimmed.starts_with("EXPLAIN")
        || trimmed.starts_with("WITH ")
    {
        let mut stream = sqlx::query(sql).fetch(&mut *conn);
        let mut columns = Vec::new();
        let mut type_names: Vec<String> = Vec::new();
        while let Some(row) = stream.try_next().await? {
            if columns.is_empty() {
                columns = row
                    .columns()
                    .iter()
                    .map(|column| ColumnInfo {
                        name: column.name().to_string(),
                        data_type: column.type_info().name().to_string(),
                        nullable: true,
                        is_primary_key: false,
                        default_value: None,
                        comment: None,
                    })
                    .collect();
                type_names = row
                    .columns()
                    .iter()
                    .map(|column| column.type_info().name().to_ascii_uppercase())
                    .collect();
                sink.columns(&columns).await;
            }
            if sink.wants_rows() {
                sink.row(
                    (0..type_names.len())
                        .map(|i| mysql_value_to_json(&row, i, &type_names[i]))
                        .collect(),
                )
                .await;
            }
        }
        Ok(QueryResult {
            columns,
            execution_time_ms: start.elapsed().as_millis() as u64,
            ..Default::default()
        })
    } else {
        let result = conn.execute(sql).await?;
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
                    reference: TableRef::qualified(
                        database,
                        mysql_catalog_text(row, "TABLE_NAME")?,
                    ),
                    row_count: row.try_get::<i64, _>("TABLE_ROWS").ok(),
                    comment: mysql_optional_catalog_text(row, "TABLE_COMMENT")?,
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
            "SELECT COLUMN_NAME, CAST(COLUMN_TYPE AS CHAR) AS COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, \
             CAST(COLUMN_DEFAULT AS CHAR) AS COLUMN_DEFAULT, CAST(COLUMN_COMMENT AS CHAR) AS COLUMN_COMMENT \
             FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
        )
        .bind(table.schema().unwrap_or(database))
        .bind(table.name())
        .fetch_all(pool)
        .await?;
        let columns = rows
            .iter()
            .map(|row| -> anyhow::Result<_> {
                Ok(ColumnInfo {
                    name: mysql_catalog_text(row, "COLUMN_NAME")?,
                    data_type: mysql_catalog_text(row, "COLUMN_TYPE")?,
                    nullable: mysql_catalog_text(row, "IS_NULLABLE")? == "YES",
                    is_primary_key: mysql_catalog_text(row, "COLUMN_KEY")? == "PRI",
                    default_value: mysql_optional_catalog_text(row, "COLUMN_DEFAULT")?,
                    comment: mysql_optional_catalog_text(row, "COLUMN_COMMENT")?,
                })
            })
            .collect::<anyhow::Result<_>>()?;
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
            let name = mysql_catalog_text(row, "INDEX_NAME")?;
            let column = mysql_catalog_text(row, "COLUMN_NAME")?;
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

    async fn get_constraints(
        &self,
        database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<ConstraintInfo>> {
        let pool = self.pool()?;
        let schema = table.schema().unwrap_or(database);
        let rows: Vec<MySqlRow> = sqlx::query(
            "SELECT tc.CONSTRAINT_NAME, tc.CONSTRAINT_TYPE, \
             GROUP_CONCAT(kcu.COLUMN_NAME ORDER BY kcu.ORDINAL_POSITION SEPARATOR ', ') AS COLUMN_NAMES, \
             cc.CHECK_CLAUSE \
             FROM information_schema.TABLE_CONSTRAINTS tc \
             LEFT JOIN information_schema.KEY_COLUMN_USAGE kcu \
               ON kcu.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA \
              AND kcu.TABLE_NAME = tc.TABLE_NAME \
              AND kcu.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
             LEFT JOIN information_schema.CHECK_CONSTRAINTS cc \
               ON cc.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA \
              AND cc.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
             WHERE tc.TABLE_SCHEMA = ? AND tc.TABLE_NAME = ? \
               AND tc.CONSTRAINT_TYPE IN ('PRIMARY KEY', 'UNIQUE', 'CHECK') \
             GROUP BY tc.CONSTRAINT_NAME, tc.CONSTRAINT_TYPE, cc.CHECK_CLAUSE \
             ORDER BY tc.CONSTRAINT_NAME",
        )
        .bind(schema)
        .bind(table.name())
        .fetch_all(pool)
        .await?;
        rows.iter()
            .map(|row| {
                let constraint_type = mysql_catalog_text(row, "CONSTRAINT_TYPE")?;
                let kind = match constraint_type.as_str() {
                    "PRIMARY KEY" => ConstraintKind::PrimaryKey,
                    "UNIQUE" => ConstraintKind::Unique,
                    "CHECK" => ConstraintKind::Check,
                    value => anyhow::bail!("Unknown MySQL constraint type {value}"),
                };
                let columns = mysql_optional_catalog_text(row, "COLUMN_NAMES")?
                    .unwrap_or_default()
                    .split(", ")
                    .filter(|column| !column.is_empty())
                    .map(str::to_string)
                    .collect();
                Ok(ConstraintInfo {
                    name: mysql_catalog_text(row, "CONSTRAINT_NAME")?,
                    kind,
                    columns,
                    definition: mysql_optional_catalog_text(row, "CHECK_CLAUSE")?,
                })
            })
            .collect()
    }

    async fn execute_query(&self, database: &str, sql: &str) -> anyhow::Result<QueryResult> {
        let pool = self.pool()?;
        let mut conn = pool.acquire().await?;
        select_database(&mut conn, database).await?;
        run_mysql_query(&mut conn, sql).await
    }

    async fn execute_query_stream(
        &self,
        database: &str,
        sql: &str,
        sink: &mut dyn QueryRowSink,
    ) -> anyhow::Result<QueryResult> {
        let pool = self.pool()?;
        let mut conn = pool.acquire().await?;
        select_database(&mut conn, database).await?;
        stream_mysql_query(&mut conn, sql, sink).await
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

    async fn begin_transaction(
        &self,
        database: &str,
        isolation: super::TransactionIsolation,
    ) -> anyhow::Result<Box<dyn super::DatabaseTransaction>> {
        anyhow::ensure!(
            self.db_type().transaction_isolations().contains(&isolation),
            "Unsupported transaction isolation"
        );
        let mut connection = self.pool()?.acquire().await?.detach();
        select_database(&mut connection, database).await?;
        if let Some(level) = isolation.sql() {
            run_mysql_query(
                &mut connection,
                &format!("SET TRANSACTION ISOLATION LEVEL {level}"),
            )
            .await?;
        }
        run_mysql_query(&mut connection, "START TRANSACTION").await?;
        Ok(Box::new(super::transaction::OwnedTransaction(
            super::transaction::TransactionConnection::Mysql(connection),
        )))
    }

    async fn execute_mutation_batch(
        &self,
        database: &str,
        statements: Vec<String>,
    ) -> anyhow::Result<Vec<StatementResult>> {
        let pool = self.pool()?;
        let mut conn = pool.acquire().await?;
        select_database(&mut conn, database).await?;
        run_mysql_query(&mut conn, "START TRANSACTION").await?;
        let mut results = Vec::with_capacity(statements.len());
        for sql in statements {
            let start = Instant::now();
            match run_mysql_query(&mut conn, &sql).await {
                Ok(result) => results.push(StatementResult::from_query_result(sql, result)),
                Err(error) => {
                    let elapsed = start.elapsed().as_millis() as u64;
                    let message = error.to_string();
                    if let Err(rollback_error) = run_mysql_query(&mut conn, "ROLLBACK").await {
                        return Err(anyhow::anyhow!(
                            "Mutation failed: {message}; rollback also failed: {rollback_error}"
                        ));
                    }
                    results.push(StatementResult::from_error(sql, message, elapsed));
                    return Ok(results);
                }
            }
        }
        run_mysql_query(&mut conn, "COMMIT").await?;
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
            .map(|row| -> anyhow::Result<_> {
                Ok(ViewInfo {
                    name: mysql_catalog_text(row, "TABLE_NAME")?,
                    definition: mysql_optional_catalog_text(row, "VIEW_DEFINITION")?,
                })
            })
            .collect::<anyhow::Result<_>>()?;
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
            .map(|row| -> anyhow::Result<_> {
                Ok(FunctionInfo {
                    name: mysql_catalog_text(row, "ROUTINE_NAME")?,
                    language: Some("SQL".to_string()),
                    return_type: mysql_optional_catalog_text(row, "DTD_IDENTIFIER")?,
                    definition: mysql_optional_catalog_text(row, "ROUTINE_DEFINITION")?,
                })
            })
            .collect::<anyhow::Result<_>>()?;
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
            .map(|row| -> anyhow::Result<_> {
                Ok(ProcedureInfo {
                    name: mysql_catalog_text(row, "ROUTINE_NAME")?,
                    language: Some("SQL".to_string()),
                    definition: mysql_optional_catalog_text(row, "ROUTINE_DEFINITION")?,
                })
            })
            .collect::<anyhow::Result<_>>()?;
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
            .map(|row| -> anyhow::Result<_> {
                Ok(TriggerInfo {
                    name: mysql_catalog_text(row, "TRIGGER_NAME")?,
                    event: mysql_catalog_text(row, "EVENT_MANIPULATION")?,
                    table: mysql_catalog_text(row, "EVENT_OBJECT_TABLE")?,
                    timing: mysql_catalog_text(row, "ACTION_TIMING")?,
                })
            })
            .collect::<anyhow::Result<_>>()?;
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
            let name = mysql_catalog_text(row, "CONSTRAINT_NAME")?;
            let from_col = mysql_catalog_text(row, "COLUMN_NAME")?;
            let to_table = mysql_catalog_text(row, "REFERENCED_TABLE_NAME")?;
            let to_col = mysql_catalog_text(row, "REFERENCED_COLUMN_NAME")?;
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
        let rows: Vec<MySqlRow> = sqlx::query(
            "SELECT CAST(User AS CHAR) AS User, CAST(Host AS CHAR) AS Host FROM mysql.user",
        )
        .fetch_all(pool)
        .await?;
        let users = rows
            .iter()
            .map(|row| -> anyhow::Result<_> {
                Ok(UserInfo {
                    name: mysql_catalog_text(row, "User")?,
                    host: mysql_optional_catalog_text(row, "Host")?,
                })
            })
            .collect::<anyhow::Result<_>>()?;
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
