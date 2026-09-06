use crate::db::{
    ColumnInfo, ForeignKeyInfo, FunctionInfo, IndexInfo, ProcedureInfo, TableInfo, TableRef,
    TriggerInfo, UnsupportedFeature, UserInfo, ViewInfo,
};

use super::connections::ConnectionManager;
use super::{
    CatalogEntry, CatalogKind, CatalogSection, TableStructureLoadError, TableStructureSnapshot,
};

#[derive(Clone)]
pub struct CatalogService {
    manager: ConnectionManager,
}

impl CatalogService {
    pub(super) fn new(manager: ConnectionManager) -> Self {
        Self { manager }
    }

    pub(crate) async fn refresh_schema(
        &self,
        connection_id: &str,
        database: Option<&str>,
    ) -> Result<(), String> {
        self.manager
            .schema_cache
            .invalidate(connection_id, database)
            .await
    }

    pub async fn databases(&self, connection_id: &str) -> Result<Vec<String>, String> {
        self.manager
            .cached_schema(connection_id, "", "databases".into(), async {
                let handle = self.manager.driver(connection_id).await?;
                let driver = handle.lock_active().await?;
                driver
                    .get_databases()
                    .await
                    .map_err(|error| format!("获取数据库列表失败: {error}"))
            })
            .await
    }

    pub async fn tables(
        &self,
        connection_id: &str,
        database: &str,
    ) -> Result<Vec<TableInfo>, String> {
        self.manager
            .cached_schema(connection_id, database, "tables".into(), async {
                let handle = self.manager.driver(connection_id).await?;
                let driver = handle.lock_active().await?;
                driver
                    .get_tables(database)
                    .await
                    .map_err(|error| format!("获取表列表失败: {error}"))
            })
            .await
    }

    pub(crate) async fn catalog_section(
        &self,
        connection_id: &str,
        database: &str,
        kind: CatalogKind,
    ) -> CatalogEntry {
        match kind {
            CatalogKind::Schemas => CatalogEntry::Schemas(CatalogSection::from_result(
                self.schemas(connection_id, database).await,
            )),
            CatalogKind::Tables => CatalogEntry::Tables(CatalogSection::from_result(
                self.tables(connection_id, database).await,
            )),
            CatalogKind::Views => CatalogEntry::Views(CatalogSection::from_result(
                self.views(connection_id, database).await,
            )),
            CatalogKind::Functions => CatalogEntry::Functions(CatalogSection::from_result(
                self.functions(connection_id, database).await,
            )),
            CatalogKind::Procedures => CatalogEntry::Procedures(CatalogSection::from_result(
                self.procedures(connection_id, database).await,
            )),
            CatalogKind::Triggers => CatalogEntry::Triggers(CatalogSection::from_result(
                self.triggers(connection_id, database).await,
            )),
            CatalogKind::Users => {
                CatalogEntry::Users(CatalogSection::from_result(self.users(connection_id).await))
            }
        }
    }

    pub async fn columns(
        &self,
        connection_id: &str,
        database: &str,
        table: &TableRef,
    ) -> Result<Vec<ColumnInfo>, String> {
        self.manager
            .cached_schema(
                connection_id,
                database,
                serde_json::to_string(&("columns", table)).expect("metadata cache key"),
                async {
                    let handle = self.manager.driver(connection_id).await?;
                    let driver = handle.lock_active().await?;
                    driver
                        .get_columns(database, table)
                        .await
                        .map_err(|error| format!("获取列信息失败: {error}"))
                },
            )
            .await
    }

    pub async fn constraints(
        &self,
        connection_id: &str,
        database: &str,
        table: &TableRef,
    ) -> Result<Vec<crate::db::ConstraintInfo>, String> {
        self.manager
            .cached_schema(
                connection_id,
                database,
                serde_json::to_string(&("constraints", table)).expect("metadata cache key"),
                async {
                    let handle = self.manager.driver(connection_id).await?;
                    let driver = handle.lock_active().await?;
                    driver
                        .get_constraints(database, table)
                        .await
                        .map_err(|error| error.to_string())
                },
            )
            .await
    }

