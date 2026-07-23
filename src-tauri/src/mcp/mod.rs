mod catalog;
mod policy;
mod sql;

use std::{sync::Arc, time::Duration};

use axum::{
    body::Body,
    extract::State,
    http::{header::AUTHORIZATION, Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    Router,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::CallToolResult,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    Peer, RoleServer, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlparser::{
    ast::Statement,
    dialect::{
        Dialect, GenericDialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect,
    },
    parser::Parser,
};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::db::{ConnectionConfig, DbType, QueryResult};
use catalog::{Catalog, ConnectionProfile, SavedQuery};
use policy::{QueryRisk, SqlAnalysis};

const DEFAULT_RESULT_ROWS: usize = 200;
const MAX_RESULT_ROWS: usize = 500;
const MAX_CELL_CHARS: usize = 16_384;
const MAX_CONTAINER_ITEMS: usize = 500;
const MAX_MUTATION_JSON_BYTES: usize = 1_048_576;
const MAX_SQL_BYTES: usize = 1_048_576;
const MAX_INSERT_COLUMNS: usize = 256;
const MAX_DELETE_ROWS: usize = 1_000;
const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(300);
const DATABASE_OPERATION_TIMEOUT: Duration = Duration::from_secs(60);
const MIN_HTTP_AUTH_TOKEN_BYTES: usize = 32;
const MAX_HTTP_AUTH_TOKEN_BYTES: usize = 256;

#[derive(Clone, Default)]
pub struct AstesiaMcp {
    catalog: Catalog,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum McpDbType {
    MySQL,
    PostgreSQL,
    SQLite,
    SQLServer,
    MongoDB,
    Redis,
}

impl McpDbType {
    fn into_db_type(self) -> DbType {
        match self {
            Self::MySQL => DbType::MySQL,
            Self::PostgreSQL => DbType::PostgreSQL,
            Self::SQLite => DbType::SQLite,
            Self::SQLServer => DbType::SQLServer,
            Self::MongoDB => DbType::MongoDB,
            Self::Redis => DbType::Redis,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum McpObjectType {
    Database,
    View,
    Function,
    Procedure,
    Trigger,
}

impl McpObjectType {
    fn keyword(self) -> &'static str {
        match self {
            Self::Database => "DATABASE",
            Self::View => "VIEW",
            Self::Function => "FUNCTION",
            Self::Procedure => "PROCEDURE",
            Self::Trigger => "TRIGGER",
        }
    }

    fn into_sql_kind(self) -> sql::ObjectKind {
        match self {
            Self::Database => sql::ObjectKind::Database,
            Self::View => sql::ObjectKind::View,
            Self::Function => sql::ObjectKind::Function,
            Self::Procedure => sql::ObjectKind::Procedure,
            Self::Trigger => sql::ObjectKind::Trigger,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateConnectionArgs {
    #[schemars(description = "Stable connection identifier; generated when omitted")]
    connection_id: Option<String>,
    #[schemars(description = "Human-readable connection name")]
    name: String,
    db_type: McpDbType,
    #[schemars(description = "Hostname, or the SQLite database file path")]
    host: String,
    #[schemars(description = "Database port; the driver default is used when omitted")]
    port: Option<u16>,
    username: Option<String>,
    #[schemars(
        description = "Environment variable containing the password. The secret itself must never be passed as a tool argument."
    )]
    password_env: Option<String>,
    database: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ConnectionIdArgs {
    connection_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteConnectionArgs {
    connection_id: String,
    #[schemars(
        description = "Also delete saved queries that reference this connection. Required when such queries exist."
    )]
    cascade_queries: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DatabaseObjectArgs {
    connection_id: String,
    database: String,
    object_type: McpObjectType,
    #[schemars(description = "Object name returned as a result label")]
    object_name: String,
    #[schemars(description = "Exactly one CREATE statement")]
    sql: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteDatabaseObjectArgs {
    connection_id: String,
    database: String,
    object_type: McpObjectType,
    #[schemars(description = "Exactly one DROP statement")]
    sql: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateSchemaArgs {
    connection_id: String,
    database: String,
    schema: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteSchemaArgs {
    connection_id: String,
    database: String,
    schema: String,
    #[schemars(description = "Use CASCADE where the database supports it")]
    cascade: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateTableArgs {
    connection_id: String,
    database: String,
    #[schemars(description = "Qualified table name shown in the result")]
    table: String,
    #[schemars(description = "Exactly one CREATE TABLE statement")]
    sql: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteTableArgs {
    connection_id: String,
    database: String,
    table: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateQueryArgs {
    query_id: Option<String>,
    name: String,
    connection_id: String,
    database: String,
    #[schemars(description = "Exactly one SQL statement")]
    sql: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct QueryIdArgs {
    query_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExecuteQueryArgs {
    query_id: String,
    #[schemars(description = "Maximum rows returned to the MCP client; capped at 500")]
    max_rows: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadRowsArgs {
    connection_id: String,
    database: String,
    table: String,
    page: Option<u32>,
    #[schemars(description = "Rows per page; capped at 500")]
    page_size: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct InsertRowArgs {
    connection_id: String,
    database: String,
    table: String,
    #[schemars(length(max = 256))]
    columns: Vec<String>,
    #[schemars(length(max = 256))]
    values: Vec<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpdateRowArgs {
    connection_id: String,
    database: String,
    table: String,
    primary_key_column: String,
    primary_key_value: Value,
    column: String,
    new_value: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteRowsArgs {
    connection_id: String,
    database: String,
    table: String,
    primary_key_column: String,
    #[schemars(length(min = 1, max = 1000))]
    primary_key_values: Vec<Value>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct DestructiveApproval {
    #[schemars(description = "Explicitly approve this one destructive operation")]
    confirm: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct UpdateApproval {
    #[schemars(description = "Explicitly approve this update")]
    confirm: bool,
    #[schemars(
        description = "Do not ask again for UPDATE operations on this connection and database during the current MCP session"
    )]
    do_not_ask_again: bool,
}

rmcp::elicit_safe!(DestructiveApproval, UpdateApproval);

impl AstesiaMcp {
    fn success(value: Value) -> CallToolResult {
        CallToolResult::structured(json!({ "ok": true, "result": value }))
    }

    fn failure(message: impl Into<String>) -> CallToolResult {
        CallToolResult::structured_error(json!({
            "ok": false,
            "error": message.into(),
        }))
    }

    fn default_port(db_type: &DbType) -> u16 {
        match db_type {
            DbType::MySQL => 3306,
            DbType::PostgreSQL => 5432,
            DbType::SQLite => 0,
            DbType::SQLServer => 1433,
            DbType::MongoDB => 27017,
            DbType::Redis => 6379,
        }
    }

    fn db_type_name(db_type: &DbType) -> &'static str {
        match db_type {
            DbType::MySQL => "mysql",
            DbType::PostgreSQL => "postgresql",
            DbType::SQLite => "sqlite",
            DbType::SQLServer => "sqlserver",
            DbType::MongoDB => "mongodb",
            DbType::Redis => "redis",
        }
    }

    fn validate_environment_variable(name: &str) -> Result<(), String> {
        let mut chars = name.chars();
        let first = chars
            .next()
            .ok_or_else(|| "password_env 不能为空".to_string())?;
        if !(first == '_' || first.is_ascii_alphabetic())
            || !chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        {
            return Err("password_env 必须是合法的环境变量名".to_string());
        }
        if std::env::var_os(name).is_none() {
            return Err(format!("环境变量 {name} 未设置"));
        }
        Ok(())
    }

    fn dialect(db_type: &DbType) -> Box<dyn Dialect> {
        match db_type {
            DbType::MySQL => Box::new(MySqlDialect {}),
            DbType::PostgreSQL => Box::new(PostgreSqlDialect {}),
            DbType::SQLite => Box::new(SQLiteDialect {}),
            DbType::SQLServer => Box::new(MsSqlDialect {}),
            DbType::MongoDB | DbType::Redis => Box::new(GenericDialect),
        }
    }

    fn parse_single_statement(db_type: &DbType, sql: &str) -> Result<Statement, String> {
        if sql.trim().is_empty() {
            return Err("SQL 不能为空".to_string());
        }
        let dialect = Self::dialect(db_type);
        let mut statements = Parser::parse_sql(dialect.as_ref(), sql)
            .map_err(|error| format!("SQL 解析失败: {error}"))?;
        if statements.len() != 1 {
            return Err(format!(
                "每个查询必须且只能包含一条语句，当前为 {} 条",
                statements.len()
            ));
        }
        Ok(statements.remove(0))
    }

    fn analyze_sql(db_type: &DbType, sql: &str) -> SqlAnalysis {
        let dialect = Self::dialect(db_type);
        policy::analyze_sql_with_dialect(sql, dialect.as_ref())
    }

    fn validate_no_credential_sql(
        db_type: &DbType,
        sql: &str,
        analysis: &SqlAnalysis,
    ) -> Result<(), String> {
        if analysis.risk != QueryRisk::Permissions {
            return Ok(());
        }
        let statement = Self::parse_single_statement(db_type, sql)?;
        let words = statement
            .to_string()
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .filter(|word| !word.is_empty())
            .map(str::to_ascii_uppercase)
            .collect::<Vec<_>>();
        let has_credential_clause = words.iter().any(|word| {
            matches!(
                word.as_str(),
                "PASSWORD" | "CREDENTIAL" | "SECRET" | "TOKEN"
            )
        }) || words
            .windows(2)
            .any(|pair| pair[0] == "IDENTIFIED" && pair[1] == "BY");
        if has_credential_clause {
            return Err(
                "MCP 不接受包含密码、令牌或凭据字面量的 SQL；请在受信任的数据库管理流程中配置凭据"
                    .to_string(),
            );
        }
        Ok(())
    }

    fn validate_statement_prefix(
        db_type: &DbType,
        sql: &str,
        prefix: &str,
        keyword: Option<&str>,
    ) -> Result<(), String> {
        let statement = Self::parse_single_statement(db_type, sql)?;
        let normalized = statement.to_string().to_uppercase();
        if !normalized.starts_with(prefix) {
            return Err(format!("只允许 {prefix} 语句"));
        }
        if let Some(keyword) = keyword {
            let prefix_window = normalized
                .split_whitespace()
                .take(5)
                .collect::<Vec<_>>()
                .join(" ");
            if !prefix_window.split_whitespace().any(|part| part == keyword) {
                return Err(format!("SQL 与声明的对象类型 {keyword} 不匹配"));
            }
        }
        Ok(())
    }

    fn ensure_sql_backend(db_type: &DbType) -> Result<(), String> {
        match db_type {
            DbType::MongoDB | DbType::Redis => Err(
                "该结构化 SQL 工具暂不支持 MongoDB/Redis；连接测试、访问和分页读取仍可用"
                    .to_string(),
            ),
            _ => Ok(()),
        }
    }

    fn validate_database_selector(db_type: &DbType, database: &str) -> Result<(), String> {
        if !matches!(db_type, DbType::SQLite) && database.is_empty() {
            return Err("database 不能为空".to_string());
        }
        if database.chars().any(char::is_control) {
            return Err("database 不能包含控制字符".to_string());
        }
        if matches!(
            db_type,
            DbType::MySQL | DbType::PostgreSQL | DbType::SQLite | DbType::SQLServer
        ) && database.contains('\'')
        {
            return Err("SQL database 不能包含单引号".to_string());
        }
        match db_type {
            DbType::MySQL if database.contains('`') => {
                Err("MySQL database 不能包含反引号".to_string())
            }
            DbType::SQLServer if database.contains(']') => {
                Err("SQL Server database 不能包含右方括号".to_string())
            }
            _ => Ok(()),
        }
    }

    fn validate_read_table(db_type: &DbType, table: &str) -> Result<(), String> {
        if table.is_empty() || table.chars().any(char::is_control) {
            return Err("table 不能为空或包含控制字符".to_string());
        }
        if matches!(
            db_type,
            DbType::MySQL | DbType::PostgreSQL | DbType::SQLite | DbType::SQLServer
        ) && table.contains('\'')
        {
            return Err("SQL table 不能包含单引号".to_string());
        }
        match db_type {
            DbType::MySQL if table.contains('`') => Err("MySQL table 不能包含反引号".to_string()),
            DbType::PostgreSQL | DbType::SQLite if table.contains('"') => {
                Err("PostgreSQL/SQLite table 不能包含双引号".to_string())
            }
            DbType::SQLServer if table.contains(']') => {
                Err("SQL Server table 不能包含右方括号".to_string())
            }
            _ => Ok(()),
        }
    }

    fn bounded_row_limit(requested: Option<u32>) -> usize {
        requested
            .map(|value| value as usize)
            .unwrap_or(DEFAULT_RESULT_ROWS)
            .clamp(1, MAX_RESULT_ROWS)
    }

    fn bounded_page(requested: Option<u32>, page_size: u32) -> u32 {
        let max_page = (u32::MAX / page_size).saturating_add(1);
        requested.unwrap_or(1).max(1).min(max_page)
    }

    fn validate_mutation_payload(values: &[Value]) -> Result<(), String> {
        let size = serde_json::to_vec(values)
            .map_err(|error| format!("无法验证行数据大小: {error}"))?
            .len();
        if size > MAX_MUTATION_JSON_BYTES {
            return Err("单次行修改的 JSON 数据不能超过 1 MiB".to_string());
        }
        Ok(())
    }

    fn validate_sql_size(sql: &str) -> Result<(), String> {
        if sql.len() > MAX_SQL_BYTES {
            return Err("单条 SQL 不能超过 1 MiB".to_string());
        }
        Ok(())
    }

    async fn validate_primary_key_target(
        &self,
        connection_id: &str,
        database: &str,
        table: &str,
        primary_key_column: &str,
        updated_column: Option<&str>,
    ) -> Result<(), String> {
        let profile = self.catalog.profile(connection_id).await?;
        Self::validate_database_selector(&profile.config.db_type, database)?;
        Self::validate_read_table(&profile.config.db_type, table)?;
        Self::ensure_sql_backend(&profile.config.db_type)?;

        let driver = self.catalog.driver(connection_id).await?;
        let columns = tokio::time::timeout(DATABASE_OPERATION_TIMEOUT, async {
            driver.lock().await.get_columns(database, table).await
        })
        .await
        .map_err(|_| "读取表主键信息超时（60 秒）".to_string())?
        .map_err(|error| format!("读取表主键信息失败: {error}"))?;
        let primary_keys = columns
            .iter()
            .filter(|column| column.is_primary_key)
            .collect::<Vec<_>>();
        if primary_keys.len() != 1 {
            return Err(format!(
                "结构化行修改仅支持单列主键表；当前检测到 {} 个主键列",
                primary_keys.len()
            ));
        }
        if primary_keys[0].name != primary_key_column {
            return Err(format!(
                "primary_key_column 必须是数据库元数据中的主键列 {}",
                primary_keys[0].name
            ));
        }
        if let Some(updated_column) = updated_column {
            if !columns.iter().any(|column| column.name == updated_column) {
                return Err(format!("更新列 {updated_column} 不存在"));
            }
        }
        Ok(())
    }

    fn truncate_cell(value: &mut Value) {
        match value {
            Value::String(text) if text.chars().count() > MAX_CELL_CHARS => {
                let truncated = text.chars().take(MAX_CELL_CHARS).collect::<String>();
                *text = format!("{truncated}…");
            }
            Value::Array(values) => {
                values.truncate(MAX_CONTAINER_ITEMS);
                for value in values {
                    Self::truncate_cell(value);
                }
            }
            Value::Object(values) => {
                if values.len() > MAX_CONTAINER_ITEMS {
                    let bounded = std::mem::take(values)
                        .into_iter()
                        .take(MAX_CONTAINER_ITEMS)
                        .collect();
                    *values = bounded;
                }
                for value in values.values_mut() {
                    Self::truncate_cell(value);
                }
            }
            _ => {}
        }
    }

    fn query_result_json(mut result: QueryResult, row_limit: usize) -> Value {
        let total_rows = result.rows.len();
        result.rows.truncate(row_limit);
        for row in &mut result.rows {
            for value in row {
                Self::truncate_cell(value);
            }
        }
        json!({
            "columns": result.columns,
            "rows": result.rows,
            "affected_rows": result.affected_rows,
            "execution_time_ms": result.execution_time_ms,
            "returned_rows": total_rows.min(row_limit),
            "truncated": total_rows > row_limit,
        })
    }

    async fn execute_sql(
        &self,
        connection_id: &str,
        database: &str,
        sql: &str,
    ) -> Result<QueryResult, String> {
        let profile = self.catalog.profile(connection_id).await?;
        Self::validate_database_selector(&profile.config.db_type, database)?;
        let driver = self.catalog.driver(connection_id).await?;
        let result = tokio::time::timeout(DATABASE_OPERATION_TIMEOUT, async {
            driver.lock().await.execute_query(database, sql).await
        })
        .await
        .map_err(|_| "数据库操作超时（60 秒）".to_string())?;
        result.map_err(|error| format!("数据库操作失败: {error}"))
    }

    async fn require_destructive_approval(
        &self,
        peer: &Peer<RoleServer>,
        message: String,
    ) -> Result<(), String> {
        let response = peer
            .elicit_with_timeout::<DestructiveApproval>(message, Some(CONFIRMATION_TIMEOUT))
            .await
            .map_err(|error| format!("高危操作已阻止：MCP 客户端未提供可用的用户确认（{error}）"))?
            .ok_or_else(|| "高危操作已取消：未收到确认内容".to_string())?;
        if response.confirm {
            Ok(())
        } else {
            Err("用户未确认，高危操作未执行".to_string())
        }
    }

    async fn require_update_approval(
        &self,
        peer: &Peer<RoleServer>,
        connection_id: &str,
        database: &str,
        message: String,
    ) -> Result<(), String> {
        if self
            .catalog
            .updates_are_approved(connection_id, database)
            .await
        {
            return Ok(());
        }
        let response = peer
            .elicit_with_timeout::<UpdateApproval>(message, Some(CONFIRMATION_TIMEOUT))
            .await
            .map_err(|error| format!("更新操作已阻止：MCP 客户端未提供可用的用户确认（{error}）"))?
            .ok_or_else(|| "更新操作已取消：未收到确认内容".to_string())?;
        if !response.confirm {
            return Err("用户未确认，更新操作未执行".to_string());
        }
        if response.do_not_ask_again {
            self.catalog.approve_updates(connection_id, database).await;
        }
        Ok(())
    }

    async fn approve_query_risk(
        &self,
        peer: &Peer<RoleServer>,
        query: &SavedQuery,
        analysis: &SqlAnalysis,
    ) -> Result<(), String> {
        if !analysis.requires_confirmation() {
            return Ok(());
        }
        let confirmation = analysis
            .confirmation_kind()
            .expect("confirmed risk must have a confirmation kind");
        let preview = format!(
            "连接: {}\n数据库: {}\n查询: {}\n风险: {}\nSQL:\n{}",
            query.connection_id,
            query.database,
            query.name,
            confirmation.as_str(),
            query.sql
        );
        if confirmation.allows_session_suppression() {
            self.require_update_approval(
                peer,
                &query.connection_id,
                &query.database,
                format!(
                    "即将执行 UPDATE。确认本次操作；可选择在当前 MCP 会话内不再提醒该连接的更新。\n\n{preview}"
                ),
            )
            .await
        } else {
            self.require_destructive_approval(
                peer,
                format!("即将执行不可自动豁免的高危 SQL，请逐项确认。\n\n{preview}"),
            )
            .await
        }
    }
}

#[tool_router]
impl AstesiaMcp {
    #[tool(
        description = "List MCP-session connection profiles without returning credentials.",
        annotations(
            title = "List Astesia connections",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_connections(&self) -> CallToolResult {
        let profiles = self.catalog.profiles().await;
        let mut output = Vec::with_capacity(profiles.len());
        for profile in profiles {
            output.push(json!({
                "connection_id": profile.config.id,
                "name": profile.config.name,
                "db_type": Self::db_type_name(&profile.config.db_type),
                "host": profile.config.host,
                "port": profile.config.port,
                "username": profile.config.username,
                "database": profile.config.database,
                "credential_source": profile.password_env.as_ref().map(|_| "environment"),
                "connected": self.catalog.is_connected(&profile.config.id).await,
            }));
        }
        Self::success(json!(output))
    }

    #[tool(
        description = "Create an in-memory connection profile. Pass only a password environment-variable reference, never the password itself.",
        annotations(
            title = "Create Astesia connection",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn create_connection(
        &self,
        Parameters(args): Parameters<CreateConnectionArgs>,
    ) -> CallToolResult {
        let db_type = args.db_type.into_db_type();
        let id = args
            .connection_id
            .unwrap_or_else(|| Uuid::new_v4().to_string())
            .trim()
            .to_string();
        if id.is_empty() || args.name.trim().is_empty() || args.host.trim().is_empty() {
            return Self::failure("connection_id、name 和 host 不能为空");
        }
        if let Some(variable) = args.password_env.as_deref() {
            if let Err(error) = Self::validate_environment_variable(variable) {
                return Self::failure(error);
            }
        }
        let config = ConnectionConfig {
            id: id.clone(),
            name: args.name.trim().to_string(),
            db_type: db_type.clone(),
            host: args.host.trim().to_string(),
            port: args.port.unwrap_or_else(|| Self::default_port(&db_type)),
            username: args.username.unwrap_or_default(),
            password: String::new(),
            database: args.database,
            color: None,
        };
        let profile = ConnectionProfile {
            config,
            password_env: args.password_env,
        };
        match self.catalog.insert_profile(profile).await {
            Ok(()) => Self::success(json!({ "connection_id": id })),
            Err(error) => Self::failure(error),
        }
    }

    #[tool(
        description = "Test a saved connection profile without keeping it open.",
        annotations(
            title = "Test Astesia connection",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn test_connection(
        &self,
        Parameters(args): Parameters<ConnectionIdArgs>,
    ) -> CallToolResult {
        match tokio::time::timeout(
            DATABASE_OPERATION_TIMEOUT,
            self.catalog.test_connection(&args.connection_id),
        )
        .await
        {
            Ok(Ok(())) => {
                Self::success(json!({ "connection_id": args.connection_id, "reachable": true }))
            }
            Ok(Err(error)) => Self::failure(error),
            Err(_) => Self::failure("测试连接超时（60 秒）"),
        }
    }

    #[tool(
        description = "Open a saved connection for subsequent metadata, query, and row tools.",
        annotations(
            title = "Access Astesia connection",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn connect_connection(
        &self,
        Parameters(args): Parameters<ConnectionIdArgs>,
    ) -> CallToolResult {
        match tokio::time::timeout(
            DATABASE_OPERATION_TIMEOUT,
            self.catalog.connect(&args.connection_id),
        )
        .await
        {
            Ok(Ok(opened)) => Self::success(json!({
                "connection_id": args.connection_id,
                "connected": true,
                "opened_now": opened,
            })),
            Ok(Err(error)) => Self::failure(error),
            Err(_) => Self::failure("访问连接超时（60 秒）"),
        }
    }

    #[tool(
        description = "Close an active connection while retaining its profile.",
        annotations(
            title = "Disconnect Astesia connection",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn disconnect_connection(
        &self,
        Parameters(args): Parameters<ConnectionIdArgs>,
    ) -> CallToolResult {
        match self.catalog.disconnect(&args.connection_id).await {
            Ok(closed) => Self::success(json!({
                "connection_id": args.connection_id,
                "connected": false,
                "closed_now": closed,
            })),
            Err(error) => Self::failure(error),
        }
    }

    #[tool(
        description = "Permanently delete an MCP-session connection profile after explicit user confirmation.",
        annotations(
            title = "Delete Astesia connection",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn delete_connection(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<DeleteConnectionArgs>,
    ) -> CallToolResult {
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        let query_count = self
            .catalog
            .query_count_for_connection(&args.connection_id)
            .await;
        if query_count > 0 && !args.cascade_queries {
            return Self::failure(format!(
                "连接仍被 {query_count} 个保存查询引用；如需同时删除请设置 cascade_queries=true"
            ));
        }
        let message = format!(
            "删除连接配置“{}”({})。连接将被断开，并删除 {} 个关联查询。此操作不可撤销。",
            profile.config.name, args.connection_id, query_count
        );
        if let Err(error) = self.require_destructive_approval(&peer, message).await {
            return Self::failure(error);
        }
        if let Err(error) = self.catalog.remove_profile(&args.connection_id).await {
            return Self::failure(error);
        }
        let deleted_queries = self
            .catalog
            .remove_queries_for_connection(&args.connection_id)
            .await;
        Self::success(json!({
            "connection_id": args.connection_id,
            "deleted_queries": deleted_queries,
        }))
    }

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
            return Self::failure(error);
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
            return Self::failure(error);
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
            return Self::failure(error);
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
            return Self::failure(error);
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
        let message = format!(
            "删除保存查询“{}”({})。\nSQL:\n{}",
            query.name, query.id, query.sql
        );
        if let Err(error) = self.require_destructive_approval(&peer, message).await {
            return Self::failure(error);
        }
        match self.catalog.remove_query(&args.query_id).await {
            Ok(query) => Self::success(json!({ "query_id": query.id })),
            Err(error) => Self::failure(error),
        }
    }

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
        if let Err(error) = Self::validate_read_table(&profile.config.db_type, &args.table) {
            return Self::failure(error);
        }
        let driver = match self.catalog.driver(&args.connection_id).await {
            Ok(driver) => driver,
            Err(error) => return Self::failure(error),
        };
        let result = match tokio::time::timeout(DATABASE_OPERATION_TIMEOUT, async {
            driver
                .lock()
                .await
                .get_table_data(&args.database, &args.table, page, page_size)
                .await
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
        let sql = match sql::build_insert_row(
            &profile.config.db_type,
            &args.table,
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
        if let Err(error) = self
            .validate_primary_key_target(
                &args.connection_id,
                &args.database,
                &args.table,
                &args.primary_key_column,
                Some(&args.column),
            )
            .await
        {
            return Self::failure(error);
        }
        let sql = match sql::build_update_row(
            &profile.config.db_type,
            &args.table,
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
            return Self::failure(error);
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
        if let Err(error) = self
            .validate_primary_key_target(
                &args.connection_id,
                &args.database,
                &args.table,
                &args.primary_key_column,
                None,
            )
            .await
        {
            return Self::failure(error);
        }
        let sql = match sql::build_delete_rows(
            &profile.config.db_type,
            &args.table,
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
            return Self::failure(error);
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

#[tool_handler(
    name = "astesia-mcp",
    instructions = "Use explicit connection and saved-query identifiers. Never pass credentials directly. Destructive operations require server-side user confirmation."
)]
impl ServerHandler for AstesiaMcp {}

pub async fn run_stdio() -> anyhow::Result<()> {
    let service = AstesiaMcp::default()
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

#[derive(Clone)]
struct HttpAuth {
    bearer: Arc<str>,
}

impl HttpAuth {
    fn new(token: String) -> anyhow::Result<Self> {
        validate_http_auth_token(&token)?;
        Ok(Self {
            bearer: Arc::from(format!("Bearer {token}")),
        })
    }

    fn authorizes(&self, request: &Request<Body>) -> bool {
        request
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| constant_time_eq(value.as_bytes(), self.bearer.as_bytes()))
    }
}

fn validate_http_auth_token(token: &str) -> anyhow::Result<()> {
    let valid_length =
        (MIN_HTTP_AUTH_TOKEN_BYTES..=MAX_HTTP_AUTH_TOKEN_BYTES).contains(&token.len());
    let valid_characters = token
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'));
    anyhow::ensure!(
        valid_length && valid_characters,
        "ASTESIA_MCP_AUTH_TOKEN must contain 32-256 URL-safe ASCII characters"
    );
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

async fn require_http_auth(
    State(auth): State<HttpAuth>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if auth.authorizes(&request) {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub async fn run_http(port: u16, auth_token: String) -> anyhow::Result<()> {
    let auth = HttpAuth::new(auth_token)?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let address = listener.local_addr()?;
    let endpoint = format!("http://{address}/mcp");

    let service: StreamableHttpService<AstesiaMcp, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(AstesiaMcp::default()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default().with_allowed_origins([
                format!("http://127.0.0.1:{}", address.port()),
                format!("http://localhost:{}", address.port()),
            ]),
        );
    let router = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(auth, require_http_auth));

    eprintln!("ASTESIA_MCP_READY {endpoint}");
    axum::serve(listener, router)
        .with_graceful_shutdown(wait_for_parent_stdin_close())
        .await?;
    Ok(())
}

async fn wait_for_parent_stdin_close() {
    let mut stdin = tokio::io::stdin();
    let mut discard = [0_u8; 64];

    loop {
        match stdin.read(&mut discard).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_the_complete_tool_set() {
        let tools = AstesiaMcp::tool_router().list_all();
        let names = tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(tools.len(), 20);
        for expected in [
            "list_connections",
            "create_connection",
            "test_connection",
            "connect_connection",
            "disconnect_connection",
            "delete_connection",
            "create_database_object",
            "delete_database_object",
            "create_schema",
            "delete_schema",
            "create_table",
            "delete_table",
            "list_queries",
            "create_query",
            "execute_query",
            "delete_query",
            "read_rows",
            "insert_row",
            "update_row",
            "delete_rows",
        ] {
            assert!(names.contains(&expected), "missing MCP tool {expected}");
        }
    }

    #[test]
    fn marks_high_risk_tools_as_destructive() {
        let tools = AstesiaMcp::tool_router().list_all();

        for name in [
            "delete_connection",
            "delete_database_object",
            "delete_schema",
            "delete_table",
            "execute_query",
            "delete_query",
            "update_row",
            "delete_rows",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool.name == name)
                .unwrap_or_else(|| panic!("missing MCP tool {name}"));
            let annotations = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("missing annotations for {name}"));
            assert_eq!(
                annotations.destructive_hint,
                Some(true),
                "{name} must advertise destructive behavior"
            );
        }
    }

    #[test]
    fn connection_schema_accepts_only_a_credential_reference() {
        let tools = AstesiaMcp::tool_router().list_all();
        let tool = tools
            .iter()
            .find(|tool| tool.name == "create_connection")
            .expect("create_connection tool");
        let schema = serde_json::to_value(&tool.input_schema).expect("serialize input schema");
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("input schema properties");

        assert!(properties.contains_key("password_env"));
        assert!(!properties.contains_key("password"));
    }

    #[test]
    fn validates_http_auth_tokens() {
        assert!(validate_http_auth_token(&"a".repeat(32)).is_ok());
        assert!(validate_http_auth_token(&format!("{}-_.~", "b".repeat(32))).is_ok());
        assert!(validate_http_auth_token("short").is_err());
        assert!(validate_http_auth_token(&"a".repeat(MAX_HTTP_AUTH_TOKEN_BYTES + 1)).is_err());
        assert!(validate_http_auth_token(&format!("{} token", "a".repeat(32))).is_err());
    }

    #[test]
    fn compares_authorization_values_without_prefix_or_length_shortcuts() {
        assert!(constant_time_eq(b"Bearer secret", b"Bearer secret"));
        assert!(!constant_time_eq(b"Bearer secret", b"Bearer other!"));
        assert!(!constant_time_eq(b"Bearer secret", b"Bearer secret-longer"));
    }

    #[test]
    fn rejects_delimiters_that_legacy_driver_quoting_cannot_escape() {
        assert!(AstesiaMcp::validate_database_selector(
            &DbType::MySQL,
            "analytics` ; DROP DATABASE x"
        )
        .is_err());
        assert!(AstesiaMcp::validate_database_selector(
            &DbType::SQLServer,
            "analytics] ; DROP DATABASE x"
        )
        .is_err());
        assert!(
            AstesiaMcp::validate_read_table(&DbType::PostgreSQL, "users\"; DELETE FROM users")
                .is_err()
        );
        assert!(AstesiaMcp::validate_read_table(&DbType::MySQL, "order_items").is_ok());
    }

    #[test]
    fn bounds_page_offsets_before_the_driver_multiplies_them() {
        let page = AstesiaMcp::bounded_page(Some(u32::MAX), MAX_RESULT_ROWS as u32);
        assert!((page - 1).checked_mul(MAX_RESULT_ROWS as u32).is_some());
    }

    #[test]
    fn rejects_credential_bearing_permission_sql() {
        let sql = "CREATE USER ada WITH PASSWORD 'do-not-store-this'";
        let analysis = AstesiaMcp::analyze_sql(&DbType::PostgreSQL, sql);
        assert!(
            AstesiaMcp::validate_no_credential_sql(&DbType::PostgreSQL, sql, &analysis).is_err()
        );
    }
}
