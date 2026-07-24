use std::{
    env, fmt,
    net::IpAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::DbType;

pub const SYNC_ENDPOINT_ENV: &str = "ASTESIA_MCP_SYNC_ENDPOINT";
pub const SYNC_TOKEN_ENV: &str = "ASTESIA_MCP_SYNC_TOKEN";
pub const SYNC_SERVICE_ID_ENV: &str = "ASTESIA_MCP_SERVICE_ID";
pub const MCP_AUTH_TOKEN_ENV: &str = "ASTESIA_MCP_AUTH_TOKEN";
pub const SYNC_PASSWORD_ENV_PREFIX: &str = "ASTESIA_DB_PASSWORD_";
pub const SYNC_PATH: &str = "/v1/sync";
pub const PROTOCOL_VERSION: u16 = 1;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(65);
const MIN_TOKEN_BYTES: usize = 32;
const MAX_TOKEN_BYTES: usize = 256;
const MAX_IDENTIFIER_BYTES: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpSyncProfile {
    pub connection_id: String,
    pub name: String,
    pub db_type: DbType,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub database: Option<String>,
    pub color: Option<String>,
    pub password_env: Option<String>,
}

impl McpSyncProfile {
    pub fn validate(&self) -> Result<(), McpSyncError> {
        validate_identifier("connection_id", &self.connection_id)?;
        validate_non_empty("name", &self.name)?;
        validate_non_empty("host", &self.host)?;
        if let Some(password_env) = self.password_env.as_deref() {
            validate_password_env(password_env)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpSyncContext {
    pub protocol_version: u16,
    pub service_id: Uuid,
    pub session_id: Uuid,
    pub operation_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum McpSyncRequest {
    Upsert {
        context: McpSyncContext,
        profile: McpSyncProfile,
    },
    Connected {
        context: McpSyncContext,
        connection_id: String,
    },
    Disconnected {
        context: McpSyncContext,
        connection_id: String,
    },
    Deleted {
        context: McpSyncContext,
        connection_id: String,
    },
    SessionClosed {
        context: McpSyncContext,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpSyncResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_connection_id: Option<String>,
}

#[derive(Clone)]
pub struct McpSyncConfig {
    endpoint: Url,
    token: Arc<str>,
    service_id: Uuid,
    http: Client,
}

impl fmt::Debug for McpSyncConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpSyncConfig")
            .field("endpoint", &self.endpoint)
            .field("token", &"[redacted]")
            .field("service_id", &self.service_id)
            .finish()
    }
}

impl McpSyncConfig {
    /// Load the private App synchronization channel.
    ///
    /// Only the Streamable HTTP entry point should call this method. The stdio
    /// entry point deliberately constructs no sync config, even if these
    /// environment variables happen to be present.
    pub fn from_env() -> Result<Self, McpSyncError> {
        let endpoint = required_env(SYNC_ENDPOINT_ENV)?;
        let token = required_env(SYNC_TOKEN_ENV)?;
        let service_id = required_env(SYNC_SERVICE_ID_ENV)?;
        Self::from_values(&endpoint, &token, &service_id)
    }

    fn from_values(endpoint: &str, token: &str, service_id: &str) -> Result<Self, McpSyncError> {
        let endpoint = validate_endpoint(endpoint)?;
        validate_token(token)?;
        let service_id = Uuid::parse_str(service_id).map_err(|_| {
            McpSyncError::InvalidConfig(format!("{SYNC_SERVICE_ID_ENV} must be a UUID"))
        })?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(McpSyncError::Http)?;
        Ok(Self {
            endpoint,
            token: Arc::from(token),
            service_id,
            http,
        })
    }

    pub fn new_session(&self) -> McpSyncClient {
        McpSyncClient {
            inner: Arc::new(SessionInner {
                config: self.clone(),
                session_id: Uuid::new_v4(),
                closed: AtomicBool::new(false),
            }),
        }
    }

    async fn post(&self, request: &McpSyncRequest) -> Result<McpSyncResponse, McpSyncError> {
        let response = self
            .http
            .post(self.endpoint.clone())
            .bearer_auth(self.token.as_ref())
            .json(request)
            .send()
            .await
            .map_err(McpSyncError::Http)?;
        let status = response.status();
        if !status.is_success() {
            return Err(McpSyncError::HttpStatus(status));
        }
        let response = response
            .json::<McpSyncResponse>()
            .await
            .map_err(McpSyncError::Http)?;
        if response.ok {
            Ok(response)
        } else {
            Err(McpSyncError::Remote(response.error.unwrap_or_else(|| {
                "Astesia App rejected the synchronization request".into()
            })))
        }
    }
}

#[derive(Clone)]
pub struct McpSyncClient {
    inner: Arc<SessionInner>,
}

impl fmt::Debug for McpSyncClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpSyncClient")
            .field("service_id", &self.inner.config.service_id)
            .field("session_id", &self.inner.session_id)
            .finish()
    }
}

impl McpSyncClient {
    pub async fn upsert(&self, profile: McpSyncProfile) -> Result<McpSyncResponse, McpSyncError> {
        profile.validate()?;
        self.send(McpSyncRequest::Upsert {
            context: self.context(),
            profile,
        })
        .await
    }

    pub async fn connected(
        &self,
        connection_id: impl Into<String>,
    ) -> Result<McpSyncResponse, McpSyncError> {
        let connection_id = connection_id.into();
        validate_identifier("connection_id", &connection_id)?;
        self.send(McpSyncRequest::Connected {
            context: self.context(),
            connection_id,
        })
        .await
    }

    pub async fn disconnected(
        &self,
        connection_id: impl Into<String>,
    ) -> Result<McpSyncResponse, McpSyncError> {
        let connection_id = connection_id.into();
        validate_identifier("connection_id", &connection_id)?;
        self.send(McpSyncRequest::Disconnected {
            context: self.context(),
            connection_id,
        })
        .await
    }

    pub async fn deleted(
        &self,
        connection_id: impl Into<String>,
    ) -> Result<McpSyncResponse, McpSyncError> {
        let connection_id = connection_id.into();
        validate_identifier("connection_id", &connection_id)?;
        self.send(McpSyncRequest::Deleted {
            context: self.context(),
            connection_id,
        })
        .await
    }

    fn context(&self) -> McpSyncContext {
        McpSyncContext {
            protocol_version: PROTOCOL_VERSION,
            service_id: self.inner.config.service_id,
            session_id: self.inner.session_id,
            operation_id: Uuid::new_v4(),
        }
    }

    async fn send(&self, request: McpSyncRequest) -> Result<McpSyncResponse, McpSyncError> {
        self.inner.config.post(&request).await
    }
}

struct SessionInner {
    config: McpSyncConfig,
    session_id: Uuid,
    closed: AtomicBool,
}

impl Drop for SessionInner {
    fn drop(&mut self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let config = self.config.clone();
        let request = McpSyncRequest::SessionClosed {
            context: McpSyncContext {
                protocol_version: PROTOCOL_VERSION,
                service_id: config.service_id,
                session_id: self.session_id,
                operation_id: Uuid::new_v4(),
            },
        };
        runtime.spawn(async move {
            if let Err(error) = config.post(&request).await {
                log::debug!("Unable to notify Astesia App that an MCP session closed: {error}");
            }
        });
    }
}

#[derive(Debug)]
pub enum McpSyncError {
    MissingEnvironment(&'static str),
    InvalidConfig(String),
    Http(reqwest::Error),
    HttpStatus(StatusCode),
    Remote(String),
}

impl fmt::Display for McpSyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEnvironment(name) => {
                write!(formatter, "required environment variable {name} is not set")
            }
            Self::InvalidConfig(message) => formatter.write_str(message),
            Self::Http(error) => write!(formatter, "MCP synchronization request failed: {error}"),
            Self::HttpStatus(status) => {
                write!(
                    formatter,
                    "Astesia App synchronization returned HTTP {status}"
                )
            }
            Self::Remote(message) => {
                write!(formatter, "Astesia App synchronization failed: {message}")
            }
        }
    }
}

