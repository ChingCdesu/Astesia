use std::time::{Duration, Instant};

use anyhow::Context;
use async_trait::async_trait;
use reqwest::{header::HeaderMap, Client, Url};
use serde_json::Value;

use super::{
    ColumnInfo, ConnectionConfig, DatabaseDriver, DbType, FunctionInfo, IndexInfo, QueryResult,
    TableInfo, UserInfo, ViewInfo,
};

const CLICKHOUSE_DEFAULT_USER: &str = "default";
const CLICKHOUSE_RESULT_FORMAT: &str = "JSONCompact";

pub struct ClickHouseDriver {
    config: ConnectionConfig,
    client: Option<Client>,
}

impl ClickHouseDriver {
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            client: None,
        }
    }

    fn build_client() -> anyhow::Result<Client> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(5)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("无法创建 ClickHouse HTTP 客户端")
    }

    fn client(&self) -> anyhow::Result<&Client> {
        self.client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))
    }

    fn endpoint_url(&self, database: &str, parameters: &[(&str, &str)]) -> anyhow::Result<Url> {
        let host = self.config.host.trim().trim_end_matches('/');
        if host.is_empty() {
            return Err(anyhow::anyhow!("ClickHouse 主机不能为空"));
        }

        let endpoint = if host.starts_with("http://") || host.starts_with("https://") {
            host.to_string()
        } else {
            format!("http://{host}")
        };
        let mut url = Url::parse(&endpoint)
            .with_context(|| format!("无效的 ClickHouse HTTP 地址: {endpoint}"))?;

        if !url.username().is_empty() || url.password().is_some() {
            return Err(anyhow::anyhow!(
                "请使用用户名和密码字段配置 ClickHouse 凭据，不要将凭据写入主机地址"
            ));
        }
        if self.config.port > 0 {
            url.set_port(Some(self.config.port))
                .map_err(|_| anyhow::anyhow!("无效的 ClickHouse 端口: {}", self.config.port))?;
        }
        url.set_fragment(None);
        if url.path().is_empty() {
            url.set_path("/");
        }

        let effective_database = if database.trim().is_empty() {
            self.config
                .database
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        } else {
            Some(database.trim())
        };
        let mut query = url.query_pairs_mut();
        query.append_pair("default_format", CLICKHOUSE_RESULT_FORMAT);
        query.append_pair("wait_end_of_query", "1");
        if let Some(database) = effective_database {
            query.append_pair("database", database);
        }
        for (name, value) in parameters {
            query.append_pair(&format!("param_{name}"), value);
        }
        drop(query);

        Ok(url)
    }

    async fn send_query_with_client(
        &self,
        client: &Client,
        database: &str,
        sql: &str,
        parameters: &[(&str, &str)],
    ) -> anyhow::Result<QueryResult> {
        let url = self.endpoint_url(database, parameters)?;
        let username = if self.config.username.trim().is_empty() {
            CLICKHOUSE_DEFAULT_USER
        } else {
            self.config.username.trim()
        };
        let start = Instant::now();
        let response = client
            .post(url)
            .basic_auth(username, Some(self.config.password.as_str()))
            .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(sql.to_string())
            .send()
            .await
            .context("无法连接 ClickHouse HTTP 服务")?;

        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .text()
            .await
            .context("无法读取 ClickHouse HTTP 响应")?;
        let elapsed = start.elapsed().as_millis() as u64;

        if !status.is_success() {
            let message = body.trim();
            return Err(anyhow::anyhow!(
                "ClickHouse HTTP {}: {}",
                status.as_u16(),
                if message.is_empty() {
                    status.canonical_reason().unwrap_or("请求失败").to_string()
                } else {
                    message.to_string()
                }
            ));
        }

        if headers.contains_key("x-clickhouse-exception-code") {
            return Err(anyhow::anyhow!("ClickHouse 查询失败: {}", body.trim()));
        }

        let affected_rows = affected_rows_from_headers(&headers);
        Ok(parse_query_result(&body, affected_rows, elapsed))
    }

    async fn send_query(
        &self,
        database: &str,
        sql: &str,
        parameters: &[(&str, &str)],
    ) -> anyhow::Result<QueryResult> {
        self.send_query_with_client(self.client()?, database, sql, parameters)
            .await
    }
}

