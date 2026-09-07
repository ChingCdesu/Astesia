use rmcp::{
    handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router, Peer,
    RoleServer,
};
use serde_json::json;

use crate::db::TableRef;

use super::{
    execution::{DATABASE_OPERATION_TIMEOUT, MAX_DELETE_ROWS, MAX_INSERT_COLUMNS},
    protocol::{DeleteRowsArgs, InsertRowArgs, ReadRowsArgs, UpdateRowArgs},
    sql, AstesiaMcp,
};

#[tool_router(router = row_tools_router, vis = "pub(super)")]
impl AstesiaMcp {
    #[tool(
        description = "Read one bounded page of rows from a table or collection.",
        annotations(
            title = "Read database rows",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn read_rows(&self, Parameters(args): Parameters<ReadRowsArgs>) -> CallToolResult {
        let page_size = Self::bounded_row_limit(args.page_size) as u32;
        let page = Self::bounded_page(args.page, page_size);
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        if let Err(error) =
            Self::validate_database_selector(&profile.config.db_type, &args.database)
        {
            return Self::failure(error);
        }
        if let Err(error) = Self::validate_read_table(&args.table) {
            return Self::failure(error);
        }
        let table = match TableRef::parse(profile.config.db_type, &args.table) {
            Ok(table) => table,
            Err(error) => return Self::failure(error.to_string()),
        };
        let driver = match self.catalog.driver(&args.connection_id).await {
            Ok(driver) => driver,
            Err(error) => return Self::failure(error),
        };
        let result = match tokio::time::timeout(DATABASE_OPERATION_TIMEOUT, async {
            let driver = driver.lock_active().await?;
            driver
                .get_table_data(&args.database, &table, page, page_size)
                .await
                .map_err(|error| error.to_string())
        })
        .await
        {
            Ok(result) => result,
            Err(_) => return Self::failure("读取行超时（60 秒）"),
        };
        match result {
            Ok(result) => Self::success(Self::query_result_json(result, page_size as usize)),
            Err(error) => Self::failure(format!("读取行失败: {error}")),
        }
    }

    #[tool(
        description = "Insert one row into a SQL table using validated identifiers and escaped values.",
        annotations(
            title = "Insert database row",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn insert_row(&self, Parameters(args): Parameters<InsertRowArgs>) -> CallToolResult {
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        if args.columns.len() > MAX_INSERT_COLUMNS {
            return Self::failure(format!("单次插入最多支持 {MAX_INSERT_COLUMNS} 个列值"));
        }
        if let Err(error) = Self::validate_mutation_payload(&args.values) {
            return Self::failure(error);
        }
        let table = match TableRef::parse(profile.config.db_type, &args.table) {
            Ok(table) => table,
            Err(error) => return Self::failure(error.to_string()),
        };
        let sql = match sql::build_insert_row_for_table(
            &profile.config.db_type,
            &table,
            &args.columns,
            &args.values,
        ) {
            Ok(sql) => sql,
            Err(error) => return Self::failure(error),
        };
        match self
            .execute_sql(&args.connection_id, &args.database, &sql)
            .await
        {
            Ok(result) => Self::success(json!({
                "affected_rows": result.affected_rows,
                "execution_time_ms": result.execution_time_ms,
            })),
            Err(error) => Self::failure(error),
        }
    }

    #[tool(
        description = "Update one column in one row. User confirmation can be suppressed only for updates on this connection and database during the current MCP session.",
        annotations(
            title = "Update database row",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn update_row(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<UpdateRowArgs>,
    ) -> CallToolResult {
        let _lifecycle = self
            .catalog
            .lock_connection_lifecycle(&args.connection_id)
            .await;
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        if let Err(error) = Self::validate_mutation_payload(&[
            args.primary_key_value.clone(),
            args.new_value.clone(),
        ]) {
            return Self::failure(error);
        }
        let table = match self
            .validate_primary_key_target(
                &args.connection_id,
                &args.database,
                &args.table,
                &args.primary_key_column,
                Some(&args.column),
            )
            .await
        {
            Ok(table) => table,
            Err(error) => return Self::failure(error),
        };
        let sql = match sql::build_update_row_for_table(
            &profile.config.db_type,
            &table,
            &args.primary_key_column,
            &args.primary_key_value,
            &args.column,
            &args.new_value,
        ) {
            Ok(sql) => sql,
            Err(error) => return Self::failure(error),
        };
        let message = format!(
            "更新表 {} 的列 {}。\n连接: {}\n数据库: {}\nSQL:\n{}",
            args.table, args.column, args.connection_id, args.database, sql
        );
        if let Err(error) = self
            .require_update_approval(&peer, &args.connection_id, &args.database, message)
            .await
        {
            return error.into();
        }
        match self
            .execute_sql(&args.connection_id, &args.database, &sql)
            .await
        {
            Ok(result) => Self::success(json!({
                "affected_rows": result.affected_rows,
                "execution_time_ms": result.execution_time_ms,
            })),
            Err(error) => Self::failure(error),
        }
    }

    #[tool(
        description = "Delete rows selected by primary-key values after explicit user confirmation.",
        annotations(
            title = "Delete database rows",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn delete_rows(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<DeleteRowsArgs>,
    ) -> CallToolResult {
        let _lifecycle = self
            .catalog
            .lock_connection_lifecycle(&args.connection_id)
            .await;
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        if args.primary_key_values.len() > MAX_DELETE_ROWS {
            return Self::failure(format!("单次删除最多支持 {MAX_DELETE_ROWS} 个主键值"));
        }
        if let Err(error) = Self::validate_mutation_payload(&args.primary_key_values) {
            return Self::failure(error);
        }
        let table = match self
            .validate_primary_key_target(
                &args.connection_id,
                &args.database,
                &args.table,
                &args.primary_key_column,
                None,
            )
            .await
        {
            Ok(table) => table,
            Err(error) => return Self::failure(error),
        };
        let sql = match sql::build_delete_rows_for_table(
            &profile.config.db_type,
            &table,
            &args.primary_key_column,
            &args.primary_key_values,
        ) {
            Ok(sql) => sql,
            Err(error) => return Self::failure(error),
        };
        let message = format!(
            "从表 {} 删除 {} 行候选记录。\n连接: {}\n数据库: {}\nSQL:\n{}",
            args.table,
            args.primary_key_values.len(),
            args.connection_id,
            args.database,
            sql
        );
        if let Err(error) = self.require_destructive_approval(&peer, message).await {
            return error.into();
        }
        match self
            .execute_sql(&args.connection_id, &args.database, &sql)
            .await
        {
            Ok(result) => Self::success(json!({
                "affected_rows": result.affected_rows,
                "execution_time_ms": result.execution_time_ms,
            })),
            Err(error) => Self::failure(error),
        }
    }
}
