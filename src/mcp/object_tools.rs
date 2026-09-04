use rmcp::{
    handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router, Peer,
    RoleServer,
};
use serde_json::json;

use super::{
    execution::DEFAULT_RESULT_ROWS,
    protocol::{
        CreateSchemaArgs, CreateTableArgs, DatabaseObjectArgs, DeleteDatabaseObjectArgs,
        DeleteSchemaArgs, DeleteTableArgs,
    },
    sql, AstesiaMcp,
};

#[tool_router(router = object_tools_router, vis = "pub(super)")]
impl AstesiaMcp {
    #[tool(
        description = "Create one database object from an explicit CREATE statement.",
        annotations(
            title = "Create database object",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_database_object(
        &self,
        Parameters(args): Parameters<DatabaseObjectArgs>,
    ) -> CallToolResult {
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        if let Err(error) = Self::validate_sql_size(&args.sql) {
            return Self::failure(error);
        }
        let sql = match sql::build_create_object(
            &profile.config.db_type,
            args.object_type.into_sql_kind(),
            &args.sql,
        ) {
            Ok(sql) => sql,
            Err(error) => return Self::failure(error),
        };
        match self
            .execute_sql(&args.connection_id, &args.database, &sql)
            .await
        {
            Ok(result) => Self::success(json!({
                "object_type": args.object_type.keyword().to_lowercase(),
                "object_name": args.object_name,
                "execution": Self::query_result_json(result, DEFAULT_RESULT_ROWS),
            })),
            Err(error) => Self::failure(error),
        }
    }

    #[tool(
        description = "Drop one database object from an explicit DROP statement after confirmation.",
        annotations(
            title = "Delete database object",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn delete_database_object(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<DeleteDatabaseObjectArgs>,
    ) -> CallToolResult {
        let _lifecycle = self
            .catalog
            .lock_connection_lifecycle(&args.connection_id)
            .await;
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        if let Err(error) = Self::validate_sql_size(&args.sql) {
            return Self::failure(error);
        }
        if let Err(error) = sql::ensure_object_operation(
            &profile.config.db_type,
            args.object_type.into_sql_kind(),
            "drop",
        ) {
            return Self::failure(error);
        }
        if let Err(error) = Self::validate_statement_prefix(
            &profile.config.db_type,
            &args.sql,
            "DROP",
            Some(args.object_type.keyword()),
        ) {
            return Self::failure(error);
        }
        let message = format!(
            "删除数据库对象 {}。\n连接: {}\n数据库: {}\n将执行以下完整 SQL，请核对实际目标：\n{}",
            args.object_type.keyword(),
            args.connection_id,
            args.database,
            args.sql
        );
        if let Err(error) = self.require_destructive_approval(&peer, message).await {
            return error.into();
        }
        match self
            .execute_sql(&args.connection_id, &args.database, &args.sql)
            .await
        {
            Ok(result) => Self::success(Self::query_result_json(result, DEFAULT_RESULT_ROWS)),
            Err(error) => Self::failure(error),
        }
    }

    #[tool(
        description = "Create a schema using dialect-aware identifier quoting.",
        annotations(
            title = "Create database schema",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_schema(
        &self,
        Parameters(args): Parameters<CreateSchemaArgs>,
    ) -> CallToolResult {
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        let sql = match sql::build_create_schema(&profile.config.db_type, &args.schema) {
            Ok(sql) => sql,
            Err(error) => return Self::failure(error),
        };
        match self
            .execute_sql(&args.connection_id, &args.database, &sql)
            .await
        {
            Ok(result) => Self::success(json!({
                "schema": args.schema,
                "sql": sql,
                "execution": Self::query_result_json(result, DEFAULT_RESULT_ROWS),
            })),
            Err(error) => Self::failure(error),
        }
    }

    #[tool(
        description = "Drop a schema after explicit user confirmation.",
        annotations(
            title = "Delete database schema",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn delete_schema(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<DeleteSchemaArgs>,
    ) -> CallToolResult {
        let _lifecycle = self
            .catalog
            .lock_connection_lifecycle(&args.connection_id)
            .await;
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        let sql = match sql::build_drop_schema(&profile.config.db_type, &args.schema, args.cascade)
        {
            Ok(sql) => sql,
            Err(error) => return Self::failure(error),
        };
        let message = format!(
            "删除模式 {}。\n连接: {}\n数据库: {}\nSQL:\n{}",
            args.schema, args.connection_id, args.database, sql
        );
        if let Err(error) = self.require_destructive_approval(&peer, message).await {
            return error.into();
        }
        match self
            .execute_sql(&args.connection_id, &args.database, &sql)
            .await
        {
            Ok(result) => Self::success(Self::query_result_json(result, DEFAULT_RESULT_ROWS)),
            Err(error) => Self::failure(error),
        }
    }

    #[tool(
        description = "Create a table from exactly one CREATE TABLE statement.",
        annotations(
            title = "Create database table",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_table(&self, Parameters(args): Parameters<CreateTableArgs>) -> CallToolResult {
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        if let Err(error) = Self::validate_sql_size(&args.sql) {
            return Self::failure(error);
        }
        let sql = match sql::build_create_table(&profile.config.db_type, &args.sql) {
            Ok(sql) => sql,
            Err(error) => return Self::failure(error),
        };
        match self
            .execute_sql(&args.connection_id, &args.database, &sql)
            .await
        {
            Ok(result) => Self::success(json!({
                "table": args.table,
                "execution": Self::query_result_json(result, DEFAULT_RESULT_ROWS),
            })),
            Err(error) => Self::failure(error),
        }
    }

    #[tool(
        description = "Drop a table after explicit user confirmation.",
        annotations(
            title = "Delete database table",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn delete_table(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<DeleteTableArgs>,
    ) -> CallToolResult {
        let _lifecycle = self
            .catalog
            .lock_connection_lifecycle(&args.connection_id)
            .await;
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        let sql = match sql::build_drop_table(&profile.config.db_type, &args.table) {
            Ok(sql) => sql,
            Err(error) => return Self::failure(error),
        };
        let message = format!(
            "删除表 {}。\n连接: {}\n数据库: {}\nSQL:\n{}",
            args.table, args.connection_id, args.database, sql
        );
        if let Err(error) = self.require_destructive_approval(&peer, message).await {
            return error.into();
        }
        match self
            .execute_sql(&args.connection_id, &args.database, &sql)
            .await
        {
            Ok(result) => Self::success(Self::query_result_json(result, DEFAULT_RESULT_ROWS)),
            Err(error) => Self::failure(error),
        }
    }
}
