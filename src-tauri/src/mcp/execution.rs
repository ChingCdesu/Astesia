use rmcp::model::CallToolResult;
use serde_json::{json, Value};
use sqlparser::{
    ast::Statement,
    dialect::{
        Dialect, GenericDialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect,
    },
    parser::Parser,
};

use crate::db::{DbType, QueryResult, TableRef};

use super::{
    failure::{McpFailureSource, McpToolFailure, ERROR_CODE_TOOL_FAILED},
    policy::{self, QueryRisk, SqlAnalysis},
    AstesiaMcp,
};

pub(super) const DEFAULT_RESULT_ROWS: usize = 200;
pub(super) const MAX_RESULT_ROWS: usize = 500;
const MAX_CELL_CHARS: usize = 16_384;
const MAX_CONTAINER_ITEMS: usize = 500;
const MAX_MUTATION_JSON_BYTES: usize = 1_048_576;
const MAX_SQL_BYTES: usize = 1_048_576;
pub(super) const MAX_SELECTOR_BYTES: usize = 256;
pub(super) const MAX_INSERT_COLUMNS: usize = 256;
pub(super) const MAX_DELETE_ROWS: usize = 1_000;
pub(super) const DATABASE_OPERATION_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(60);

impl AstesiaMcp {
    pub(super) fn success(value: Value) -> CallToolResult {
        CallToolResult::structured(json!({ "ok": true, "result": value }))
    }

    pub(super) fn failure(source: impl Into<McpFailureSource>) -> CallToolResult {
        match source.into() {
            McpFailureSource::Message(message) => {
                McpToolFailure::new(ERROR_CODE_TOOL_FAILED, message, false, json!({})).into()
            }
            McpFailureSource::Repository(error) => McpToolFailure::from_repository(error).into(),
        }
    }

    pub(super) fn db_type_name(db_type: &DbType) -> &'static str {
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

    pub(super) fn analyze_sql(db_type: &DbType, sql: &str) -> SqlAnalysis {
        let dialect = Self::dialect(db_type);
        policy::analyze_sql_with_dialect(sql, dialect.as_ref())
    }

    pub(super) fn validate_no_credential_sql(
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

    pub(super) fn validate_statement_prefix(
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

    pub(super) fn ensure_sql_backend(db_type: &DbType) -> Result<(), String> {
        if db_type.capabilities().sql {
            Ok(())
        } else {
            Err(
                "该结构化 SQL 工具暂不支持 MongoDB/Redis；连接测试、访问和分页读取仍可用"
                    .to_string(),
            )
        }
    }

    pub(super) fn validate_database_selector(
        db_type: &DbType,
        database: &str,
    ) -> Result<(), String> {
        if !matches!(db_type, DbType::SQLite) && database.is_empty() {
            return Err("database 不能为空".to_string());
        }
        if database.len() > MAX_SELECTOR_BYTES {
            return Err(format!("database 不能超过 {MAX_SELECTOR_BYTES} 字节"));
        }
        if database.chars().any(char::is_control) {
            return Err("database 不能包含控制字符".to_string());
        }
        Ok(())
    }

    pub(super) fn validate_read_table(table: &str) -> Result<(), String> {
        if table.is_empty() || table.chars().any(char::is_control) {
            return Err("table 不能为空或包含控制字符".to_string());
        }
        if table.len() > MAX_SELECTOR_BYTES {
            return Err(format!("table 不能超过 {MAX_SELECTOR_BYTES} 字节"));
        }
        Ok(())
    }

    pub(super) fn bounded_row_limit(requested: Option<u32>) -> usize {
        requested
            .map(|value| value as usize)
            .unwrap_or(DEFAULT_RESULT_ROWS)
            .clamp(1, MAX_RESULT_ROWS)
    }

    pub(super) fn bounded_page(requested: Option<u32>, page_size: u32) -> u32 {
        let max_page = (u32::MAX / page_size).saturating_add(1);
        requested.unwrap_or(1).max(1).min(max_page)
    }

    pub(super) fn validate_mutation_payload(values: &[Value]) -> Result<(), String> {
        let size = serde_json::to_vec(values)
            .map_err(|error| format!("无法验证行数据大小: {error}"))?
            .len();
        if size > MAX_MUTATION_JSON_BYTES {
            return Err("单次行修改的 JSON 数据不能超过 1 MiB".to_string());
        }
        Ok(())
    }

    pub(super) fn validate_sql_size(sql: &str) -> Result<(), String> {
        if sql.len() > MAX_SQL_BYTES {
            return Err("单条 SQL 不能超过 1 MiB".to_string());
        }
        Ok(())
    }

    pub(super) async fn validate_primary_key_target(
        &self,
        connection_id: &str,
        database: &str,
        table: &str,
        primary_key_column: &str,
        updated_column: Option<&str>,
    ) -> Result<TableRef, String> {
        let profile = self.catalog.profile(connection_id).await?;
        Self::validate_database_selector(&profile.config.db_type, database)?;
        Self::validate_read_table(table)?;
        Self::ensure_sql_backend(&profile.config.db_type)?;

        let table = TableRef::parse(profile.config.db_type, table)?;
        let driver = self.catalog.driver(connection_id).await?;
        let columns = tokio::time::timeout(DATABASE_OPERATION_TIMEOUT, async {
            let driver = driver.lock_active().await?;
            driver
                .get_columns(database, &table)
                .await
                .map_err(|error| error.to_string())
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
        Ok(table)
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

    pub(super) fn query_result_json(mut result: QueryResult, row_limit: usize) -> Value {
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

    pub(super) async fn execute_sql(
        &self,
        connection_id: &str,
        database: &str,
        sql: &str,
    ) -> Result<QueryResult, String> {
        let profile = self.catalog.profile(connection_id).await?;
        Self::validate_database_selector(&profile.config.db_type, database)?;
        let driver = self.catalog.driver(connection_id).await?;
        let result = tokio::time::timeout(DATABASE_OPERATION_TIMEOUT, async {
            let driver = driver.lock_active().await?;
            driver
                .execute_query(database, sql)
                .await
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|_| "数据库操作超时（60 秒）".to_string())?;
        result.map_err(|error| format!("数据库操作失败: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_sql_support_follows_engine_capabilities() {
        for db_type in [
            DbType::MySQL,
            DbType::PostgreSQL,
            DbType::SQLite,
            DbType::SQLServer,
            DbType::ClickHouse,
        ] {
            assert!(AstesiaMcp::ensure_sql_backend(&db_type).is_ok());
        }
        assert!(AstesiaMcp::ensure_sql_backend(&DbType::MongoDB).is_err());
        assert!(AstesiaMcp::ensure_sql_backend(&DbType::Redis).is_err());
    }
}