#[async_trait]
impl DatabaseDriver for ClickHouseDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let client = Self::build_client()?;
        self.send_query_with_client(&client, "", "SELECT 1", &[])
            .await?;
        self.client = Some(client);
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        self.client.take();
        Ok(())
    }

    async fn test_connection(&self) -> anyhow::Result<bool> {
        let client = Self::build_client()?;
        self.send_query_with_client(&client, "", "SELECT 1", &[])
            .await?;
        Ok(true)
    }

    async fn get_databases(&self) -> anyhow::Result<Vec<String>> {
        let result = self
            .send_query("", "SELECT name FROM system.databases ORDER BY name", &[])
            .await?;
        Ok(result
            .rows
            .iter()
            .filter_map(|row| row.first().and_then(value_to_string))
            .collect())
    }

    async fn get_tables(&self, database: &str) -> anyhow::Result<Vec<TableInfo>> {
        let result = self
            .send_query(
                database,
                "SELECT name, total_rows, comment \
                 FROM system.tables \
                 WHERE database = {database:String} \
                   AND is_temporary = 0 \
                   AND engine NOT IN ('View', 'MaterializedView', 'LiveView', 'WindowView') \
                 ORDER BY name",
                &[("database", database)],
            )
            .await?;

        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                let name = row.first().and_then(value_to_string)?;
                let row_count = row
                    .get(1)
                    .and_then(value_to_u64)
                    .and_then(|value| i64::try_from(value).ok());
                let comment = row
                    .get(2)
                    .and_then(value_to_string)
                    .filter(|value| !value.is_empty());
                Some(TableInfo {
                    name,
                    schema: None,
                    row_count,
                    comment,
                })
            })
            .collect())
    }

    async fn get_columns(&self, database: &str, table: &str) -> anyhow::Result<Vec<ColumnInfo>> {
        let result = self
            .send_query(
                database,
                "SELECT name, type, default_expression, comment, is_in_primary_key \
                 FROM system.columns \
                 WHERE database = {database:String} AND table = {table:String} \
                 ORDER BY position",
                &[("database", database), ("table", table)],
            )
            .await?;

        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                let name = row.first().and_then(value_to_string)?;
                let data_type = row.get(1).and_then(value_to_string)?;
                let default_value = row
                    .get(2)
                    .and_then(value_to_string)
                    .filter(|value| !value.is_empty());
                let comment = row
                    .get(3)
                    .and_then(value_to_string)
                    .filter(|value| !value.is_empty());
                let is_primary_key = row.get(4).is_some_and(value_to_bool);
                Some(ColumnInfo {
                    name,
                    nullable: is_nullable_type(&data_type),
                    data_type,
                    is_primary_key,
                    default_value,
                    comment,
                })
            })
            .collect())
    }

    async fn get_indexes(&self, database: &str, table: &str) -> anyhow::Result<Vec<IndexInfo>> {
        let result = self
            .send_query(
                database,
                "SELECT name \
                 FROM system.columns \
                 WHERE database = {database:String} \
                   AND table = {table:String} \
                   AND is_in_primary_key = 1 \
                 ORDER BY position",
                &[("database", database), ("table", table)],
            )
            .await?;
        let columns: Vec<String> = result
            .rows
            .iter()
            .filter_map(|row| row.first().and_then(value_to_string))
            .collect();
        if columns.is_empty() {
            Ok(vec![])
        } else {
            Ok(vec![IndexInfo {
                name: "PRIMARY".to_string(),
                columns,
                is_unique: false,
                is_primary: true,
            }])
        }
    }

    async fn execute_query(&self, database: &str, sql: &str) -> anyhow::Result<QueryResult> {
        self.send_query(database, sql, &[]).await
    }

    async fn get_table_data(
        &self,
        database: &str,
        table: &str,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<QueryResult> {
        let offset = u64::from(page.saturating_sub(1)) * u64::from(page_size);
        let sql = format!(
            "SELECT * FROM {}.{} LIMIT {} OFFSET {}",
            quote_identifier(database),
            quote_identifier(table),
            page_size,
            offset
        );
        self.execute_query(database, &sql).await
    }

    fn db_type(&self) -> DbType {
        DbType::ClickHouse
    }

    async fn get_views(&self, database: &str) -> anyhow::Result<Vec<ViewInfo>> {
        let result = self
            .send_query(
                database,
                "SELECT name, create_table_query \
                 FROM system.tables \
                 WHERE database = {database:String} \
                   AND engine IN ('View', 'MaterializedView', 'LiveView', 'WindowView') \
                 ORDER BY name",
                &[("database", database)],
            )
            .await?;
        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                Some(ViewInfo {
                    name: row.first().and_then(value_to_string)?,
                    definition: row
                        .get(1)
                        .and_then(value_to_string)
                        .filter(|value| !value.is_empty()),
                })
            })
            .collect())
    }

    async fn get_functions(&self, database: &str) -> anyhow::Result<Vec<FunctionInfo>> {
        let result = self
            .send_query(
                database,
                "SELECT name, create_query \
                 FROM system.functions \
                 WHERE create_query != '' \
                 ORDER BY name",
                &[],
            )
            .await?;
        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                Some(FunctionInfo {
                    name: row.first().and_then(value_to_string)?,
                    language: Some("SQL".to_string()),
                    return_type: None,
                    definition: row
                        .get(1)
                        .and_then(value_to_string)
                        .filter(|value| !value.is_empty()),
                })
            })
            .collect())
    }

    async fn get_users(&self) -> anyhow::Result<Vec<UserInfo>> {
        let result = self
            .send_query("", "SELECT name FROM system.users ORDER BY name", &[])
            .await?;
        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                Some(UserInfo {
                    name: row.first().and_then(value_to_string)?,
                    host: None,
                })
            })
            .collect())
    }

    async fn get_enum_values(
        &self,
        _database: &str,
        enum_type: &str,
    ) -> anyhow::Result<Vec<String>> {
        Ok(parse_enum_values(enum_type))
    }

    async fn get_create_table_sql(&self, database: &str, table: &str) -> anyhow::Result<String> {
        let sql = format!(
            "SHOW CREATE TABLE {}.{}",
            quote_identifier(database),
            quote_identifier(table)
        );
        let result = self.send_query(database, &sql, &[]).await?;
        result
            .rows
            .first()
            .and_then(|row| row.first())
            .and_then(value_to_string)
            .ok_or_else(|| anyhow::anyhow!("Table not found"))
    }
}