impl std::error::Error for McpSyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(error) => Some(error),
            _ => None,
        }
    }
}

fn required_env(name: &'static str) -> Result<String, McpSyncError> {
    env::var(name).map_err(|_| McpSyncError::MissingEnvironment(name))
}

fn validate_endpoint(raw: &str) -> Result<Url, McpSyncError> {
    let endpoint = Url::parse(raw).map_err(|_| {
        McpSyncError::InvalidConfig(format!("{SYNC_ENDPOINT_ENV} must be a valid URL"))
    })?;
    if endpoint.scheme() != "http" {
        return Err(McpSyncError::InvalidConfig(format!(
            "{SYNC_ENDPOINT_ENV} must use http"
        )));
    }
    if !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(McpSyncError::InvalidConfig(format!(
            "{SYNC_ENDPOINT_ENV} must not contain credentials, a query, or a fragment"
        )));
    }
    if endpoint.port().is_none() {
        return Err(McpSyncError::InvalidConfig(format!(
            "{SYNC_ENDPOINT_ENV} must include an explicit loopback port"
        )));
    }
    if endpoint.path() != SYNC_PATH {
        return Err(McpSyncError::InvalidConfig(format!(
            "{SYNC_ENDPOINT_ENV} must use the exact path {SYNC_PATH}"
        )));
    }
    let host = endpoint.host_str().ok_or_else(|| {
        McpSyncError::InvalidConfig(format!("{SYNC_ENDPOINT_ENV} must include a loopback host"))
    })?;
    let host_without_ipv6_brackets = host.trim_start_matches('[').trim_end_matches(']');
    let is_loopback = host.eq_ignore_ascii_case("localhost")
        || host_without_ipv6_brackets
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if !is_loopback {
        return Err(McpSyncError::InvalidConfig(format!(
            "{SYNC_ENDPOINT_ENV} must target a loopback address"
        )));
    }
    Ok(endpoint)
}

