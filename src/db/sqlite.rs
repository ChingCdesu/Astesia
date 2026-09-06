use async_trait::async_trait;
use futures::TryStreamExt;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions, SqliteRow};
use sqlx::{Column, Row, SqliteConnection, TypeInfo, ValueRef};
use std::time::Instant;

use super::{
    bytes_to_hex, f64_to_json, ColumnInfo, ConnectionConfig, ConstraintInfo, ConstraintKind,
    DatabaseDriver, DbType, ForeignKeyInfo, IndexInfo, QueryResult, QueryRowCollector,
    QueryRowSink, SqlDialect, StatementResult, TableInfo, TableRef, TriggerInfo, ViewInfo,
};

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
        _ => row
            .try_get::<String, _>(i)
            .map(J::String)
            .unwrap_or(J::Null),
    }
}

pub(super) async fn run_sqlite_query(
    conn: &mut SqliteConnection,
    sql: &str,
) -> anyhow::Result<QueryResult> {
    let mut collector = QueryRowCollector::new(None);
    let result = stream_sqlite_query(conn, sql, &mut collector).await?;
    Ok(collector.finish(result))
}

pub(super) async fn stream_sqlite_query(
    conn: &mut SqliteConnection,
    sql: &str,
    sink: &mut dyn QueryRowSink,
) -> anyhow::Result<QueryResult> {
    let start = Instant::now();
    let trimmed = sql.trim().to_uppercase();
    if trimmed.starts_with("SELECT")
        || trimmed.starts_with("PRAGMA")
        || trimmed.starts_with("EXPLAIN")
        || trimmed.starts_with("WITH ")
        || trimmed.starts_with("VALUES")
    {
        let mut stream = sqlx::query(sql).fetch(&mut *conn);
        let mut columns = Vec::new();
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
                sink.columns(&columns).await;
            }
            if sink.wants_rows() {
                sink.row(
                    (0..row.columns().len())
                        .map(|i| sqlite_value_to_json(&row, i))
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
        let result = sqlx::query(sql).execute(&mut *conn).await?;
        let elapsed = start.elapsed().as_millis() as u64;
        Ok(QueryResult {
            affected_rows: result.rows_affected(),
            execution_time_ms: elapsed,
            ..Default::default()
        })
    }
}

fn table_data_sql(table: &TableRef, page: u32, page_size: u32) -> anyhow::Result<String> {
    let table = SqlDialect::new(DbType::SQLite).quote_table_ref(table)?;
    let offset = u64::from(page.saturating_sub(1)) * u64::from(page_size);
    Ok(format!(
        "SELECT * FROM {table} LIMIT {page_size} OFFSET {offset}"
    ))
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
        self.pool
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))
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
            .map(|row| -> anyhow::Result<_> {
                Ok(TableInfo {
                    reference: TableRef::unqualified(row.get::<String, _>("name")),
                    row_count: None,
                    comment: None,
                })
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(tables)
    }

    async fn get_columns(
        &self,
        _database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<ColumnInfo>> {
        let pool = self.pool()?;
        let rows: Vec<SqliteRow> = sqlx::query(
            "SELECT name, type, \"notnull\", dflt_value, pk \
             FROM pragma_table_info(?) ORDER BY cid",
        )
        .bind(table.name())
        .fetch_all(pool)
        .await?;
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

    async fn get_indexes(
        &self,
        _database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<IndexInfo>> {
        let pool = self.pool()?;
        let rows: Vec<SqliteRow> = sqlx::query("SELECT * FROM pragma_index_list(?)")
            .bind(table.name())
            .fetch_all(pool)
            .await?;
        let mut indexes = Vec::new();
        for row in &rows {
            let name: String = row.get("name");
            let unique: i32 = row.get("unique");
            let info_rows: Vec<SqliteRow> = sqlx::query("SELECT * FROM pragma_index_info(?)")
                .bind(&name)
                .fetch_all(pool)
                .await?;
            let columns: Vec<String> = info_rows
                .iter()
                .map(|r| r.get::<String, _>("name"))
                .collect();
            indexes.push(IndexInfo {
                name,
                columns,
                is_unique: unique == 1,
                is_primary: false,
            });
        }
        Ok(indexes)
    }

    async fn get_constraints(
        &self,
        _database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<ConstraintInfo>> {
        let pool = self.pool()?;
        let table_rows: Vec<SqliteRow> = sqlx::query("SELECT * FROM pragma_table_info(?)")
            .bind(table.name())
            .fetch_all(pool)
            .await?;
        let mut primary_columns = table_rows
            .iter()
            .filter_map(|row| {
                let position = row.get::<i32, _>("pk");
                (position > 0).then(|| (position, row.get::<String, _>("name")))
            })
            .collect::<Vec<_>>();
        primary_columns.sort_by_key(|(position, _)| *position);

        let mut constraints = Vec::new();
        if !primary_columns.is_empty() {
            constraints.push(ConstraintInfo {
                name: "PRIMARY KEY".to_string(),
                kind: ConstraintKind::PrimaryKey,
                columns: primary_columns
                    .into_iter()
                    .map(|(_, column)| column)
                    .collect(),
                definition: None,
            });
        }

        let index_rows: Vec<SqliteRow> = sqlx::query("SELECT * FROM pragma_index_list(?)")
            .bind(table.name())
            .fetch_all(pool)
            .await?;
        for row in index_rows
            .iter()
            .filter(|row| row.get::<String, _>("origin") == "u")
        {
            let name = row.get::<String, _>("name");
            let info_rows: Vec<SqliteRow> = sqlx::query("SELECT * FROM pragma_index_info(?)")
                .bind(&name)
                .fetch_all(pool)
                .await?;
            constraints.push(ConstraintInfo {
                name,
                kind: ConstraintKind::Unique,
                columns: info_rows
                    .iter()
                    .map(|row| row.get::<String, _>("name"))
                    .collect(),
                definition: None,
            });
        }

        let create_sql =
            sqlx::query("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?")
                .bind(table.name())
                .fetch_optional(pool)
                .await?
                .and_then(|row| row.try_get::<String, _>("sql").ok());
        if let Some(create_sql) = create_sql {
            constraints.extend(
                sqlite_check_expressions(&create_sql)
                    .into_iter()
                    .enumerate()
                    .map(|(index, definition)| ConstraintInfo {
                        name: format!("CHECK {}", index + 1),
                        kind: ConstraintKind::Check,
                        columns: Vec::new(),
                        definition: Some(definition),
                    }),
            );
        }
        Ok(constraints)
    }

    async fn execute_query(&self, _database: &str, sql: &str) -> anyhow::Result<QueryResult> {
        let pool = self.pool()?;
        let mut conn = pool.acquire().await?;
        run_sqlite_query(&mut conn, sql).await
    }

    async fn execute_query_stream(
        &self,
        _database: &str,
        sql: &str,
        sink: &mut dyn QueryRowSink,
    ) -> anyhow::Result<QueryResult> {
        let pool = self.pool()?;
        let mut conn = pool.acquire().await?;
        stream_sqlite_query(&mut conn, sql, sink).await
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
            match run_sqlite_query(&mut conn, &sql).await {
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
        _database: &str,
        isolation: super::TransactionIsolation,
    ) -> anyhow::Result<Box<dyn super::DatabaseTransaction>> {
        anyhow::ensure!(
            self.db_type().transaction_isolations().contains(&isolation),
            "Unsupported transaction isolation"
        );
        let mut connection = self.pool()?.acquire().await?.detach();
        run_sqlite_query(&mut connection, "BEGIN").await?;
        Ok(Box::new(super::transaction::OwnedTransaction(
            super::transaction::TransactionConnection::Sqlite(connection),
        )))
    }

    async fn execute_mutation_batch(
        &self,
        _database: &str,
        statements: Vec<String>,
    ) -> anyhow::Result<Vec<StatementResult>> {
        let pool = self.pool()?;
        let mut conn = pool.acquire().await?;
        run_sqlite_query(&mut conn, "BEGIN IMMEDIATE").await?;
        let mut results = Vec::with_capacity(statements.len());
        for sql in statements {
            let start = Instant::now();
            match run_sqlite_query(&mut conn, &sql).await {
                Ok(result) => results.push(StatementResult::from_query_result(sql, result)),
                Err(error) => {
                    let elapsed = start.elapsed().as_millis() as u64;
                    let message = error.to_string();
                    if let Err(rollback_error) = run_sqlite_query(&mut conn, "ROLLBACK").await {
                        return Err(anyhow::anyhow!(
                            "Mutation failed: {message}; rollback also failed: {rollback_error}"
                        ));
                    }
                    results.push(StatementResult::from_error(sql, message, elapsed));
                    return Ok(results);
                }
            }
        }
        run_sqlite_query(&mut conn, "COMMIT").await?;
        Ok(results)
    }

    async fn get_table_data(
        &self,
        database: &str,
        table: &TableRef,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<QueryResult> {
        let sql = table_data_sql(table, page, page_size)?;
        self.execute_query(database, &sql).await
    }

    async fn get_views(&self, _database: &str) -> anyhow::Result<Vec<ViewInfo>> {
        let pool = self.pool()?;
        let rows: Vec<SqliteRow> =
            sqlx::query("SELECT name, sql FROM sqlite_master WHERE type = 'view'")
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
        let rows: Vec<SqliteRow> =
            sqlx::query("SELECT name, tbl_name, sql FROM sqlite_master WHERE type = 'trigger'")
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

    async fn get_foreign_keys(
        &self,
        _database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<ForeignKeyInfo>> {
        let pool = self.pool()?;
        let rows: Vec<SqliteRow> = sqlx::query("SELECT * FROM pragma_foreign_key_list(?)")
            .bind(table.name())
            .fetch_all(pool)
            .await?;
        let mut fk_map: std::collections::HashMap<i32, ForeignKeyInfo> =
            std::collections::HashMap::new();
        for row in &rows {
            let id: i32 = row.get("id");
            let ref_table: String = row.get("table");
            let from_col: String = row.get("from");
            let to_col: String = row.get("to");
            let ref_table = TableRef::from_parts(table.schema().map(str::to_string), ref_table);
            let entry = fk_map.entry(id).or_insert_with(|| ForeignKeyInfo {
                name: format!("fk_{}_{}", table, id),
                from_table: table.clone(),
                from_columns: vec![],
                to_table: ref_table,
                to_columns: vec![],
            });
            entry.from_columns.push(from_col);
            entry.to_columns.push(to_col);
        }
        Ok(fk_map.into_values().collect())
    }

    async fn get_create_table_sql(
        &self,
        _database: &str,
        table: &TableRef,
    ) -> anyhow::Result<String> {
        let pool = self.pool()?;
        let rows: Vec<SqliteRow> =
            sqlx::query("SELECT sql FROM sqlite_master WHERE type='table' AND name = ?")
                .bind(table.name())
                .fetch_all(pool)
                .await?;
        rows.first()
            .and_then(|r| r.try_get::<String, _>("sql").ok())
            .ok_or_else(|| anyhow::anyhow!("Table not found"))
    }

    fn db_type(&self) -> DbType {
        DbType::SQLite
    }
}

fn sqlite_check_expressions(create_sql: &str) -> Vec<String> {
    let bytes = create_sql.as_bytes();
    let mut expressions = Vec::new();
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        if let Some(delimiter) = quote {
            if bytes[index] == delimiter {
                if index + 1 < bytes.len() && bytes[index + 1] == delimiter {
                    index += 2;
                    continue;
                }
                quote = None;
            }
            index += 1;
            continue;
        }
        if matches!(bytes[index], b'\'' | b'"' | b'`') {
            quote = Some(bytes[index]);
            index += 1;
            continue;
        }
        let keyword_end = index.saturating_add(5);
        let is_check = keyword_end <= bytes.len()
            && bytes[index..keyword_end].eq_ignore_ascii_case(b"CHECK")
            && (index == 0 || !is_identifier_byte(bytes[index - 1]))
            && (keyword_end == bytes.len() || !is_identifier_byte(bytes[keyword_end]));
        if !is_check {
            index += 1;
            continue;
        }
        let mut open = keyword_end;
        while open < bytes.len() && bytes[open].is_ascii_whitespace() {
            open += 1;
        }
        if open == bytes.len() || bytes[open] != b'(' {
            index = keyword_end;
            continue;
        }
        if let Some(close) = matching_sql_parenthesis(bytes, open) {
            expressions.push(create_sql[open + 1..close].trim().to_string());
            index = close + 1;
        } else {
            break;
        }
    }
    expressions
}

fn matching_sql_parenthesis(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut index = open;
    while index < bytes.len() {
        if let Some(delimiter) = quote {
            if bytes[index] == delimiter {
                if index + 1 < bytes.len() && bytes[index + 1] == delimiter {
                    index += 2;
                    continue;
                }
                quote = None;
            }
            index += 1;
            continue;
        }
        match bytes[index] {
            b'\'' | b'"' | b'`' => quote = Some(bytes[index]),
            b'(' => depth += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn is_identifier_byte(value: u8) -> bool {
    value.is_ascii_alphanumeric() || value == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ConnectionConfig {
        ConnectionConfig {
            id: "sqlite-special-identifiers".to_string(),
            name: "SQLite special identifiers".to_string(),
            db_type: DbType::SQLite,
            host: ":memory:".to_string(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            color: None,
        }
    }

    #[tokio::test]
    async fn limited_queries_keep_a_truncation_sentinel_and_drain_mutations() {
        let mut driver = SqliteDriver::new(config());
        driver.connect().await.unwrap();
        driver
            .execute_query("main", "CREATE TABLE memory_rows (id INTEGER)")
            .await
            .unwrap();
        let sql = "WITH RECURSIVE numbers(id) AS (VALUES(1) UNION ALL \
                   SELECT id + 1 FROM numbers WHERE id < 100) \
                   INSERT INTO memory_rows SELECT id FROM numbers RETURNING id";
        let result = driver.execute_query_limited("main", sql, 2).await.unwrap();
        assert_eq!(result.rows.len(), 3);
        assert_eq!(result.columns[0].name, "id");
        let count = driver
            .execute_query("main", "SELECT COUNT(*) FROM memory_rows")
            .await
            .unwrap();
        assert_eq!(count.rows[0][0], serde_json::json!(100));
        let empty = driver
            .execute_query_limited("main", "SELECT id FROM memory_rows WHERE 0", 2)
            .await
            .unwrap();
        assert!(empty.rows.is_empty());
        assert!(empty.columns.is_empty());
        let changed = driver
            .execute_query_limited("main", "UPDATE memory_rows SET id = id + 1", 2)
            .await
            .unwrap();
        assert_eq!(changed.affected_rows, 100);
    }

    #[tokio::test]
    async fn limited_queries_report_errors_after_the_retained_rows() {
        let mut driver = SqliteDriver::new(config());
        driver.connect().await.unwrap();
        let sql = "WITH RECURSIVE numbers(id) AS (VALUES(1) UNION ALL \
                   SELECT id + 1 FROM numbers WHERE id < 10) \
                   SELECT CASE WHEN id = 10 THEN abs(-9223372036854775808) ELSE id END FROM numbers";
        let result = driver.execute_query_limited("main", sql, 1).await;
        assert!(result.is_err());
        assert!(driver.execute_query("main", "SELECT 1").await.is_ok());
    }

    #[tokio::test]
    async fn columns_and_pagination_accept_delimiter_bearing_table_names() {
        let mut driver = SqliteDriver::new(config());
        driver.connect().await.unwrap();
        let table = TableRef::unqualified("odd\"table's");
        driver
            .execute_query(
                "main",
                "CREATE TABLE \"odd\"\"table's\" (\"display\"\"name\" TEXT UNIQUE)",
            )
            .await
            .unwrap();
        driver
            .execute_query(
                "main",
                "INSERT INTO \"odd\"\"table's\" (\"display\"\"name\") VALUES ('ready')",
            )
            .await
            .unwrap();

        let columns = driver.get_columns("main", &table).await.unwrap();
        assert_eq!(columns[0].name, "display\"name");
        assert_eq!(driver.get_indexes("main", &table).await.unwrap().len(), 1);
        assert!(driver
            .get_create_table_sql("main", &table)
            .await
            .unwrap()
            .starts_with("CREATE TABLE"));
        let rows = driver.get_table_data("main", &table, 1, 10).await.unwrap();
        assert_eq!(rows.rows[0][0], serde_json::json!("ready"));
        assert_eq!(
            table_data_sql(&table, 2, 10).unwrap(),
            "SELECT * FROM \"odd\"\"table's\" LIMIT 10 OFFSET 10"
        );
    }

    #[tokio::test]
    async fn explain_executes_the_sqlite_query_plan() {
        let mut driver = SqliteDriver::new(config());
        driver.connect().await.unwrap();

        let plan = driver.explain("main", "SELECT 1").await.unwrap();

        assert!(!plan.rows.is_empty());
        assert_eq!(
            plan.columns
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            vec!["id", "parent", "notused", "detail"]
        );
    }

    #[tokio::test]
    async fn constraints_distinguish_keys_unique_constraints_and_checks() {
        let mut driver = SqliteDriver::new(config());
        driver.connect().await.unwrap();
        let table = TableRef::unqualified("accounts");
        driver
            .execute_query(
                "main",
                "CREATE TABLE accounts (tenant_id INTEGER, id INTEGER, email TEXT UNIQUE, balance INTEGER CHECK (balance >= 0 AND length('CHECK (ignored)') > 0), PRIMARY KEY (tenant_id, id))",
            )
            .await
            .unwrap();

        let constraints = driver.get_constraints("main", &table).await.unwrap();

        assert!(constraints.iter().any(|constraint| {
            constraint.kind == ConstraintKind::PrimaryKey
                && constraint.columns == ["tenant_id", "id"]
        }));
        assert!(constraints.iter().any(|constraint| {
            constraint.kind == ConstraintKind::Unique && constraint.columns == ["email"]
        }));
        assert!(constraints.iter().any(|constraint| {
            constraint.kind == ConstraintKind::Check
                && constraint.definition.as_deref()
                    == Some("balance >= 0 AND length('CHECK (ignored)') > 0")
        }));
    }
}
