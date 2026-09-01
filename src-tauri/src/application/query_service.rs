use crate::db::{QueryResult, StatementResult, TableRef};

use super::connections::ConnectionManager;

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