fn quote_identifier(identifier: &str) -> String {
    let escaped = identifier.replace('\\', "\\\\").replace('`', "\\`");
    format!("`{escaped}`")
}

fn is_nullable_type(data_type: &str) -> bool {
    let mut current = data_type.trim();
    loop {
        if current.starts_with("Nullable(") && current.ends_with(')') {
            return true;
        }
        if let Some(inner) = current
            .strip_prefix("LowCardinality(")
            .and_then(|value| value.strip_suffix(')'))
        {
            current = inner.trim();
            continue;
        }
        return false;
    }
}

fn parse_query_result(body: &str, affected_rows: u64, execution_time_ms: u64) -> QueryResult {
    if body.trim().is_empty() {
        return QueryResult {
            affected_rows,
            execution_time_ms,
            ..Default::default()
        };
    }

    if let Ok(root) = serde_json::from_str::<Value>(body) {
        if let (Some(meta), Some(data)) = (
            root.get("meta").and_then(Value::as_array),
            root.get("data").and_then(Value::as_array),
        ) {
            let columns: Vec<ColumnInfo> = meta
                .iter()
                .filter_map(|entry| {
                    let name = entry.get("name")?.as_str()?.to_string();
                    let data_type = entry
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown")
                        .to_string();
                    Some(ColumnInfo {
                        name,
                        nullable: is_nullable_type(&data_type),
                        data_type,
                        is_primary_key: false,
                        default_value: None,
                        comment: None,
                    })
                })
                .collect();
            let rows = data
                .iter()
                .map(|row| {
                    let values = match row {
                        Value::Array(values) => values.clone(),
                        Value::Object(values) => columns
                            .iter()
                            .map(|column| values.get(&column.name).cloned().unwrap_or(Value::Null))
                            .collect(),
                        value => vec![value.clone()],
                    };
                    normalize_row(values, columns.len())
                })
                .collect();
            return QueryResult {
                columns,
                rows,
                affected_rows,
                execution_time_ms,
            };
        }
    }

    QueryResult {
        columns: vec![ColumnInfo {
            name: "result".to_string(),
            data_type: "String".to_string(),
            nullable: false,
            is_primary_key: false,
            default_value: None,
            comment: None,
        }],
        rows: body
            .lines()
            .map(|line| vec![Value::String(line.to_string())])
            .collect(),
        affected_rows,
        execution_time_ms,
    }
}

