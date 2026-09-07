use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use futures::TryStreamExt;
use std::time::Instant;
use tiberius::{AuthMethod, Client, ColumnData, Config};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use super::{
    bytes_to_hex, f32_to_json, f64_to_json, ColumnInfo, ConnectionConfig, ConstraintInfo,
    ConstraintKind, DatabaseDriver, DbType, ForeignKeyInfo, FunctionInfo, IndexInfo, ProcedureInfo,
    QueryResult, QueryRowCollector, QueryRowSink, SqlDialect, StatementResult, TableInfo, TableRef,
    TriggerInfo, UserInfo, ViewInfo,
};

/// Decode a SQL Server cell into a JSON value by matching on the tiberius
/// `ColumnData` variant — covering every TDS type. Date/time variants are
/// converted through tiberius' chrono `FromSql` impls (by column index).
fn mssql_cell_to_json(
    row: &tiberius::Row,
    i: usize,
    cell: &ColumnData<'static>,
) -> serde_json::Value {
    use serde_json::Value as J;

    macro_rules! via {
        ($ty:ty, $f:expr) => {
            row.try_get::<$ty, _>(i).ok().flatten().map_or(J::Null, $f)
        };
    }

    match cell {
        ColumnData::U8(o) => (*o).map_or(J::Null, |v| J::Number(v.into())),
        ColumnData::I16(o) => (*o).map_or(J::Null, |v| J::Number(v.into())),
        ColumnData::I32(o) => (*o).map_or(J::Null, |v| J::Number(v.into())),
        ColumnData::I64(o) => (*o).map_or(J::Null, |v| J::Number(v.into())),
        ColumnData::F32(o) => (*o).map_or(J::Null, f32_to_json),
        ColumnData::F64(o) => (*o).map_or(J::Null, f64_to_json),
        ColumnData::Bit(o) => (*o).map_or(J::Null, J::Bool),
        ColumnData::String(o) => o.as_ref().map_or(J::Null, |s| J::String(s.to_string())),
        ColumnData::Guid(o) => (*o).map_or(J::Null, |g| J::String(g.to_string())),
        ColumnData::Numeric(o) => (*o).map_or(J::Null, |n| J::String(n.to_string())),
        ColumnData::Xml(o) => o.as_ref().map_or(J::Null, |x| J::String(x.to_string())),
        ColumnData::Binary(o) => o
            .as_ref()
            .map_or(J::Null, |b| J::String(bytes_to_hex(&b[..], "0x"))),
        ColumnData::Date(_) => via!(NaiveDate, |v: NaiveDate| J::String(v.to_string())),
        ColumnData::Time(_) => via!(NaiveTime, |v: NaiveTime| J::String(v.to_string())),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            via!(NaiveDateTime, |v: NaiveDateTime| J::String(v.to_string()))
        }
        ColumnData::DateTimeOffset(_) => {
            via!(DateTime<Utc>, |v: DateTime<Utc>| J::String(v.to_rfc3339()))
        }
    }
}

async fn run_mssql_query(
    client: &mut Client<tokio_util::compat::Compat<TcpStream>>,
    sql: &str,
) -> anyhow::Result<QueryResult> {
    let start = Instant::now();
    let stream = client.query(sql, &[]).await?;
    let mut collector = QueryRowCollector::new(None);
    let result = consume_mssql_stream(stream, &mut collector, start).await?;
    Ok(collector.finish(result))
}

pub(super) async fn run_mssql_batch(
    client: &mut Client<tokio_util::compat::Compat<TcpStream>>,
    sql: &str,
) -> anyhow::Result<QueryResult> {
    let start = Instant::now();
    let stream = client.simple_query(sql).await?;
    let mut collector = QueryRowCollector::new(None);
    let result = consume_mssql_stream(stream, &mut collector, start).await?;
    Ok(collector.finish(result))
}

