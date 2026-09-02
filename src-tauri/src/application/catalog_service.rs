use crate::db::{
    ColumnInfo, ForeignKeyInfo, FunctionInfo, IndexInfo, ProcedureInfo, TableInfo, TableRef,
    TriggerInfo, UnsupportedFeature, UserInfo, ViewInfo,
};

use super::connections::ConnectionManager;
use super::{CatalogKind, CatalogLoadResult, TableStructureLoadError, TableStructureSnapshot};

#[derive(Clone)]
pub struct CatalogService {
    manager: ConnectionManager,
}

impl CatalogService {
    pub(super) fn new(manager: ConnectionManager) -> Self {
        Self { manager }
    }

    pub async fn databases(&self, connection_id: &str) -> Result<Vec<String>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_databases()
            .await
            .map_err(|error| format!("获取数据库列表失败: {error}"))
    }

    pub async fn tables(
        &self,
        connection_id: &str,
        database: &str,
    ) -> Result<Vec<TableInfo>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_tables(database)
            .await
            .map_err(|error| format!("获取表列表失败: {error}"))
    }

    pub(crate) async fn catalog_section(
        &self,
        connection_id: &str,
        database: &str,
        kind: CatalogKind,
    ) -> CatalogLoadResult {
        let handle = match self.manager.driver(connection_id).await {
            Ok(handle) => handle,
            Err(error) => return CatalogLoadResult::failed(kind, error),
        };
        let driver = match handle.lock_active().await {
            Ok(driver) => driver,
            Err(error) => return CatalogLoadResult::failed(kind, error),
        };
        if !kind.supported(driver.db_type()) {
            return CatalogLoadResult::failed(
                kind,
                format!("{kind:?} is not supported for {:?}", driver.db_type()),
            );
        }
        match kind {
            CatalogKind::Schemas => CatalogLoadResult::Schemas(
                driver
                    .get_schemas(database)
                    .await
                    .map_err(|error| format!("获取Schema列表失败: {error}")),
            ),
            CatalogKind::Tables => CatalogLoadResult::Tables(
                driver
                    .get_tables(database)
                    .await
                    .map_err(|error| format!("获取表列表失败: {error}")),
            ),
            CatalogKind::Views => CatalogLoadResult::Views(
                driver
                    .get_views(database)
                    .await
                    .map_err(|error| format!("获取视图失败: {error}")),
            ),
            CatalogKind::Functions => CatalogLoadResult::Functions(
                driver
                    .get_functions(database)
                    .await
                    .map_err(|error| format!("获取函数失败: {error}")),
            ),
            CatalogKind::Procedures => CatalogLoadResult::Procedures(
                driver
                    .get_procedures(database)
                    .await
                    .map_err(|error| format!("获取存储过程失败: {error}")),
            ),
            CatalogKind::Triggers => CatalogLoadResult::Triggers(
                driver
                    .get_triggers(database)
                    .await
                    .map_err(|error| format!("获取触发器失败: {error}")),
            ),
            CatalogKind::Users => CatalogLoadResult::Users(
                driver
                    .get_users()
                    .await
                    .map_err(|error| format!("获取用户列表失败: {error}")),
            ),
        }
    }

    pub async fn columns(
        &self,
        connection_id: &str,
        database: &str,
        table: &TableRef,
    ) -> Result<Vec<ColumnInfo>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_columns(database, table)
            .await
            .map_err(|error| format!("获取列信息失败: {error}"))
    }

    pub async fn indexes(
        &self,
        connection_id: &str,
        database: &str,
        table: &TableRef,
    ) -> Result<Vec<IndexInfo>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_indexes(database, table)
            .await
            .map_err(|error| format!("获取索引信息失败: {error}"))
    }

    pub(crate) async fn table_structure(
        &self,
        connection_id: &str,
        database: &str,
        table: &TableRef,
    ) -> Result<TableStructureSnapshot, TableStructureLoadError> {
        let handle = self
            .manager
            .driver(connection_id)
            .await
            .map_err(TableStructureLoadError::Connection)?;
        let driver = handle
            .lock_active()
            .await
            .map_err(TableStructureLoadError::Connection)?;
        let db_type = driver.db_type();
        let capabilities = db_type.capabilities();
        if !capabilities.sql {
            return Err(TableStructureLoadError::Unsupported(
                UnsupportedFeature::new(db_type, "SQL table structure browsing").to_string(),
            ));
        }
        let columns = driver
            .get_columns(database, table)
            .await
            .map_err(|error| TableStructureLoadError::Columns(error.to_string()))?;
        let indexes = driver
            .get_indexes(database, table)
            .await
            .map_err(|error| TableStructureLoadError::Indexes(error.to_string()))?;
        let constraints = if capabilities.constraints {
            Some(
                driver
                    .get_constraints(database, table)
                    .await
                    .map_err(|error| TableStructureLoadError::Constraints(error.to_string()))?,
            )
        } else {
            None
        };
        let foreign_keys = if capabilities.foreign_keys {
            Some(
                driver
                    .get_foreign_keys(database, table)
                    .await
                    .map_err(|error| TableStructureLoadError::ForeignKeys(error.to_string()))?,
            )
        } else {
            None
        };
        Ok(TableStructureSnapshot {
            columns,
            indexes,
            constraints,
            foreign_keys,
        })
    }

    pub async fn schemas(
        &self,
        connection_id: &str,
        database: &str,
    ) -> Result<Vec<String>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_schemas(database)
            .await
            .map_err(|error| format!("获取Schema列表失败: {error}"))
    }

    pub async fn enum_values(
        &self,
        connection_id: &str,
        database: &str,
        enum_type: &str,
    ) -> Result<Vec<String>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_enum_values(database, enum_type)
            .await
            .map_err(|error| format!("获取枚举值失败: {error}"))
    }

    pub async fn views(
        &self,
        connection_id: &str,
        database: &str,
    ) -> Result<Vec<ViewInfo>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_views(database)
            .await
            .map_err(|error| format!("获取视图失败: {error}"))
    }

    pub async fn functions(
        &self,
        connection_id: &str,
        database: &str,
    ) -> Result<Vec<FunctionInfo>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_functions(database)
            .await
            .map_err(|error| format!("获取函数失败: {error}"))
    }

    pub async fn procedures(
        &self,
        connection_id: &str,
        database: &str,
    ) -> Result<Vec<ProcedureInfo>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_procedures(database)
            .await
            .map_err(|error| format!("获取存储过程失败: {error}"))
    }

    pub async fn triggers(
        &self,
        connection_id: &str,
        database: &str,
    ) -> Result<Vec<TriggerInfo>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_triggers(database)
            .await
            .map_err(|error| format!("获取触发器失败: {error}"))
    }

    pub async fn foreign_keys(
        &self,
        connection_id: &str,
        database: &str,
        table: &TableRef,
    ) -> Result<Vec<ForeignKeyInfo>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_foreign_keys(database, table)
            .await
            .map_err(|error| format!("获取外键失败: {error}"))
    }

    pub async fn users(&self, connection_id: &str) -> Result<Vec<UserInfo>, String> {
        let handle = self.manager.driver(connection_id).await?;
        let driver = handle.lock_active().await?;
        driver
            .get_users()
            .await
            .map_err(|error| format!("获取用户列表失败: {error}"))
    }
}