fn normalize_row(mut values: Vec<Value>, column_count: usize) -> Vec<Value> {
    values.truncate(column_count);
    values.resize(column_count, Value::Null);
    values
}

fn affected_rows_from_headers(headers: &HeaderMap) -> u64 {
    headers
        .get("x-clickhouse-summary")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .as_ref()
        .and_then(|summary| summary.get("written_rows"))
        .and_then(value_to_u64)
        .unwrap_or(0)
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        value => Some(value.to_string()),
    }
}

fn value_to_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn value_to_bool(value: &Value) -> bool {
    value
        .as_bool()
        .or_else(|| value.as_u64().map(|value| value != 0))
        .or_else(|| {
            value
                .as_str()
                .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        })
        .unwrap_or(false)
}

fn parse_enum_values(data_type: &str) -> Vec<String> {
    let trimmed = data_type.trim();
    if !(trimmed.starts_with("Enum8(") || trimmed.starts_with("Enum16(")) {
        return vec![];
    }

    let mut values = Vec::new();
    let mut chars = trimmed.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\'' {
            continue;
        }
        let mut value = String::new();
        while let Some(character) = chars.next() {
            match character {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        value.push(escaped);
                    }
                }
                '\'' if chars.peek() == Some(&'\'') => {
                    chars.next();
                    value.push('\'');
                }
                '\'' => break,
                character => value.push(character),
            }
        }
        values.push(value);
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(host: &str, port: u16) -> ConnectionConfig {
        ConnectionConfig {
            id: "clickhouse-test".to_string(),
            name: "ClickHouse test".to_string(),
            db_type: DbType::ClickHouse,
            host: host.to_string(),
            port,
            username: "default".to_string(),
            password: String::new(),
            database: Some("analytics".to_string()),
            color: None,
        }
    }

    #[test]
    fn builds_http_and_https_endpoints() {
        let driver = ClickHouseDriver::new(config("localhost", 8123));
        let url = driver
            .endpoint_url("events", &[("table", "page views")])
            .unwrap();
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("localhost"));
        assert_eq!(url.port(), Some(8123));
        assert!(url.as_str().contains("database=events"));
        assert!(url.as_str().contains("param_table=page+views"));

        let secure_driver = ClickHouseDriver::new(config("https://example.clickhouse.cloud", 8443));
        let secure_url = secure_driver.endpoint_url("", &[]).unwrap();
        assert_eq!(secure_url.scheme(), "https");
        assert_eq!(secure_url.port(), Some(8443));
        assert!(secure_url.as_str().contains("database=analytics"));
    }

    #[test]
    fn parses_json_compact_metadata_and_rows() {
        let body = r#"{
            "meta": [
                {"name": "id", "type": "UInt64"},
                {"name": "label", "type": "LowCardinality(Nullable(String))"}
            ],
            "data": [["18446744073709551615", null]],
            "rows": 1
        }"#;
        let result = parse_query_result(body, 0, 7);
        assert_eq!(result.columns.len(), 2);
        assert_eq!(result.columns[0].data_type, "UInt64");
        assert!(result.columns[1].nullable);
        assert_eq!(
            result.rows[0][0],
            Value::String("18446744073709551615".to_string())
        );
        assert_eq!(result.execution_time_ms, 7);
    }

    #[test]
    fn preserves_exception_like_strings_in_query_results() {
        let body = r#"{
            "meta": [{"name": "message", "type": "String"}],
            "data": [["__exception__"]]
        }"#;
        let result = parse_query_result(body, 0, 1);
        assert_eq!(
            result.rows,
            vec![vec![Value::String("__exception__".to_string())]]
        );
    }

    #[test]
    fn parses_clickhouse_enum_values() {
        assert_eq!(
            parse_enum_values(r#"Enum8('new' = 1, 'it\'s done' = 2, 'a''b' = 3)"#),
            vec!["new", "it's done", "a'b"]
        );
        assert!(parse_enum_values("String").is_empty());
    }

    #[test]
    fn quotes_identifiers_and_detects_nullable_wrappers() {
        assert_eq!(quote_identifier(r"odd\`name"), r"`odd\\\`name`");
        assert!(is_nullable_type("Nullable(UInt64)"));
        assert!(is_nullable_type("LowCardinality(Nullable(String))"));
        assert!(!is_nullable_type("Array(Nullable(String))"));
    }
}
