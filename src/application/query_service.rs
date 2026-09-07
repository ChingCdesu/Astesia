use std::time::Instant;

use crate::db::{QueryResult, QueryRowSink, StatementResult, TableRef};

use super::{connections::ConnectionManager, QueryTarget};

#[derive(Clone)]
pub struct QueryService {
    manager: ConnectionManager,
}

impl QueryService {
    pub(super) fn new(manager: ConnectionManager) -> Self {
        Self { manager }
    }

    pub async fn execute(
        &self,
        connection_id: &str,
        database: &str,
        sql: &str,
    ) -> Result<QueryResult, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .execute_query(database, sql)
            .await
            .map_err(|error| format!("查询失败: {error}"))
    }

    pub(crate) async fn stream_export(
        &self,
        connection_id: &str,
        database: &str,
        sql: &str,
        sink: &mut dyn QueryRowSink,
    ) -> Result<QueryResult, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .execute_query_stream(database, sql, sink)
            .await
            .map_err(|error| format!("Export query failed: {error}"))
    }

    pub(crate) async fn execute_export_query(
        &self,
        target: &QueryTarget,
        sql: &str,
        sink: &mut dyn QueryRowSink,
    ) -> Result<QueryResult, String> {
        let (handle, generation) = self.manager.driver_session(&target.connection_id).await?;
        if generation != target.session_generation {
            return Err(format!(
                "Database Session changed before export (expected {}, found {generation})",
                target.session_generation
            ));
        }
        let driver = handle.lock_active().await?;
        if driver.db_type() != target.db_type {
            return Err("Database Session engine changed before export".to_string());
        }
        driver
            .execute_query_stream(&target.database, sql, sink)
            .await
            .map_err(|error| format!("Export query failed: {error}"))
    }

    pub async fn execute_statements(
        &self,
        connection_id: &str,
        database: &str,
        statements: Vec<String>,
    ) -> Result<Vec<StatementResult>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .execute_statements(database, statements)
            .await
            .map_err(|error| format!("查询失败: {error}"))
    }

    pub(crate) async fn execute_in_context(
        &self,
        target: &QueryTarget,
        schema: Option<&str>,
        operation: super::QueryOperation,
    ) -> Result<Vec<StatementResult>, String> {
        let (handle, generation) = self.manager.driver_session(&target.connection_id).await?;
        if generation != target.session_generation {
            return Err("Database Session changed before execution".into());
        }
        let driver = handle.lock_active().await?;
        if driver.db_type() != target.db_type {
            return Err("Database engine changed before execution".into());
        }
        let (statements, explained) = match operation {
            super::QueryOperation::Statements(statements) => (statements, None),
            super::QueryOperation::Explain(statement) if schema.is_none() => {
                let started = Instant::now();
                return Ok(vec![
                    match driver.explain(&target.database, &statement).await {
                        Ok(result) => StatementResult::from_query_result(statement, result),
                        Err(error) => StatementResult::from_error(
                            statement,
                            error,
                            started.elapsed().as_millis() as u64,
                        ),
                    },
                ]);
            }
            super::QueryOperation::Explain(statement) => {
                let sql = crate::db::SqlDialect::new(target.db_type)
                    .build_explain_statement(&statement)
                    .map_err(|error| error.to_string())?;
                (vec![sql], Some(statement))
            }
            super::QueryOperation::Redis { .. } => {
                return Err("Redis does not support SQL contexts".into())
            }
        };
        let mut results = driver
            .execute_statements_in_schema(&target.database, schema, statements)
            .await
            .map_err(|error| error.to_string())?;
        if let (Some(source), Some(result)) = (explained, results.first_mut()) {
            result.sql = source;
        }
        Ok(results)
    }

    pub async fn explain(
        &self,
        connection_id: &str,
        database: &str,
        statement: String,
    ) -> Result<StatementResult, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        let started = Instant::now();
        Ok(match driver.explain(database, &statement).await {
            Ok(result) => StatementResult::from_query_result(statement, result),
            Err(error) => {
                StatementResult::from_error(statement, error, started.elapsed().as_millis() as u64)
            }
        })
    }

    pub async fn table_data(
        &self,
        connection_id: &str,
        database: &str,
        table: &TableRef,
        page: u32,
        page_size: u32,
    ) -> Result<QueryResult, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_table_data(database, table, page, page_size)
            .await
            .map_err(|error| format!("获取数据失败: {error}"))
    }
}