    pub async fn indexes(
        &self,
        connection_id: &str,
        database: &str,
        table: &TableRef,
    ) -> Result<Vec<IndexInfo>, String> {
        self.manager
            .cached_schema(
                connection_id,
                database,
                serde_json::to_string(&("indexes", table)).expect("metadata cache key"),
                async {
                    let handle = self.manager.driver(connection_id).await?;
                    let driver = handle.lock_active().await?;
                    driver
                        .get_indexes(database, table)
                        .await
                        .map_err(|error| format!("获取索引信息失败: {error}"))
                },
            )
            .await
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
        drop(driver);
        let columns = self
            .columns(connection_id, database, table)
            .await
            .map_err(TableStructureLoadError::Columns)?;
        let indexes = self
            .indexes(connection_id, database, table)
            .await
            .map_err(TableStructureLoadError::Indexes)?;
        let constraints = if capabilities.constraints {
            Some(
                self.constraints(connection_id, database, table)
                    .await
                    .map_err(TableStructureLoadError::Constraints)?,
            )
        } else {
            None
        };
        let foreign_keys = if capabilities.foreign_keys {
            Some(
                self.foreign_keys(connection_id, database, table)
                    .await
                    .map_err(TableStructureLoadError::ForeignKeys)?,
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
        self.manager
            .cached_schema(connection_id, database, "schemas".into(), async {
                let handle = self.manager.driver(connection_id).await?;
                let driver = handle.lock_active().await?;
                driver
                    .get_schemas(database)
                    .await
                    .map_err(|error| format!("获取Schema列表失败: {error}"))
            })
            .await
    }

    pub async fn enum_values(
        &self,
        connection_id: &str,
        database: &str,
        enum_type: &str,
    ) -> Result<Vec<String>, String> {
        self.manager
            .cached_schema(
                connection_id,
                database,
                serde_json::to_string(&("enum_values", enum_type)).expect("metadata cache key"),
                async {
                    let handle = self.manager.driver(connection_id).await?;
                    let driver = handle.lock_active().await?;
                    driver
                        .get_enum_values(database, enum_type)
                        .await
                        .map_err(|error| format!("获取枚举值失败: {error}"))
                },
            )
            .await
    }

    pub async fn views(
        &self,
        connection_id: &str,
        database: &str,
    ) -> Result<Vec<ViewInfo>, String> {
        self.manager
            .cached_schema(connection_id, database, "views".into(), async {
                let handle = self.manager.driver(connection_id).await?;
                let driver = handle.lock_active().await?;
                driver
                    .get_views(database)
                    .await
                    .map_err(|error| format!("获取视图失败: {error}"))
            })
            .await
    }

    pub async fn functions(
        &self,
        connection_id: &str,
        database: &str,
    ) -> Result<Vec<FunctionInfo>, String> {
        self.manager
            .cached_schema(connection_id, database, "functions".into(), async {
                let handle = self.manager.driver(connection_id).await?;
                let driver = handle.lock_active().await?;
                driver
                    .get_functions(database)
                    .await
                    .map_err(|error| format!("获取函数失败: {error}"))
            })
            .await
    }

    pub async fn procedures(
        &self,
        connection_id: &str,
        database: &str,
    ) -> Result<Vec<ProcedureInfo>, String> {
        self.manager
            .cached_schema(connection_id, database, "procedures".into(), async {
                let handle = self.manager.driver(connection_id).await?;
                let driver = handle.lock_active().await?;
                driver
                    .get_procedures(database)
                    .await
                    .map_err(|error| format!("获取存储过程失败: {error}"))
            })
            .await
    }

    pub async fn triggers(
        &self,
        connection_id: &str,
        database: &str,
    ) -> Result<Vec<TriggerInfo>, String> {
        self.manager
            .cached_schema(connection_id, database, "triggers".into(), async {
                let handle = self.manager.driver(connection_id).await?;
                let driver = handle.lock_active().await?;
                driver
                    .get_triggers(database)
                    .await
                    .map_err(|error| format!("获取触发器失败: {error}"))
            })
            .await
    }

    pub async fn foreign_keys(
        &self,
        connection_id: &str,
        database: &str,
        table: &TableRef,
    ) -> Result<Vec<ForeignKeyInfo>, String> {
        self.manager
            .cached_schema(
                connection_id,
                database,
                serde_json::to_string(&("foreign_keys", table)).expect("metadata cache key"),
                async {
                    let handle = self.manager.driver(connection_id).await?;
                    let driver = handle.lock_active().await?;
                    driver
                        .get_foreign_keys(database, table)
                        .await
                        .map_err(|error| format!("获取外键失败: {error}"))
                        .map(|keys| {
                            keys.into_iter()
                                .map(|key| {
                                    (
                                        key.name,
                                        key.from_table,
                                        key.from_columns,
                                        key.to_table,
                                        key.to_columns,
                                    )
                                })
                                .collect::<Vec<_>>()
                        })
                },
            )
            .await
            .map(|keys| {
                keys.into_iter()
                    .map(
                        |(name, from_table, from_columns, to_table, to_columns)| ForeignKeyInfo {
                            name,
                            from_table,
                            from_columns,
                            to_table,
                            to_columns,
                        },
                    )
                    .collect()
            })
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