fn validate_token(token: &str) -> Result<(), McpSyncError> {
    let valid_length = (MIN_TOKEN_BYTES..=MAX_TOKEN_BYTES).contains(&token.len());
    let valid_characters = token
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'));
    if valid_length && valid_characters {
        Ok(())
    } else {
        Err(McpSyncError::InvalidConfig(format!(
            "{SYNC_TOKEN_ENV} must contain 32-256 URL-safe ASCII characters"
        )))
    }
}

fn validate_non_empty(field: &str, value: &str) -> Result<(), McpSyncError> {
    if value.trim().is_empty() {
        Err(McpSyncError::InvalidConfig(format!(
            "MCP sync profile {field} must not be empty"
        )))
    } else {
        Ok(())
    }
}

fn validate_identifier(field: &str, value: &str) -> Result<(), McpSyncError> {
    validate_non_empty(field, value)?;
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(McpSyncError::InvalidConfig(format!(
            "MCP sync profile {field} must not exceed {MAX_IDENTIFIER_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_password_env(name: &str) -> Result<(), McpSyncError> {
    let mut characters = name.chars();
    let valid = characters
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
    if !valid {
        return Err(McpSyncError::InvalidConfig(
            "password_env must be a valid environment variable name".into(),
        ));
    }
    if !name
        .to_ascii_uppercase()
        .starts_with(SYNC_PASSWORD_ENV_PREFIX)
    {
        return Err(McpSyncError::InvalidConfig(format!(
            "App-managed HTTP password_env must start with {SYNC_PASSWORD_ENV_PREFIX}"
        )));
    }
    if name.eq_ignore_ascii_case(MCP_AUTH_TOKEN_ENV) || name.eq_ignore_ascii_case(SYNC_TOKEN_ENV) {
        return Err(McpSyncError::InvalidConfig(format!(
            "password_env must not reference Astesia's MCP authentication variables"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn config(endpoint: &str) -> Result<McpSyncConfig, McpSyncError> {
        McpSyncConfig::from_values(
            endpoint,
            "abcdEFGH0123-._~abcdEFGH0123abcd",
            "4fa85f64-5717-4562-b3fc-2c963f66afa6",
        )
    }

    fn profile(password_env: Option<&str>) -> McpSyncProfile {
        McpSyncProfile {
            connection_id: "analytics".into(),
            name: "Analytics".into(),
            db_type: DbType::PostgreSQL,
            host: "127.0.0.1".into(),
            port: 5432,
            username: "reader".into(),
            database: Some("warehouse".into()),
            color: None,
            password_env: password_env.map(str::to_string),
        }
    }

    #[test]
    fn accepts_only_explicit_loopback_http_endpoints() {
        assert!(config("http://127.0.0.1:43678/v1/sync").is_ok());
        assert!(config("http://localhost:43678/v1/sync").is_ok());
        assert!(config("http://[::1]:43678/v1/sync").is_ok());

        assert!(config("https://127.0.0.1:43678/v1/sync").is_err());
        assert!(config("http://192.168.1.2:43678/v1/sync").is_err());
        assert!(config("http://127.0.0.1/v1/sync").is_err());
        assert!(config("http://127.0.0.1:43678/not-sync").is_err());
        assert!(config("http://user@127.0.0.1:43678/v1/sync").is_err());
        assert!(config("http://127.0.0.1:43678/v1/sync?token=x").is_err());
    }

    #[test]
    fn validates_token_and_service_id_without_exposing_the_token_in_debug() {
        let config = config("http://127.0.0.1:43678/v1/sync").expect("valid config");
        let debug = format!("{config:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("abcdEFGH0123"));

        assert!(McpSyncConfig::from_values(
            "http://127.0.0.1:43678/v1/sync",
            "short",
            "4fa85f64-5717-4562-b3fc-2c963f66afa6",
        )
        .is_err());
        assert!(McpSyncConfig::from_values(
            "http://127.0.0.1:43678/v1/sync",
            "abcdEFGH0123-._~abcdEFGH0123abcd",
            "not-a-uuid",
        )
        .is_err());
    }

    #[test]
    fn serializes_profiles_without_a_plaintext_password_field() {
        let request = McpSyncRequest::Upsert {
            context: McpSyncContext {
                protocol_version: PROTOCOL_VERSION,
                service_id: Uuid::nil(),
                session_id: Uuid::nil(),
                operation_id: Uuid::nil(),
            },
            profile: profile(Some("ASTESIA_DB_PASSWORD_ANALYTICS")),
        };
        let value = serde_json::to_value(request).expect("serialize sync request");
        assert_eq!(value["event"], "upsert");
        let object = value["profile"].as_object().expect("profile object");
        assert!(!object.contains_key("password"));
        assert_eq!(
            object.get("password_env"),
            Some(&Value::String("ASTESIA_DB_PASSWORD_ANALYTICS".into()))
        );
    }

    #[test]
    fn rejects_using_mcp_authentication_tokens_as_database_passwords() {
        assert!(profile(Some(MCP_AUTH_TOKEN_ENV)).validate().is_err());
        assert!(profile(Some("astesia_mcp_auth_token")).validate().is_err());
        assert!(profile(Some(SYNC_TOKEN_ENV)).validate().is_err());
        assert!(profile(Some("ASTESIA_DB_PASSWORD_ANALYTICS"))
            .validate()
            .is_ok());
        assert!(profile(Some("AWS_SECRET_ACCESS_KEY")).validate().is_err());
        assert!(profile(Some("GITHUB_TOKEN")).validate().is_err());
    }

    #[test]
    fn request_variants_have_stable_event_names() {
        let context = McpSyncContext {
            protocol_version: PROTOCOL_VERSION,
            service_id: Uuid::nil(),
            session_id: Uuid::nil(),
            operation_id: Uuid::nil(),
        };
        let requests = [
            (
                McpSyncRequest::Connected {
                    context: context.clone(),
                    connection_id: "one".into(),
                },
                "connected",
            ),
            (
                McpSyncRequest::Disconnected {
                    context: context.clone(),
                    connection_id: "one".into(),
                },
                "disconnected",
            ),
            (
                McpSyncRequest::Deleted {
                    context: context.clone(),
                    connection_id: "one".into(),
                },
                "deleted",
            ),
            (McpSyncRequest::SessionClosed { context }, "session_closed"),
        ];
        for (request, expected) in requests {
            assert_eq!(
                serde_json::to_value(request).expect("serialize request")["event"],
                expected
            );
        }
    }
}
