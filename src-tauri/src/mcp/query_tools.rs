use rmcp::{
    handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router, Peer,
    RoleServer,
};
use serde_json::json;
use uuid::Uuid;

use super::{
    catalog::SavedQuery,
    protocol::{CreateQueryArgs, ExecuteQueryArgs, QueryIdArgs},
    AstesiaMcp,
};

#[tool_router(router = query_tools_router, vis = "pub(super)")]
impl AstesiaMcp {
    #[tool(
        description = "List saved query definitions in the current MCP session.",
        annotations(
            title = "List Astesia queries",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_queries(&self) -> CallToolResult {
        Self::success(json!(self.catalog.queries().await))
    }

    #[tool(
        description = "Save exactly one parsed SQL statement for later execution.",
        annotations(
            title = "Create Astesia query",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_query(&self, Parameters(args): Parameters<CreateQueryArgs>) -> CallToolResult {
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
        if let Err(error) = Self::ensure_sql_backend(&profile.config.db_type) {
            return Self::failure(error);
        }
        let analysis = Self::analyze_sql(&profile.config.db_type, &args.sql);
        if !analysis.is_single_statement() {
            return Self::failure(
                analysis
                    .parse_error
                    .as_deref()
                    .unwrap_or("查询必须包含且仅包含一条 SQL 语句"),
            );
        }
        if let Err(error) =
            Self::validate_no_credential_sql(&profile.config.db_type, &args.sql, &analysis)
        {
            return Self::failure(error);
        }
        let query_id = args
            .query_id
            .unwrap_or_else(|| Uuid::new_v4().to_string())
            .trim()
            .to_string();
        if query_id.is_empty() || args.name.trim().is_empty() {
            return Self::failure("query_id 和 name 不能为空");
        }
        let query = SavedQuery {
            id: query_id.clone(),
            name: args.name.trim().to_string(),
            connection_id: args.connection_id,
            database: args.database,
            sql: args.sql.trim().to_string(),
        };
        match self.catalog.insert_query(query).await {
            Ok(()) => Self::success(json!({
                "query_id": query_id,
                "risk": analysis.risk.as_str(),
            })),
            Err(error) => Self::failure(error),
        }
    }

    #[tool(
        description = "Execute a saved query. The server classifies the SQL and confirms UPDATE, DELETE, permission, destructive DDL, and unknown statements before execution.",
        annotations(
            title = "Execute Astesia query",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn execute_query(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<ExecuteQueryArgs>,
    ) -> CallToolResult {
        let query = match self.catalog.query(&args.query_id).await {
            Ok(query) => query,
            Err(error) => return Self::failure(error),
        };
        let _lifecycle = self
            .catalog
            .lock_connection_lifecycle(&query.connection_id)
            .await;
        let profile = match self.catalog.profile(&query.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        let analysis = Self::analyze_sql(&profile.config.db_type, &query.sql);
        if !analysis.is_single_statement() {
            return Self::failure(
                analysis
                    .parse_error
                    .as_deref()
                    .unwrap_or("仅允许执行单条 SQL"),
            );
        }
        if let Err(error) =
            Self::validate_no_credential_sql(&profile.config.db_type, &query.sql, &analysis)
        {
            return Self::failure(error);
        }
        if let Err(error) = self.approve_query_risk(&peer, &query, &analysis).await {
            return error.into();
        }
        let row_limit = Self::bounded_row_limit(args.max_rows);
        match self
            .execute_sql(&query.connection_id, &query.database, &query.sql)
            .await
        {
            Ok(result) => Self::success(json!({
                "query_id": query.id,
                "risk": analysis.risk.as_str(),
                "execution": Self::query_result_json(result, row_limit),
            })),
            Err(error) => Self::failure(error),
        }
    }

    #[tool(
        description = "Delete a saved query definition after explicit user confirmation.",
        annotations(
            title = "Delete Astesia query",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn delete_query(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<QueryIdArgs>,
    ) -> CallToolResult {
        let query = match self.catalog.query(&args.query_id).await {
            Ok(query) => query,
            Err(error) => return Self::failure(error),
        };
        let _lifecycle = self
            .catalog
            .lock_connection_lifecycle(&query.connection_id)
            .await;
        let message = format!(
            "删除保存查询“{}”({})。\nSQL:\n{}",
            query.name, query.id, query.sql
        );
        if let Err(error) = self.require_destructive_approval(&peer, message).await {
            return error.into();
        }
        match self.catalog.remove_query_if_unchanged(&query).await {
            Ok(query) => Self::success(json!({ "query_id": query.id })),
            Err(error) => Self::failure(error),
        }
    }
}