pub(super) async fn consume_mssql_stream(
    mut stream: tiberius::QueryStream<'_>,
    sink: &mut dyn QueryRowSink,
    start: Instant,
) -> anyhow::Result<QueryResult> {
    let mut columns = Vec::new();
    while let Some(item) = stream.try_next().await? {
        let tiberius::QueryItem::Row(row) = item else {
            continue;
        };
        if row.result_index() != 0 {
            continue;
        }
        if columns.is_empty() {
            columns = row
                .columns()
                .iter()
                .map(|column| ColumnInfo {
                    name: column.name().to_string(),
                    data_type: format!("{:?}", column.column_type()),
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
                row.cells()
                    .enumerate()
                    .map(|(index, (_, cell))| mssql_cell_to_json(&row, index, cell))
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
}

fn use_database_sql(database: &str) -> anyhow::Result<String> {
    let database = SqlDialect::new(DbType::SQLServer).quote_identifier(database)?;
    Ok(format!("USE {database}"))
}

fn in_database_sql(database: &str, sql: &str) -> anyhow::Result<String> {
    Ok(format!("{}; {sql}", use_database_sql(database)?))
}

fn table_data_sql(table: &TableRef, page: u32, page_size: u32) -> anyhow::Result<String> {
    let table = SqlDialect::new(DbType::SQLServer).quote_table_ref(table)?;
    let offset = u64::from(page.saturating_sub(1)) * u64::from(page_size);
    Ok(format!(
        "SELECT * FROM {table} ORDER BY (SELECT NULL) OFFSET {offset} ROWS FETCH NEXT {page_size} ROWS ONLY"
    ))
}

pub struct SqlServerDriver {
    config: ConnectionConfig,
    client: Option<Mutex<Client<tokio_util::compat::Compat<TcpStream>>>>,
}

impl SqlServerDriver {
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            client: None,
        }
    }

    fn tiberius_config(&self) -> anyhow::Result<Config> {
        let mut config = Config::new();
        config.host(&self.config.host);
        config.port(self.config.port);
        config.authentication(AuthMethod::sql_server(
            &self.config.username,
            &self.config.password,
        ));
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
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let stream = client
            .query("SELECT name FROM sys.databases ORDER BY name", &[])
            .await?;
        let rows = stream.into_first_result().await?;
        let databases: Vec<String> = rows
            .iter()
            .filter_map(|row| {
                row.try_get::<&str, _>(0)
                    .ok()
                    .flatten()
                    .map(|s| s.to_string())
            })
            .collect();
        Ok(databases)
    }

    async fn get_tables(&self, database: &str) -> anyhow::Result<Vec<TableInfo>> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = in_database_sql(
            database,
            "SELECT TABLE_NAME, TABLE_SCHEMA FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME",
        )?;
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let tables = rows
            .iter()
            .map(|row| -> anyhow::Result<_> {
                Ok(TableInfo {
                    reference: TableRef::from_parts(
                        row.try_get::<&str, _>(1).ok().flatten().map(str::to_string),
                        row.try_get::<&str, _>(0)
                            .ok()
                            .flatten()
                            .unwrap_or("")
                            .to_string(),
                    ),
                    row_count: None,
                    comment: None,
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
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let (schema, table_name) = table.schema_and_table("dbo");
        let sql = in_database_sql(
            database,
            "SELECT c.COLUMN_NAME, c.DATA_TYPE, c.IS_NULLABLE, c.COLUMN_DEFAULT, \
             CASE WHEN pk.COLUMN_NAME IS NOT NULL THEN 1 ELSE 0 END as IS_PK \
             FROM INFORMATION_SCHEMA.COLUMNS c \
             LEFT JOIN (SELECT ku.TABLE_SCHEMA, ku.TABLE_NAME, ku.COLUMN_NAME FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc \
             JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE ku ON tc.CONSTRAINT_NAME = ku.CONSTRAINT_NAME \
             WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY') pk ON c.TABLE_SCHEMA = pk.TABLE_SCHEMA AND c.TABLE_NAME = pk.TABLE_NAME AND c.COLUMN_NAME = pk.COLUMN_NAME \
             WHERE c.TABLE_NAME = @P1 AND c.TABLE_SCHEMA = @P2 ORDER BY c.ORDINAL_POSITION",
        )?;
        let stream = client.query(sql.as_str(), &[&table_name, &schema]).await?;
        let rows = stream.into_first_result().await?;
        let columns: Vec<ColumnInfo> = rows
            .iter()
            .map(|row| ColumnInfo {
                name: row
                    .try_get::<&str, _>(0)
                    .ok()
                    .flatten()
                    .unwrap_or("")
                    .to_string(),
                data_type: row
                    .try_get::<&str, _>(1)
                    .ok()
                    .flatten()
                    .unwrap_or("")
                    .to_string(),
                nullable: row.try_get::<&str, _>(2).ok().flatten().unwrap_or("YES") == "YES",
                is_primary_key: row.try_get::<i32, _>(4).ok().flatten().unwrap_or(0) == 1,
                default_value: row
                    .try_get::<&str, _>(3)
                    .ok()
                    .flatten()
                    .map(|s| s.to_string()),
                comment: None,
            })
            .collect();
        Ok(columns)
    }

    async fn get_indexes(
        &self,
        database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<IndexInfo>> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let (schema, table_name) = table.schema_and_table("dbo");
        let sql = in_database_sql(
            database,
            "SELECT i.name AS index_name, COL_NAME(ic.object_id, ic.column_id) AS column_name, \
             i.is_unique, i.is_primary_key \
             FROM sys.indexes i \
             JOIN sys.index_columns ic ON i.object_id = ic.object_id AND i.index_id = ic.index_id \
             WHERE OBJECT_NAME(i.object_id) = @P1 AND OBJECT_SCHEMA_NAME(i.object_id) = @P2 \
             ORDER BY i.name, ic.key_ordinal",
        )?;
        let stream = client.query(sql.as_str(), &[&table_name, &schema]).await?;
        let rows = stream.into_first_result().await?;
        let mut indexes: std::collections::HashMap<String, IndexInfo> =
            std::collections::HashMap::new();
        for row in &rows {
            let name = row
                .try_get::<&str, _>(0)
                .ok()
                .flatten()
                .unwrap_or("")
                .to_string();
            let column = row
                .try_get::<&str, _>(1)
                .ok()
                .flatten()
                .unwrap_or("")
                .to_string();
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

    async fn get_constraints(
        &self,
        database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<ConstraintInfo>> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let (schema, table_name) = table.schema_and_table("dbo");
        let sql = in_database_sql(
            database,
            "SELECT kc.name, kc.type_desc, COL_NAME(ic.object_id, ic.column_id) AS column_name, \
             CAST(NULL AS nvarchar(max)) AS definition, ic.key_ordinal \
             FROM sys.key_constraints kc \
             JOIN sys.index_columns ic \
               ON ic.object_id = kc.parent_object_id AND ic.index_id = kc.unique_index_id \
             WHERE kc.parent_object_id = OBJECT_ID(QUOTENAME(@P1) + '.' + QUOTENAME(@P2)) \
             UNION ALL \
             SELECT cc.name, 'CHECK_CONSTRAINT', CAST(NULL AS nvarchar(128)), cc.definition, 0 \
             FROM sys.check_constraints cc \
             WHERE cc.parent_object_id = OBJECT_ID(QUOTENAME(@P1) + '.' + QUOTENAME(@P2)) \
             ORDER BY 1, 5",
        )?;
        let stream = client.query(sql.as_str(), &[&schema, &table_name]).await?;
        let rows = stream.into_first_result().await?;
        let mut constraints = std::collections::BTreeMap::<String, ConstraintInfo>::new();
        for row in &rows {
            let name = row
                .try_get::<&str, _>(0)
                .ok()
                .flatten()
                .unwrap_or("")
                .to_string();
            let kind = match row.try_get::<&str, _>(1).ok().flatten().unwrap_or("") {
                "PRIMARY_KEY_CONSTRAINT" => ConstraintKind::PrimaryKey,
                "UNIQUE_CONSTRAINT" => ConstraintKind::Unique,
                "CHECK_CONSTRAINT" => ConstraintKind::Check,
                value => anyhow::bail!("Unknown SQL Server constraint type {value}"),
            };
            let entry = constraints
                .entry(name.clone())
                .or_insert_with(|| ConstraintInfo {
                    name,
                    kind,
                    columns: Vec::new(),
                    definition: row.try_get::<&str, _>(3).ok().flatten().map(str::to_string),
                });
            if let Some(column) = row.try_get::<&str, _>(2).ok().flatten() {
                entry.columns.push(column.to_string());
            }
        }
        Ok(constraints.into_values().collect())
    }

    async fn execute_query(&self, database: &str, sql: &str) -> anyhow::Result<QueryResult> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let use_database = use_database_sql(database)?;
        run_mssql_query(&mut client, &use_database).await?;
        run_mssql_batch(&mut client, sql).await
    }

    async fn execute_query_stream(
        &self,
        database: &str,
        sql: &str,
        sink: &mut dyn QueryRowSink,
    ) -> anyhow::Result<QueryResult> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        run_mssql_query(&mut client, &use_database_sql(database)?).await?;
        let start = Instant::now();
        let stream = client.simple_query(sql).await?;
        consume_mssql_stream(stream, sink, start).await
    }

    async fn explain(&self, database: &str, statement: &str) -> anyhow::Result<QueryResult> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let use_database = use_database_sql(database)?;
        client.query(use_database.as_str(), &[]).await?;
        run_mssql_query(&mut client, "SET SHOWPLAN_ALL ON").await?;

        // SHOWPLAN_ALL is session-scoped, so cleanup must run even when plan generation fails.
        let plan = run_mssql_query(&mut client, statement).await;
        let cleanup = run_mssql_query(&mut client, "SET SHOWPLAN_ALL OFF").await;
        match (plan, cleanup) {
            (Ok(plan), Ok(_)) => Ok(plan),
            (Err(error), Ok(_)) => Err(error),
            (Ok(_), Err(cleanup_error)) => Err(anyhow::anyhow!(
                "Failed to disable SQL Server SHOWPLAN_ALL: {cleanup_error}"
            )),
            (Err(error), Err(cleanup_error)) => Err(anyhow::anyhow!(
                "Explain failed: {error}; disabling SQL Server SHOWPLAN_ALL also failed: {cleanup_error}"
            )),
        }
    }

    async fn execute_statements(
        &self,
        database: &str,
        statements: Vec<String>,
    ) -> anyhow::Result<Vec<StatementResult>> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let use_database = use_database_sql(database)?;
        client.query(use_database.as_str(), &[]).await?;
        let mut results = Vec::with_capacity(statements.len());
        for sql in statements {
            let start = Instant::now();
            match run_mssql_batch(&mut client, &sql).await {
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
        let mut connection = self.create_client().await?;
        run_mssql_batch(&mut connection, &use_database_sql(database)?).await?;
        if let Some(level) = isolation.sql() {
            run_mssql_batch(
                &mut connection,
                &format!("SET TRANSACTION ISOLATION LEVEL {level}"),
            )
            .await?;
        }
        run_mssql_batch(&mut connection, "BEGIN TRANSACTION").await?;
        Ok(Box::new(super::transaction::OwnedTransaction(
            super::transaction::TransactionConnection::SqlServer(Box::new(connection)),
        )))
    }

    async fn execute_mutation_batch(
        &self,
        database: &str,
        statements: Vec<String>,
    ) -> anyhow::Result<Vec<StatementResult>> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let use_database = use_database_sql(database)?;
        client.query(use_database.as_str(), &[]).await?;
        run_mssql_batch(&mut client, "BEGIN TRANSACTION").await?;
        let mut results = Vec::with_capacity(statements.len());
        for sql in statements {
            let start = Instant::now();
            match run_mssql_batch(&mut client, &sql).await {
                Ok(result) => results.push(StatementResult::from_query_result(sql, result)),
                Err(error) => {
                    let elapsed = start.elapsed().as_millis() as u64;
                    let message = error.to_string();
                    if let Err(rollback_error) =
                        run_mssql_batch(&mut client, "IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION")
                            .await
                    {
                        return Err(anyhow::anyhow!(
                            "Mutation failed: {message}; rollback also failed: {rollback_error}"
                        ));
                    }
                    results.push(StatementResult::from_error(sql, message, elapsed));
                    return Ok(results);
                }
            }
        }
        run_mssql_batch(&mut client, "COMMIT TRANSACTION").await?;
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

    async fn get_views(&self, database: &str) -> anyhow::Result<Vec<ViewInfo>> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = in_database_sql(
            database,
            "SELECT SCHEMA_NAME(schema_id) AS schema_name, name, OBJECT_DEFINITION(object_id) AS definition FROM sys.views",
        )?;
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let views: Vec<ViewInfo> = rows
            .iter()
            .map(|row| ViewInfo {
                name: format!(
                    "{}.{}",
                    row.try_get::<&str, _>(0).ok().flatten().unwrap_or("dbo"),
                    row.try_get::<&str, _>(1).ok().flatten().unwrap_or("")
                ),
                definition: row
                    .try_get::<&str, _>(2)
                    .ok()
                    .flatten()
                    .map(|s| s.to_string()),
            })
            .collect();
        Ok(views)
    }

    async fn get_functions(&self, database: &str) -> anyhow::Result<Vec<FunctionInfo>> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = in_database_sql(
            database,
            "SELECT SCHEMA_NAME(schema_id) AS schema_name, name, OBJECT_DEFINITION(object_id) AS definition, type_desc FROM sys.objects WHERE type IN ('FN', 'IF', 'TF')",
        )?;
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let functions: Vec<FunctionInfo> = rows
            .iter()
            .map(|row| FunctionInfo {
                name: format!(
                    "{}.{}",
                    row.try_get::<&str, _>(0).ok().flatten().unwrap_or("dbo"),
                    row.try_get::<&str, _>(1).ok().flatten().unwrap_or("")
                ),
                language: Some("T-SQL".to_string()),
                return_type: row
                    .try_get::<&str, _>(3)
                    .ok()
                    .flatten()
                    .map(|s| s.to_string()),
                definition: row
                    .try_get::<&str, _>(2)
                    .ok()
                    .flatten()
                    .map(|s| s.to_string()),
            })
            .collect();
        Ok(functions)
    }

    async fn get_procedures(&self, database: &str) -> anyhow::Result<Vec<ProcedureInfo>> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = in_database_sql(
            database,
            "SELECT SCHEMA_NAME(schema_id) AS schema_name, name, OBJECT_DEFINITION(object_id) AS definition FROM sys.procedures",
        )?;
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let procedures: Vec<ProcedureInfo> = rows
            .iter()
            .map(|row| ProcedureInfo {
                name: format!(
                    "{}.{}",
                    row.try_get::<&str, _>(0).ok().flatten().unwrap_or("dbo"),
                    row.try_get::<&str, _>(1).ok().flatten().unwrap_or("")
                ),
                language: Some("T-SQL".to_string()),
                definition: row
                    .try_get::<&str, _>(2)
                    .ok()
                    .flatten()
                    .map(|s| s.to_string()),
            })
            .collect();
        Ok(procedures)
    }

    async fn get_triggers(&self, database: &str) -> anyhow::Result<Vec<TriggerInfo>> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = in_database_sql(
            database,
            "SELECT OBJECT_SCHEMA_NAME(t.object_id) AS trigger_schema, t.name, \
             t.is_instead_of_trigger, te.type_desc, \
             OBJECT_SCHEMA_NAME(t.parent_id) AS table_schema, OBJECT_NAME(t.parent_id) AS table_name \
             FROM sys.triggers t \
             JOIN sys.trigger_events te ON t.object_id = te.object_id",
        )?;
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        let triggers: Vec<TriggerInfo> = rows
            .iter()
            .map(|row| {
                let name = format!(
                    "{}.{}",
                    row.try_get::<&str, _>(0).ok().flatten().unwrap_or("dbo"),
                    row.try_get::<&str, _>(1).ok().flatten().unwrap_or("")
                );
                let instead_of = row.try_get::<bool, _>(2).ok().flatten().unwrap_or(false);
                let event = row
                    .try_get::<&str, _>(3)
                    .ok()
                    .flatten()
                    .unwrap_or("")
                    .to_string();
                let table = format!(
                    "{}.{}",
                    row.try_get::<&str, _>(4).ok().flatten().unwrap_or("dbo"),
                    row.try_get::<&str, _>(5).ok().flatten().unwrap_or("")
                );
                TriggerInfo {
                    name,
                    event: event.clone(),
                    table,
                    timing: if instead_of {
                        "INSTEAD OF".to_string()
                    } else {
                        "AFTER".to_string()
                    },
                }
            })
            .collect();
        Ok(triggers)
    }

    async fn get_foreign_keys(
        &self,
        database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<ForeignKeyInfo>> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let (schema, table_name) = table.schema_and_table("dbo");
        let sql = in_database_sql(
            database,
            "SELECT fk.name AS constraint_name, \
             COL_NAME(fkc.parent_object_id, fkc.parent_column_id) AS from_column, \
             OBJECT_SCHEMA_NAME(fkc.referenced_object_id) AS to_schema, \
             OBJECT_NAME(fkc.referenced_object_id) AS to_table, \
             COL_NAME(fkc.referenced_object_id, fkc.referenced_column_id) AS to_column \
             FROM sys.foreign_keys fk \
             JOIN sys.foreign_key_columns fkc ON fk.object_id = fkc.constraint_object_id \
             WHERE OBJECT_NAME(fk.parent_object_id) = @P1 \
             AND OBJECT_SCHEMA_NAME(fk.parent_object_id) = @P2",
        )?;
        let stream = client.query(sql.as_str(), &[&table_name, &schema]).await?;
        let rows = stream.into_first_result().await?;
        let mut fk_map: std::collections::HashMap<String, ForeignKeyInfo> =
            std::collections::HashMap::new();
        for row in &rows {
            let name = row
                .try_get::<&str, _>(0)
                .ok()
                .flatten()
                .unwrap_or("")
                .to_string();
            let from_col = row
                .try_get::<&str, _>(1)
                .ok()
                .flatten()
                .unwrap_or("")
                .to_string();
            let to_table = row
                .try_get::<&str, _>(3)
                .ok()
                .flatten()
                .unwrap_or("")
                .to_string();
            let to_schema = row
                .try_get::<&str, _>(2)
                .ok()
                .flatten()
                .unwrap_or("dbo")
                .to_string();
            let to_col = row
                .try_get::<&str, _>(4)
                .ok()
                .flatten()
                .unwrap_or("")
                .to_string();
            let to_table = TableRef::qualified(to_schema, to_table);
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

    async fn get_schemas(&self, database: &str) -> anyhow::Result<Vec<String>> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let sql = in_database_sql(
            database,
            "SELECT SCHEMA_NAME FROM INFORMATION_SCHEMA.SCHEMATA ORDER BY SCHEMA_NAME",
        )?;
        let stream = client.query(sql.as_str(), &[]).await?;
        let rows = stream.into_first_result().await?;
        Ok(rows
            .iter()
            .filter_map(|row| row.try_get::<&str, _>(0).ok().flatten().map(str::to_string))
            .collect())
    }

    async fn get_users(&self) -> anyhow::Result<Vec<UserInfo>> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let stream = client
            .query(
                "SELECT name FROM sys.server_principals \
                 WHERE type = 'S' AND name <> 'sa' AND name NOT LIKE '##%' ORDER BY name",
                &[],
            )
            .await?;
        let rows = stream.into_first_result().await?;
        let users: Vec<UserInfo> = rows
            .iter()
            .map(|row| UserInfo {
                name: row
                    .try_get::<&str, _>(0)
                    .ok()
                    .flatten()
                    .unwrap_or("")
                    .to_string(),
                host: None,
            })
            .collect();
        Ok(users)
    }

    async fn get_create_table_sql(
        &self,
        database: &str,
        table: &TableRef,
    ) -> anyhow::Result<String> {
        let mutex = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let mut client = mutex.lock().await;
        let (schema, table_name) = table.schema_and_table("dbo");
        let sql = in_database_sql(
            database,
            "SELECT c.COLUMN_NAME, c.DATA_TYPE, c.IS_NULLABLE, c.COLUMN_DEFAULT, \
                    CAST(c.CHARACTER_MAXIMUM_LENGTH AS int), CAST(c.NUMERIC_PRECISION AS int), \
                    CAST(c.NUMERIC_SCALE AS int), CAST(c.DATETIME_PRECISION AS int), \
                    CAST(COLUMNPROPERTY(OBJECT_ID(QUOTENAME(c.TABLE_SCHEMA) + '.' + QUOTENAME(c.TABLE_NAME)), c.COLUMN_NAME, 'IsIdentity') AS int), \
                    CAST(IDENT_SEED(QUOTENAME(c.TABLE_SCHEMA) + '.' + QUOTENAME(c.TABLE_NAME)) AS bigint), \
                    CAST(IDENT_INCR(QUOTENAME(c.TABLE_SCHEMA) + '.' + QUOTENAME(c.TABLE_NAME)) AS bigint), \
                    CAST(CASE WHEN EXISTS ( \
                        SELECT 1 FROM sys.indexes i \
                        JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
                        JOIN sys.columns sc ON sc.object_id = ic.object_id AND sc.column_id = ic.column_id \
                        WHERE i.is_primary_key = 1 \
                          AND i.object_id = OBJECT_ID(QUOTENAME(c.TABLE_SCHEMA) + '.' + QUOTENAME(c.TABLE_NAME)) \
                          AND sc.name = c.COLUMN_NAME \
                    ) THEN 1 ELSE 0 END AS int) \
             FROM INFORMATION_SCHEMA.COLUMNS c WHERE c.TABLE_NAME = @P1 AND c.TABLE_SCHEMA = @P2 \
             ORDER BY c.ORDINAL_POSITION",
        )?;
        let stream = client.query(sql.as_str(), &[&table_name, &schema]).await?;
        let rows = stream.into_first_result().await?;
        let dialect = SqlDialect::new(DbType::SQLServer);
        let mut ddl = format!("CREATE TABLE {} (\n", dialect.quote_table_ref(table)?);
        let mut primary_key_columns = Vec::new();
        let col_defs: Vec<String> = rows
            .iter()
            .map(|row| -> anyhow::Result<String> {
                let name = row.try_get::<&str, _>(0).ok().flatten().unwrap_or("");
                let dtype = row.try_get::<&str, _>(1).ok().flatten().unwrap_or("");
                let nullable = row.try_get::<&str, _>(2).ok().flatten().unwrap_or("YES");
                let default = row.try_get::<&str, _>(3).ok().flatten();
                let character_length = row.try_get::<i32, _>(4).ok().flatten();
                let precision = row.try_get::<i32, _>(5).ok().flatten();
                let scale = row.try_get::<i32, _>(6).ok().flatten();
                let datetime_precision = row.try_get::<i32, _>(7).ok().flatten();
                let identity = row.try_get::<i32, _>(8).ok().flatten() == Some(1);
                let identity_seed = row.try_get::<i64, _>(9).ok().flatten();
                let identity_increment = row.try_get::<i64, _>(10).ok().flatten();
                let primary_key = row.try_get::<i32, _>(11).ok().flatten() == Some(1);
                let data_type = sql_server_column_type(
                    dtype,
                    character_length,
                    precision,
                    scale,
                    datetime_precision,
                );
                let null_str = if nullable == "NO" { " NOT NULL" } else { "" };
                let identity = identity_clause(identity, identity_seed, identity_increment);
                let default = default
                    .map(|default| format!(" DEFAULT {default}"))
                    .unwrap_or_default();
                if primary_key {
                    primary_key_columns.push(dialect.quote_identifier(name)?);
                }
                Ok(format!(
                    "  {} {data_type}{identity}{null_str}{default}",
                    dialect.quote_identifier(name)?,
                ))
            })
            .collect::<Result<_, _>>()?;
        let mut definitions = col_defs;
        if !primary_key_columns.is_empty() {
            definitions.push(format!(
                "  PRIMARY KEY ({})",
                primary_key_columns.join(", ")
            ));
        }
        ddl.push_str(&definitions.join(",\n"));
        ddl.push_str("\n);");
        Ok(ddl)
    }

    fn db_type(&self) -> DbType {
        DbType::SQLServer
    }
}

