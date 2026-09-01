use crate::db::{DbType, RowMutationMode, SqlDialect, TableRef, UnsupportedFeature};
use serde_json::Value;

use super::connections::ConnectionManager;

#[derive(Clone)]
pub struct MutationService {
    manager: ConnectionManager,
}

#[derive(Debug, Clone)]
pub struct RowUpdate {
    pub table: TableRef,
    pub primary_key_column: String,
    pub primary_key_value: Value,
    pub column: String,
    pub new_value: Value,
}

impl MutationService {
    pub(super) fn new(manager: ConnectionManager) -> Self {
        Self { manager }
    }

    pub async fn update_row(
        &self,
        connection_id: &str,
        database: &str,
        update: RowUpdate,
    ) -> Result<u64, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        let db_type = driver.db_type();
        require_mutation_mode(db_type, RowMutationMode::StructuredSql)?;
        let sql = SqlDialect::new(db_type).build_update_row(
            &update.table,
            &update.primary_key_column,
            &update.primary_key_value,
            &update.column,
            &update.new_value,
        )?;
        let result = driver
            .execute_query(database, &sql)
            .await
            .map_err(|error| error.to_string())?;
        Ok(result.affected_rows)
    }

    pub async fn delete_rows(
        &self,
        connection_id: &str,
        database: &str,
        table: &TableRef,
        primary_key_column: &str,
        primary_key_values: Vec<Value>,
    ) -> Result<u64, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        let db_type = driver.db_type();
        require_mutation_mode(db_type, RowMutationMode::StructuredSql)?;
        let sql = SqlDialect::new(db_type).build_delete_rows(
            table,
            primary_key_column,
            &primary_key_values,
        )?;
        let result = driver
            .execute_query(database, &sql)
            .await
            .map_err(|error| error.to_string())?;
        Ok(result.affected_rows)
    }

    pub async fn insert_row(
        &self,
        connection_id: &str,
        database: &str,
        table: &TableRef,
        columns: Vec<String>,
        values: Vec<Value>,
    ) -> Result<u64, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        let db_type = driver.db_type();
        require_mutation_mode(db_type, RowMutationMode::StructuredSql)?;
        let sql = SqlDialect::new(db_type).build_insert_row(table, &columns, &values)?;
        let result = driver
            .execute_query(database, &sql)
            .await
            .map_err(|error| error.to_string())?;
        Ok(result.affected_rows)
    }

    pub async fn redis_set_key(
        &self,
        connection_id: &str,
        database: &str,
        key: &str,
        value: &str,
        ttl: Option<i64>,
    ) -> Result<String, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        require_mutation_mode(driver.db_type(), RowMutationMode::RedisKeyValue)?;

        driver
            .set_key(database, key, value, positive_ttl_seconds(ttl))
            .await
            .map_err(|e| e.to_string())?;
        Ok("OK".to_string())
    }

    pub async fn redis_delete_key(
        &self,
        connection_id: &str,
        database: &str,
        key: &str,
    ) -> Result<u64, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        require_mutation_mode(driver.db_type(), RowMutationMode::RedisKeyValue)?;
        driver
            .delete_key(database, key)
            .await
            .map_err(|e| e.to_string())
    }
}

fn positive_ttl_seconds(ttl: Option<i64>) -> Option<u64> {
    ttl.and_then(|ttl| u64::try_from(ttl).ok())
        .filter(|ttl| *ttl > 0)
}

fn require_mutation_mode(db_type: DbType, expected: RowMutationMode) -> Result<(), String> {
    if db_type.capabilities().row_mutation == expected {
        Ok(())
    } else {
        Err(UnsupportedFeature::new(db_type, "row mutation").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_services_follow_the_engine_policy() {
        assert!(require_mutation_mode(DbType::PostgreSQL, RowMutationMode::StructuredSql).is_ok());
        assert!(require_mutation_mode(DbType::Redis, RowMutationMode::RedisKeyValue).is_ok());
        assert!(require_mutation_mode(DbType::MongoDB, RowMutationMode::StructuredSql).is_err());
        assert!(require_mutation_mode(DbType::MySQL, RowMutationMode::RedisKeyValue).is_err());
    }

    #[test]
    fn redis_ttl_preserves_the_existing_positive_only_contract() {
        assert_eq!(positive_ttl_seconds(Some(30)), Some(30));
        assert_eq!(positive_ttl_seconds(Some(0)), None);
        assert_eq!(positive_ttl_seconds(Some(-1)), None);
        assert_eq!(positive_ttl_seconds(None), None);
    }
}
