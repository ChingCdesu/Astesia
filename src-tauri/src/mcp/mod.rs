mod catalog;
mod policy;
mod sql;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex, Weak},
    time::Duration,
};

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
    service::ElicitationError,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    Peer, RoleServer, ServerHandler, ServiceError, ServiceExt,
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
use tokio::{io::AsyncReadExt, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::connection_repository::{
    ConnectionRepositoryError, CredentialVerificationReport, SharedConnectionRepository,
};
use crate::db::{DbType, QueryResult};
use crate::mcp_sync::{McpControlCommand, McpSyncClient, McpSyncConfig};
use catalog::{Catalog, CatalogError, SavedQuery};
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
const ERROR_CODE_TOOL_FAILED: &str = "astesia.tool.failed";
const ERROR_CODE_CONNECTION_NOT_FOUND: &str = "astesia.connection.not_found";
const ERROR_CODE_APPROVAL_UNSUPPORTED: &str = "astesia.approval.unsupported";
const ERROR_CODE_APPROVAL_DECLINED: &str = "astesia.approval.declined";
const ERROR_CODE_APPROVAL_CANCELLED: &str = "astesia.approval.cancelled";
const ERROR_CODE_APPROVAL_TIMEOUT: &str = "astesia.approval.timeout";
const ERROR_CODE_APPROVAL_INVALID_RESPONSE: &str = "astesia.approval.invalid_response";
const ERROR_CODE_APPROVAL_UNAVAILABLE: &str = "astesia.approval.unavailable";
pub(crate) const CREDENTIAL_VERIFY_MARKER: &str = "ASTESIA_SHARED_CREDENTIALS_VERIFIED ";

#[derive(Clone)]
pub struct AstesiaMcp {
    active_tests: ActiveConnectionTests,
    catalog: Catalog,
    sync: Option<McpSyncClient>,
    _control_loop: Option<Arc<ControlLoopTask>>,
}

#[derive(Clone, Default)]
struct ActiveConnectionTests {
    inner: Arc<StdMutex<HashMap<String, Weak<ActiveConnectionTest>>>>,
}

struct ActiveConnectionTest {
    generation: u64,
    owns_sync_ownership: bool,
    cancellation: CancellationToken,
    future_dropped: CancellationToken,
}

impl ActiveConnectionTest {
    fn new(generation: u64, owns_sync_ownership: bool) -> Self {
        Self {
            generation,
            owns_sync_ownership,
            cancellation: CancellationToken::new(),
            future_dropped: CancellationToken::new(),
        }
    }

    fn cancel(&self) {
        self.cancellation.cancel();
    }

    async fn wait_until_future_dropped(&self) {
        self.future_dropped.cancelled().await;
    }

    fn mark_future_dropped(&self) {
        self.future_dropped.cancel();
    }
}

impl ActiveConnectionTests {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Weak<ActiveConnectionTest>>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn current(&self, connection_id: &str) -> Option<Arc<ActiveConnectionTest>> {
        let mut active = self.lock();
        let current = active.get(connection_id).and_then(Weak::upgrade);
        if current.is_none() {
            active.remove(connection_id);
        }
        current
    }

    fn register(
        &self,
        connection_id: &str,
        generation: u64,
        owns_sync_ownership: bool,
        pending_release: Option<(McpSyncClient, String)>,
    ) -> Result<ActiveConnectionTestGuard, String> {
        let mut active = self.lock();
        if active.get(connection_id).and_then(Weak::upgrade).is_some() {
            return Err(format!(
                "连接 {connection_id} 已有并发测试，请等待其完成后重试"
            ));
        }
        let test = Arc::new(ActiveConnectionTest::new(generation, owns_sync_ownership));
        active.insert(connection_id.to_string(), Arc::downgrade(&test));
        let pending_release = pending_release.map(|(sync, owned_connection_id)| {
            PendingTestRelease::new(sync, owned_connection_id, generation, test.clone())
        });
        Ok(ActiveConnectionTestGuard {
            test,
            pending_release,
        })
    }
}

#[derive(Clone)]
struct ActiveTestMarker {
    _state: Arc<ActiveConnectionTest>,
}

impl ActiveTestMarker {
    fn new(state: Arc<ActiveConnectionTest>) -> Self {
        Self { _state: state }
    }
}

struct PendingTestRelease {
    sync: McpSyncClient,
    marker: ActiveTestMarker,
    connection_id: String,
    generation: u64,
    armed: bool,
}

impl PendingTestRelease {
    fn new(
        sync: McpSyncClient,
        connection_id: String,
        generation: u64,
        active_test: Arc<ActiveConnectionTest>,
    ) -> Self {
        Self {
            sync,
            marker: ActiveTestMarker::new(active_test),
            connection_id,
            generation,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingTestRelease {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let sync = self.sync.clone();
        let connection_id = self.connection_id.clone();
        let generation = self.generation;
        // Keep the Weak-map marker live until the delayed Released finishes,
        // otherwise a new connect could race with the old generation (ABA).
        let marker = self.marker.clone();
        runtime.spawn(async move {
            let _marker = marker;
            if let Err(error) = sync.released(connection_id, generation).await {
                log::debug!(
                    "Unable to release an abandoned HTTP connection test generation: {error}"
                );
            }
        });
    }
}

struct ActiveConnectionTestGuard {
    test: Arc<ActiveConnectionTest>,
    pending_release: Option<PendingTestRelease>,
}

impl std::ops::Deref for ActiveConnectionTestGuard {
    type Target = ActiveConnectionTest;

    fn deref(&self) -> &Self::Target {
        &self.test
    }
}

impl ActiveConnectionTestGuard {
    fn disarm_pending_release(&mut self) {
        if let Some(mut pending_release) = self.pending_release.take() {
            pending_release.disarm();
        }
    }
}

impl Drop for ActiveConnectionTestGuard {
    fn drop(&mut self) {
        // Dropping an MCP request future must always tell App controls that
        // the database test future (and its shared OS lease) is gone.
        self.test.mark_future_dropped();
    }
}
struct ControlLoopTask {
    handle: JoinHandle<()>,
}

impl Drop for ControlLoopTask {
    fn drop(&mut self) {
        self.handle.abort();
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
struct ConnectionIdArgs {
    connection_id: String,
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

#[derive(Debug, Clone, PartialEq)]
struct McpToolFailure {
    code: String,
    message: String,
    remediation: Option<String>,
    retryable: bool,
    details: Value,
}

impl McpToolFailure {
    fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        details: Value,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            remediation: None,
            retryable,
            details,
        }
    }

    fn from_repository(error: ConnectionRepositoryError) -> Self {
        Self {
            code: error.code.as_str().to_string(),
            message: error.message,
            remediation: Some(error.remediation),
            retryable: error.retryable,
            details: error.details,
        }
    }

    fn with_details(mut self, details: Value) -> Self {
        if let (Some(current), Some(additional)) =
            (self.details.as_object_mut(), details.as_object())
        {
            current.extend(additional.clone());
        } else {
            self.details = details;
        }
        self
    }
}

impl From<McpToolFailure> for CallToolResult {
    fn from(failure: McpToolFailure) -> Self {
        let mut payload = json!({
            "ok": false,
            "error": failure.message,
            "error_code": failure.code,
            "retryable": failure.retryable,
            "details": failure.details,
        });
        if let Some(remediation) = failure.remediation {
            payload["remediation"] = Value::String(remediation);
        }
        CallToolResult::structured_error(payload)
    }
}

enum McpFailureSource {
    Message(String),
    Repository(ConnectionRepositoryError),
}

impl From<String> for McpFailureSource {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for McpFailureSource {
    fn from(message: &str) -> Self {
        Self::Message(message.to_string())
    }
}

impl From<CatalogError> for McpFailureSource {
    fn from(error: CatalogError) -> Self {
        match error {
            CatalogError::Repository(error) => Self::Repository(error),
            CatalogError::Message(message) => Self::Message(message),
        }
    }
}

impl From<sql::SqlBuildError> for McpFailureSource {
    fn from(error: sql::SqlBuildError) -> Self {
        Self::Message(error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalKind {
    Destructive,
    Update,
}

impl ApprovalKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Destructive => "destructive",
            Self::Update => "update",
        }
    }

    fn blocked_message(self, error: &ElicitationError) -> String {
        match self {
            Self::Destructive => {
                format!("高危操作已阻止：MCP 客户端未提供可用的用户确认（{error}）")
            }
            Self::Update => {
                format!("更新操作已阻止：MCP 客户端未提供可用的用户确认（{error}）")
            }
        }
    }

    const fn missing_content_message(self) -> &'static str {
        match self {
            Self::Destructive => "高危操作已取消：未收到确认内容",
            Self::Update => "更新操作已取消：未收到确认内容",
        }
    }

    const fn declined_message(self) -> &'static str {
        match self {
            Self::Destructive => "用户拒绝确认，高危操作未执行",
            Self::Update => "用户拒绝确认，更新操作未执行",
        }
    }

    const fn cancelled_message(self) -> &'static str {
        match self {
            Self::Destructive => "用户取消确认，高危操作未执行",
            Self::Update => "用户取消确认，更新操作未执行",
        }
    }
}

fn approval_details(kind: ApprovalKind) -> Value {
    json!({ "approval_kind": kind.as_str() })
}

fn map_elicitation_failure(kind: ApprovalKind, error: ElicitationError) -> McpToolFailure {
    let (code, retryable) = match &error {
        ElicitationError::CapabilityNotSupported => (ERROR_CODE_APPROVAL_UNSUPPORTED, false),
        ElicitationError::UserDeclined => (ERROR_CODE_APPROVAL_DECLINED, false),
        ElicitationError::UserCancelled => (ERROR_CODE_APPROVAL_CANCELLED, false),
        ElicitationError::Service(ServiceError::Timeout { .. }) => {
            (ERROR_CODE_APPROVAL_TIMEOUT, true)
        }
        ElicitationError::ParseError { .. } | ElicitationError::NoContent => {
            (ERROR_CODE_APPROVAL_INVALID_RESPONSE, false)
        }
        ElicitationError::Service(_) => (ERROR_CODE_APPROVAL_UNAVAILABLE, true),
        _ => (ERROR_CODE_APPROVAL_UNAVAILABLE, true),
    };
    let message = match &error {
        ElicitationError::CapabilityNotSupported => format!(
            "{}已拒绝：客户端初始化时未声明 MCP form elicitation 能力，Astesia 无法向用户请求必要确认，因此没有执行该操作。请改用支持 elicitation 的 MCP 客户端。",
            match kind {
                ApprovalKind::Destructive => "高危操作",
                ApprovalKind::Update => "更新操作",
            }
        ),
        ElicitationError::UserDeclined => kind.declined_message().to_string(),
        ElicitationError::UserCancelled => kind.cancelled_message().to_string(),
        _ => kind.blocked_message(&error),
    };
    McpToolFailure::new(code, message, retryable, approval_details(kind))
}

fn missing_approval_content(kind: ApprovalKind) -> McpToolFailure {
    McpToolFailure::new(
        ERROR_CODE_APPROVAL_INVALID_RESPONSE,
        kind.missing_content_message(),
        false,
        approval_details(kind),
    )
}

fn declined_approval(kind: ApprovalKind) -> McpToolFailure {
    McpToolFailure::new(
        ERROR_CODE_APPROVAL_DECLINED,
        kind.declined_message(),
        false,
        approval_details(kind),
    )
}

async fn handle_control_command(
    catalog: &Catalog,
    active_tests: &ActiveConnectionTests,
    command: &McpControlCommand,
) -> Result<bool, CatalogError> {
    // This lock linearizes App controls with HTTP test Acquire + registration.
    // Once the control owns it, an acquired test is either registered here or
    // has already dropped its future and lease.
    let _lifecycle = catalog
        .lock_connection_lifecycle(&command.connection_id)
        .await;
    let active_test = active_tests.current(&command.connection_id);
    if let Some(active_test) =
        active_test.filter(|active_test| active_test.generation == command.generation)
    {
        active_test.cancel();
        active_test.wait_until_future_dropped().await;
        if active_test.owns_sync_ownership {
            // The test owns this generation and has no persistent driver. Its
            // Released request may still be in flight; ACK is safe now because
            // the test future and cross-process shared lease are already gone.
            return Ok(true);
        }
    }
    catalog
        .disconnect_if_generation_under_lifecycle(&command.connection_id, command.generation)
        .await
}

impl AstesiaMcp {
    fn with_repository(repository: SharedConnectionRepository) -> Self {
        Self {
            catalog: Catalog::with_repository(repository),
            active_tests: ActiveConnectionTests::default(),
            sync: None,
            _control_loop: None,
        }
    }

    fn with_repository_and_sync(
        repository: SharedConnectionRepository,
        sync: McpSyncClient,
    ) -> Self {
        let catalog = Catalog::with_repository(repository);
        let active_tests = ActiveConnectionTests::default();
        let loop_catalog = catalog.clone();
        let loop_sync = sync.clone();
        let loop_active_tests = active_tests.clone();
        let handle = tokio::spawn(async move {
            loop {
                match loop_sync.poll_control().await {
                    Ok(Some(command)) => {
                        let result =
                            handle_control_command(&loop_catalog, &loop_active_tests, &command)
                                .await;
                        let (ok, error) = match result {
                            Ok(_) => (true, None),
                            Err(error) => (false, Some(error.to_string())),
                        };
                        if let Err(report_error) =
                            loop_sync.control_result(&command, ok, error).await
                        {
                            log::warn!(
                                "Unable to report an App-requested MCP disconnect: {report_error}"
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        log::warn!("Unable to poll Astesia MCP control commands: {error}");
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                }
            }
        });
        Self {
            catalog,
            active_tests,
            sync: Some(sync),
            _control_loop: Some(Arc::new(ControlLoopTask { handle })),
        }
    }

    fn success(value: Value) -> CallToolResult {
        CallToolResult::structured(json!({ "ok": true, "result": value }))
    }

    fn failure(source: impl Into<McpFailureSource>) -> CallToolResult {
        match source.into() {
            McpFailureSource::Message(message) => {
                McpToolFailure::new(ERROR_CODE_TOOL_FAILED, message, false, json!({})).into()
            }
            McpFailureSource::Repository(error) => McpToolFailure::from_repository(error).into(),
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
            DbType::ClickHouse => "clickhouse",
        }
    }

    fn dialect(db_type: &DbType) -> Box<dyn Dialect> {
        match db_type {
            DbType::MySQL => Box::new(MySqlDialect {}),
            DbType::PostgreSQL => Box::new(PostgreSqlDialect {}),
            DbType::SQLite => Box::new(SQLiteDialect {}),
            DbType::SQLServer => Box::new(MsSqlDialect {}),
            DbType::ClickHouse => Box::new(GenericDialect),
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
            DbType::MySQL
                | DbType::PostgreSQL
                | DbType::SQLite
                | DbType::SQLServer
                | DbType::ClickHouse
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
            DbType::ClickHouse if database.contains('`') => {
                Err("ClickHouse database 不能包含反引号".to_string())
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
            DbType::MySQL
                | DbType::PostgreSQL
                | DbType::SQLite
                | DbType::SQLServer
                | DbType::ClickHouse
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
            DbType::ClickHouse if table.contains('`') => {
                Err("ClickHouse table 不能包含反引号".to_string())
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
    ) -> Result<(), McpToolFailure> {
        let kind = ApprovalKind::Destructive;
        let response = peer
            .elicit_with_timeout::<DestructiveApproval>(message, Some(CONFIRMATION_TIMEOUT))
            .await
            .map_err(|error| map_elicitation_failure(kind, error))?
            .ok_or_else(|| missing_approval_content(kind))?;
        if response.confirm {
            Ok(())
        } else {
            Err(declined_approval(kind))
        }
    }

    async fn require_update_approval(
        &self,
        peer: &Peer<RoleServer>,
        connection_id: &str,
        database: &str,
        message: String,
    ) -> Result<(), McpToolFailure> {
        let profile = self
            .catalog
            .profile(connection_id)
            .await
            .map_err(|error| match error {
                CatalogError::Repository(error) => McpToolFailure::from_repository(error),
                CatalogError::Message(message) => McpToolFailure::new(
                    ERROR_CODE_CONNECTION_NOT_FOUND,
                    message,
                    false,
                    json!({ "connection_id": connection_id }),
                ),
            })?;
        if self.catalog.updates_are_approved(&profile, database).await {
            return Ok(());
        }
        let kind = ApprovalKind::Update;
        let response = peer
            .elicit_with_timeout::<UpdateApproval>(message, Some(CONFIRMATION_TIMEOUT))
            .await
            .map_err(|error| map_elicitation_failure(kind, error))?
            .ok_or_else(|| missing_approval_content(kind))?;
        if !response.confirm {
            return Err(declined_approval(kind));
        }
        if response.do_not_ask_again {
            self.catalog.approve_updates(&profile, database).await;
        }
        Ok(())
    }

    async fn approve_query_risk(
        &self,
        peer: &Peer<RoleServer>,
        query: &SavedQuery,
        analysis: &SqlAnalysis,
    ) -> Result<(), McpToolFailure> {
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
        .map_err(|failure| {
            failure.with_details(json!({
                "query_id": query.id,
                "connection_id": query.connection_id,
                "database": query.database,
                "risk": analysis.risk.as_str(),
                "classification_reason": match analysis.risk {
                    QueryRisk::Unknown => "Astesia 无法证明该 SQL 为只读；未列入安全白名单的函数调用或尚未建模的语法会按 fail-closed 规则要求确认。",
                    QueryRisk::Update => "SQL 会更新现有数据。",
                    QueryRisk::Delete => "SQL 会删除数据。",
                    QueryRisk::Permissions => "SQL 会修改用户、角色或权限。",
                    QueryRisk::Destructive => "SQL 包含不可逆或影响范围较大的结构变更。",
                    QueryRisk::ReadOnly | QueryRisk::Additive => "该风险等级不需要确认。",
                },
            }))
        })
    }
}

#[tool_router]
impl AstesiaMcp {
    #[tool(
        description = "List desktop connection profiles that are enabled for MCP, without returning credentials.",
        annotations(
            title = "List Astesia connections",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_connections(&self) -> CallToolResult {
        let profiles = match self.catalog.profiles().await {
            Ok(profiles) => profiles,
            Err(error) => return Self::failure(error),
        };
        let mut output = Vec::with_capacity(profiles.len());
        for profile in profiles {
            let generation = self.catalog.connected_generation(&profile.config.id).await;
            output.push(json!({
                "connection_id": profile.config.id,
                "name": profile.config.name,
                "db_type": Self::db_type_name(&profile.config.db_type),
                "host": profile.config.host,
                "port": profile.config.port,
                "username": profile.config.username,
                "database": profile.config.database,
                "credential_source": profile.has_credential.then_some("system_vault"),
                "revision": profile.revision,
                "persistent": true,
                "connected": generation.is_some(),
                "generation": generation,
            }));
        }
        Self::success(json!(output))
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
        let Some(sync) = self.sync.as_ref() else {
            let _lifecycle = self
                .catalog
                .lock_connection_lifecycle(&args.connection_id)
                .await;
            return match tokio::time::timeout(
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
            };
        };

        let lifecycle = self
            .catalog
            .lock_connection_lifecycle(&args.connection_id)
            .await;
        if self.active_tests.current(&args.connection_id).is_some() {
            return Self::failure(format!(
                "连接 {} 已有并发测试，请等待其完成后重试",
                args.connection_id
            ));
        }
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        let (generation, owns_sync_ownership) =
            match self.catalog.connected_generation(&args.connection_id).await {
                Some(generation) => (generation, false),
                None => match sync
                    .acquire(args.connection_id.clone(), profile.revision)
                    .await
                {
                    Ok(generation) => (generation, true),
                    Err(error) => {
                        return Self::failure(format!(
                            "无法向 Astesia App 申请测试连接占用: {error}"
                        ))
                    }
                },
            };
        let pending_release =
            owns_sync_ownership.then(|| (sync.clone(), args.connection_id.clone()));
        let mut active_test = match self.active_tests.register(
            &args.connection_id,
            generation,
            owns_sync_ownership,
            pending_release,
        ) {
            Ok(active_test) => active_test,
            Err(error) => return Self::failure(error),
        };
        let prepared_test = match self
            .catalog
            .prepare_connection_test(&args.connection_id, profile.revision)
            .await
        {
            Ok(prepared_test) => prepared_test,
            Err(error) => {
                // prepare_connection_test has already dropped any partial OS
                // lease. Keep the same RAII/release path as a completed test.
                active_test.mark_future_dropped();
                // Let an already queued App control ACK this generation while
                // the best-effort Released request is in flight.
                drop(lifecycle);
                if owns_sync_ownership {
                    match sync.released(args.connection_id.clone(), generation).await {
                        Ok(()) => active_test.disarm_pending_release(),
                        Err(release_error) => {
                            return Self::failure(format!(
                            "{error}; 同时无法释放 Astesia App 中的测试连接占用: {release_error}"
                        ))
                        }
                    }
                }
                return Self::failure(error);
            }
        };
        drop(lifecycle);

        let test_result = {
            let operation = tokio::time::timeout(DATABASE_OPERATION_TIMEOUT, prepared_test.run());
            tokio::pin!(operation);
            tokio::select! {
                result = &mut operation => Some(result),
                _ = active_test.cancellation.cancelled() => None,
            }
        };
        // The timeout/test future (and therefore its shared OS lease) is
        // dropped before controls are allowed to ACK this generation.
        active_test.mark_future_dropped();

        if owns_sync_ownership {
            match sync.released(args.connection_id.clone(), generation).await {
                Ok(()) => active_test.disarm_pending_release(),
                Err(error) => {
                    return Self::failure(format!(
                        "测试连接已结束，但无法释放 Astesia App 中的连接占用: {error}"
                    ))
                }
            }
        }

        match test_result {
            None => Self::failure("连接测试已由 Astesia App 取消"),
            Some(Ok(Ok(()))) => {
                Self::success(json!({ "connection_id": args.connection_id, "reachable": true }))
            }
            Some(Ok(Err(error))) => Self::failure(error),
            Some(Err(_)) => Self::failure("测试连接超时（60 秒）"),
        }
    }

    #[tool(
        description = "Open a desktop connection by ID for subsequent metadata, query, and row tools.",
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
        let _lifecycle = self
            .catalog
            .lock_connection_lifecycle(&args.connection_id)
            .await;
        if self.active_tests.current(&args.connection_id).is_some() {
            return Self::failure("连接测试正在进行或仍在释放占用，请稍后再连接");
        }
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };

        let acquired_generation = match self.sync.as_ref() {
            Some(sync) => match sync
                .acquire(args.connection_id.clone(), profile.revision)
                .await
            {
                Ok(generation) => Some(generation),
                Err(error) => {
                    return Self::failure(format!("无法向 Astesia App 申请连接占用: {error}"))
                }
            },
            None => None,
        };

        let connect_result = if let Some(generation) = acquired_generation {
            tokio::time::timeout(
                DATABASE_OPERATION_TIMEOUT,
                self.catalog.connect_with_generation(
                    &args.connection_id,
                    generation,
                    profile.revision,
                ),
            )
            .await
        } else {
            tokio::time::timeout(
                DATABASE_OPERATION_TIMEOUT,
                self.catalog.connect(&args.connection_id),
            )
            .await
        };

        let outcome = match connect_result {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(error)) => {
                if let (Some(sync), Some(generation)) = (self.sync.as_ref(), acquired_generation) {
                    if let Err(release_error) =
                        sync.released(args.connection_id.clone(), generation).await
                    {
                        return Self::failure(format!(
                            "{error}; 同时无法释放 Astesia App 中的连接占用: {release_error}"
                        ));
                    }
                }
                return Self::failure(error);
            }
            Err(_) => {
                if let (Some(sync), Some(generation)) = (self.sync.as_ref(), acquired_generation) {
                    if let Err(release_error) =
                        sync.released(args.connection_id.clone(), generation).await
                    {
                        return Self::failure(format!(
                            "访问连接超时（60 秒）；同时无法释放 Astesia App 中的连接占用: {release_error}"
                        ));
                    }
                }
                return Self::failure("访问连接超时（60 秒）");
            }
        };

        if let Some(sync) = self.sync.as_ref() {
            if let Err(error) = sync
                .connected(args.connection_id.clone(), outcome.generation)
                .await
            {
                let local_rollback = self.catalog.disconnect(&args.connection_id).await.result;
                let app_rollback = sync
                    .released(args.connection_id.clone(), outcome.generation)
                    .await;
                let rollback_status = match (local_rollback, app_rollback) {
                    (Ok(_), Ok(_)) => "连接访问已完整回滚".to_string(),
                    (local, app) => format!(
                        "连接访问回滚不完整（MCP: {}; App: {}）",
                        local
                            .err()
                            .map(|error| error.to_string())
                            .unwrap_or_else(|| "已断开".to_string()),
                        app.err()
                            .map(|error| error.to_string())
                            .unwrap_or_else(|| "已释放".to_string()),
                    ),
                };
                return Self::failure(format!(
                    "无法同步连接状态到 Astesia App: {error}; {rollback_status}"
                ));
            }
        }

        Self::success(json!({
            "connection_id": args.connection_id,
            "connected": true,
            "opened_now": outcome.opened_now,
            "generation": outcome.generation,
            "app_synced": self.sync.is_some(),
        }))
    }

    #[tool(
        description = "Close the active MCP database connection while retaining its desktop profile.",
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
        let _lifecycle = self
            .catalog
            .lock_connection_lifecycle(&args.connection_id)
            .await;
        let active_test = self.active_tests.current(&args.connection_id);
        if let Some(active_test) = active_test.as_ref() {
            active_test.cancel();
            active_test.wait_until_future_dropped().await;
        }
        let disconnect_outcome = self.catalog.disconnect(&args.connection_id).await;
        let generation = active_test
            .as_ref()
            .map(|active_test| active_test.generation)
            .or(disconnect_outcome.generation);
        let disconnected = disconnect_outcome
            .result
            .map(|closed| closed || active_test.is_some());
        let sync_error = match (self.sync.as_ref(), generation) {
            (Some(sync), Some(generation)) => sync
                .released(args.connection_id.clone(), generation)
                .await
                .err()
                .map(|error| error.to_string()),
            _ => None,
        };

        match (disconnected, sync_error) {
            (Ok(closed), None) => Self::success(json!({
                "connection_id": args.connection_id,
                "connected": false,
                "closed_now": closed,
                "generation": generation,
                "app_synced": self.sync.is_some(),
            })),
            (Ok(_), Some(error)) => {
                Self::failure(format!("连接已断开，但无法同步到 Astesia App: {error}"))
            }
            (Err(error), Some(sync_error)) => {
                Self::failure(format!("{error}; 同时无法同步到 Astesia App: {sync_error}"))
            }
            (Err(error), None) => Self::failure(error),
        }
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

#[tool_handler(
    name = "astesia-mcp",
    instructions = "Use explicit connection and saved-query identifiers. Never pass credentials directly. Destructive operations require server-side user confirmation."
)]
impl ServerHandler for AstesiaMcp {}

pub async fn run_stdio() -> anyhow::Result<()> {
    let repository = SharedConnectionRepository::new_default_strict()?;
    let service = AstesiaMcp::with_repository(repository)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

pub async fn verify_shared_credentials() -> anyhow::Result<()> {
    let report = match SharedConnectionRepository::new_default_strict() {
        Ok(repository) => match repository.verify_enabled_credentials().await {
            Ok(verified) => CredentialVerificationReport::success(verified),
            Err(error) => CredentialVerificationReport::failure(error),
        },
        Err(error) => CredentialVerificationReport::failure(error),
    };
    println!(
        "{CREDENTIAL_VERIFY_MARKER}{}",
        serde_json::to_string(&report)?
    );
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
    let repository = SharedConnectionRepository::new_default_strict()?;
    let sync_config = McpSyncConfig::from_env()?;
    let auth = HttpAuth::new(auth_token)?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let address = listener.local_addr()?;
    let endpoint = format!("http://{address}/mcp");

    let service: StreamableHttpService<AstesiaMcp, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok(AstesiaMcp::with_repository_and_sync(
                    repository.clone(),
                    sync_config.new_session(),
                ))
            },
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
    use crate::connection_repository::ConnectionRepositoryErrorCode;

    fn structured_error_payload(result: CallToolResult) -> Value {
        assert_eq!(result.is_error, Some(true));
        result
            .structured_content
            .expect("structured MCP error payload")
    }

    #[test]
    fn failure_payload_keeps_the_legacy_error_string_and_adds_metadata() {
        let payload = structured_error_payload(AstesiaMcp::failure("legacy failure message"));

        assert_eq!(payload["ok"], false);
        assert_eq!(payload["error"], "legacy failure message");
        assert_eq!(payload["error_code"], ERROR_CODE_TOOL_FAILED);
        assert_eq!(payload["retryable"], false);
        assert_eq!(payload["details"], json!({}));
        assert!(payload.get("remediation").is_none());
    }

    #[test]
    fn repository_failure_preserves_migration_reason_and_remediation() {
        let payload = structured_error_payload(AstesiaMcp::failure(CatalogError::Repository(
            ConnectionRepositoryError {
                code: ConnectionRepositoryErrorCode::CredentialMigrationRequired,
                message: "旧版凭据尚未完成安全迁移".to_string(),
                remediation: "请先打开 Astesia App 完成强制凭据迁移。".to_string(),
                retryable: false,
                details: json!({ "connection_id": "legacy-connection" }),
            },
        )));

        assert_eq!(payload["error"], "旧版凭据尚未完成安全迁移");
        assert_eq!(
            payload["error_code"],
            ConnectionRepositoryErrorCode::CredentialMigrationRequired.as_str()
        );
        assert_eq!(
            payload["remediation"],
            "请先打开 Astesia App 完成强制凭据迁移。"
        );
        assert_eq!(payload["retryable"], false);
        assert_eq!(
            payload["details"],
            json!({ "connection_id": "legacy-connection" })
        );
    }

    #[test]
    fn maps_elicitation_errors_to_stable_codes() {
        let parse_error = serde_json::from_value::<bool>(json!({ "confirm": true }))
            .expect_err("object must not deserialize as a boolean");
        let cases = vec![
            (
                ElicitationError::CapabilityNotSupported,
                ERROR_CODE_APPROVAL_UNSUPPORTED,
                false,
            ),
            (
                ElicitationError::UserDeclined,
                ERROR_CODE_APPROVAL_DECLINED,
                false,
            ),
            (
                ElicitationError::UserCancelled,
                ERROR_CODE_APPROVAL_CANCELLED,
                false,
            ),
            (
                ElicitationError::Service(ServiceError::Timeout {
                    timeout: Duration::from_secs(1),
                }),
                ERROR_CODE_APPROVAL_TIMEOUT,
                true,
            ),
            (
                ElicitationError::ParseError {
                    error: parse_error,
                    data: json!({ "confirm": true }),
                },
                ERROR_CODE_APPROVAL_INVALID_RESPONSE,
                false,
            ),
            (
                ElicitationError::NoContent,
                ERROR_CODE_APPROVAL_INVALID_RESPONSE,
                false,
            ),
            (
                ElicitationError::Service(ServiceError::UnexpectedResponse),
                ERROR_CODE_APPROVAL_UNAVAILABLE,
                true,
            ),
        ];

        for (error, expected_code, expected_retryable) in cases {
            let failure = map_elicitation_failure(ApprovalKind::Update, error);
            assert_eq!(failure.code, expected_code);
            assert_eq!(failure.retryable, expected_retryable);
            assert_eq!(failure.details, json!({ "approval_kind": "update" }));
            assert!(failure.message.contains("更新操作"));
            if expected_code == ERROR_CODE_APPROVAL_UNSUPPORTED {
                assert!(failure.message.contains("未声明 MCP form elicitation 能力"));
                assert!(failure.message.contains("没有执行该操作"));
            } else if expected_code == ERROR_CODE_APPROVAL_DECLINED {
                assert_eq!(failure.message, "用户拒绝确认，更新操作未执行");
            } else if expected_code == ERROR_CODE_APPROVAL_CANCELLED {
                assert_eq!(failure.message, "用户取消确认，更新操作未执行");
            }
        }
    }

    #[test]
    fn unsupported_elicitation_explains_refusal_for_every_approval_kind() {
        for kind in [ApprovalKind::Destructive, ApprovalKind::Update] {
            let failure = map_elicitation_failure(kind, ElicitationError::CapabilityNotSupported);

            assert_eq!(failure.code, ERROR_CODE_APPROVAL_UNSUPPORTED);
            assert!(!failure.retryable);
            assert_eq!(failure.details, json!({ "approval_kind": kind.as_str() }));
            assert!(failure.message.contains("未声明 MCP form elicitation 能力"));
            assert!(failure.message.contains("没有执行该操作"));

            let payload = structured_error_payload(failure.into());
            assert_eq!(payload["ok"], false);
            assert_eq!(payload["error_code"], ERROR_CODE_APPROVAL_UNSUPPORTED);
            assert_eq!(payload["retryable"], false);
            assert_eq!(
                payload["details"],
                json!({ "approval_kind": kind.as_str() })
            );
        }
    }

    #[test]
    fn declined_and_cancelled_elicitation_explain_that_nothing_was_executed() {
        for kind in [ApprovalKind::Destructive, ApprovalKind::Update] {
            let declined = map_elicitation_failure(kind, ElicitationError::UserDeclined);
            assert_eq!(declined.code, ERROR_CODE_APPROVAL_DECLINED);
            assert!(declined.message.contains("用户拒绝确认"));
            assert!(declined.message.contains("未执行"));

            let cancelled = map_elicitation_failure(kind, ElicitationError::UserCancelled);
            assert_eq!(cancelled.code, ERROR_CODE_APPROVAL_CANCELLED);
            assert!(cancelled.message.contains("用户取消确认"));
            assert!(cancelled.message.contains("未执行"));
        }
    }

    #[test]
    fn exposes_the_complete_tool_set() {
        let tools = AstesiaMcp::tool_router().list_all();
        let names = tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(tools.len(), 18);
        for expected in [
            "list_connections",
            "test_connection",
            "connect_connection",
            "disconnect_connection",
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
        assert!(!names.contains(&"create_connection"));
        assert!(!names.contains(&"delete_connection"));
    }

    #[test]
    fn marks_high_risk_tools_as_destructive() {
        let tools = AstesiaMcp::tool_router().list_all();

        for name in [
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

    #[tokio::test]
    async fn active_test_guard_drop_unblocks_controls_and_clears_the_weak_marker() {
        let active_tests = ActiveConnectionTests::default();
        let active_test = active_tests
            .register("connection-a", 17, false, None)
            .expect("register test");
        let state = active_tests
            .current("connection-a")
            .expect("active test state");
        let waiting_state = state.clone();
        let waiter = tokio::spawn(async move {
            waiting_state.cancel();
            waiting_state.wait_until_future_dropped().await;
            waiting_state.generation
        });

        tokio::time::timeout(Duration::from_secs(1), state.cancellation.cancelled())
            .await
            .expect("cancellation observed");
        tokio::task::yield_now().await;
        assert!(
            !waiter.is_finished(),
            "control must wait until the database test future is dropped"
        );

        drop(state);
        drop(active_test);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .expect("control waiter completed")
                .expect("control waiter task"),
            17
        );
        assert!(
            active_tests.current("connection-a").is_none(),
            "a cancelled handler future must not leave a stale active-test marker"
        );
    }

    #[test]
    fn active_test_marker_survives_until_the_handler_finishes_sync_release() {
        let active_tests = ActiveConnectionTests::default();
        let active_test = active_tests
            .register("connection-a", 23, true, None)
            .expect("register test");

        active_test.mark_future_dropped();
        assert!(
            active_tests.current("connection-a").is_some(),
            "owned generation remains visible while Released is in flight"
        );
        drop(active_test);
        assert!(active_tests.current("connection-a").is_none());

        active_tests
            .register("connection-a", 24, false, None)
            .expect("a completed handler must allow another test");
    }

    #[test]
    fn delayed_release_marker_prevents_generation_aba() {
        let active_tests = ActiveConnectionTests::default();
        let active_test = active_tests
            .register("connection-a", 31, true, None)
            .expect("register test");
        let marker = ActiveTestMarker::new(active_test.test.clone());

        active_test.mark_future_dropped();
        drop(active_test);
        assert!(
            active_tests.current("connection-a").is_some(),
            "the delayed Released task must keep connect/test blocked"
        );
        assert!(
            active_tests
                .register("connection-a", 32, false, None)
                .is_err(),
            "a new generation must not start before old Released completes"
        );

        drop(marker);
        assert!(active_tests.current("connection-a").is_none());
        active_tests
            .register("connection-a", 32, false, None)
            .expect("new generation after Released");
    }
}