fn sql_server_column_type(
    data_type: &str,
    character_length: Option<i32>,
    precision: Option<i32>,
    scale: Option<i32>,
    datetime_precision: Option<i32>,
) -> String {
    match data_type.to_ascii_lowercase().as_str() {
        "char" | "varchar" | "nchar" | "nvarchar" | "binary" | "varbinary" => {
            match character_length {
                Some(-1) => format!("{data_type}(MAX)"),
                Some(length) => format!("{data_type}({length})"),
                None => data_type.to_string(),
            }
        }
        "decimal" | "numeric" => match (precision, scale) {
            (Some(precision), Some(scale)) => format!("{data_type}({precision},{scale})"),
            _ => data_type.to_string(),
        },
        "datetime2" | "datetimeoffset" | "time" => datetime_precision
            .map(|precision| format!("{data_type}({precision})"))
            .unwrap_or_else(|| data_type.to_string()),
        _ => data_type.to_string(),
    }
}

fn identity_clause(identity: bool, seed: Option<i64>, increment: Option<i64>) -> String {
    if identity {
        format!(
            " IDENTITY({},{})",
            seed.unwrap_or(1),
            increment.unwrap_or(1)
        )
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        identity_clause, in_database_sql, sql_server_column_type, table_data_sql, use_database_sql,
    };
    use crate::db::TableRef;

    #[test]
    fn pagination_and_database_selection_quote_sqlserver_delimiters() {
        assert_eq!(
            table_data_sql(
                &TableRef::qualified("odd]schema", "odd]table's"),
                2,
                25,
            )
            .unwrap(),
            "SELECT * FROM [odd]]schema].[odd]]table's] ORDER BY (SELECT NULL) OFFSET 25 ROWS FETCH NEXT 25 ROWS ONLY"
        );
        assert_eq!(
            use_database_sql("odd]database").unwrap(),
            "USE [odd]]database]"
        );
        assert_eq!(
            in_database_sql("odd]database", "SELECT 1").unwrap(),
            "USE [odd]]database]; SELECT 1"
        );
    }

    #[test]
    fn reconstructed_column_types_keep_size_precision_and_scale() {
        assert_eq!(
            sql_server_column_type("nvarchar", Some(255), None, None, None),
            "nvarchar(255)"
        );
        assert_eq!(
            sql_server_column_type("varchar", Some(-1), None, None, None),
            "varchar(MAX)"
        );
        assert_eq!(
            sql_server_column_type("decimal", None, Some(18), Some(4), None),
            "decimal(18,4)"
        );
        assert_eq!(
            sql_server_column_type("datetime2", None, None, None, Some(3)),
            "datetime2(3)"
        );
        assert_eq!(identity_clause(true, Some(10), Some(5)), " IDENTITY(10,5)");
        assert_eq!(identity_clause(false, Some(10), Some(5)), "");
    }
}
