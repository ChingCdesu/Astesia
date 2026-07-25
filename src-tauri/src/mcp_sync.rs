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

pub const SYNC_ENDPOINT_ENV: &str = "ASTESIA_MCP_SYNC_ENDPOINT";
pub const SYNC_TOKEN_ENV: &str = "ASTESIA_MCP_SYNC_TOKEN";
pub const SYNC_SERVICE_ID_ENV: &str = "ASTESIA_MCP_SERVICE_ID";
pub const MCP_AUTH_TOKEN_ENV: &str = "ASTESIA_MCP_AUTH_TOKEN";
pub const SYNC_PATH: &str = "/v1/sync";
pub const PROTOCOL_VERSION: u16 = 2;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(65);
const MIN_TOKEN_BYTES: usize = 32;
const MAX_TOKEN_BYTES: usize = 256;
const MAX_IDENTIFIER_BYTES: usize = 256;
const MAX_CONTROL_ERROR_BYTES: usize = 4_096;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpSyncContext {
    pub protocol_version: u16,
    pub service_id: Uuid,
    pub session_id: Uuid,
    pub operation_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum McpSyncRequest {
    Acquire {
        context: McpSyncContext,
        connection_id: String,
        profile_revision: i64,
    },
    Connected {
        context: McpSyncContext,
        connection_id: String,
        generation: u64,
    },
    Released {
        context: McpSyncContext,
        connection_id: String,
        generation: u64,
    },
    PollControl {
        context: McpSyncContext,
    },
    ControlResult {
        context: McpSyncContext,
        command_id: Uuid,
        connection_id: String,
        generation: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    SessionClosed {
        context: McpSyncContext,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpControlCommand {
    pub command_id: Uuid,
    pub connection_id: String,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpSyncResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<McpControlCommand>,
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
    /// Only the App-managed Streamable HTTP entry point should call this
    /// method. Standalone stdio intentionally has no reverse-control channel.
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
    /// Reserve a shared profile revision before opening its database driver.
    ///
    /// The returned generation must accompany every later state transition.
    /// This makes a delayed force-disconnect command harmless after reconnect.
    pub async fn acquire(
        &self,
        connection_id: impl Into<String>,
        profile_revision: i64,
    ) -> Result<u64, McpSyncError> {
        let connection_id = connection_id.into();
        validate_identifier("connection_id", &connection_id)?;
        if profile_revision < 0 {
            return Err(McpSyncError::InvalidConfig(
                "profile_revision must not be negative".into(),
            ));
        }
        let response = self
            .send(McpSyncRequest::Acquire {
                context: self.context(),
                connection_id,
                profile_revision,
            })
            .await?;
        response
            .generation
            .filter(|generation| *generation > 0)
            .ok_or_else(|| {
                McpSyncError::InvalidResponse(
                    "Astesia App did not return an acquire generation".into(),
                )
            })
    }

    pub async fn connected(
        &self,
        connection_id: impl Into<String>,
        generation: u64,
    ) -> Result<(), McpSyncError> {
        let connection_id = connection_id.into();
        validate_identifier("connection_id", &connection_id)?;
        validate_generation(generation)?;
        self.send(McpSyncRequest::Connected {
            context: self.context(),
            connection_id,
            generation,
        })
        .await
        .map(|_| ())
    }

    pub async fn released(
        &self,
        connection_id: impl Into<String>,
        generation: u64,
    ) -> Result<(), McpSyncError> {
        let connection_id = connection_id.into();
        validate_identifier("connection_id", &connection_id)?;
        validate_generation(generation)?;
        self.send(McpSyncRequest::Released {
            context: self.context(),
            connection_id,
            generation,
        })
        .await
        .map(|_| ())
    }

    /// Long-poll for a private App-to-HTTP-session control command.
    ///
    /// Callers should run at most one poll loop for each MCP session and stop
    /// that loop when the session handler is dropped.
    pub async fn poll_control(&self) -> Result<Option<McpControlCommand>, McpSyncError> {
        self.send(McpSyncRequest::PollControl {
            context: self.context(),
        })
        .await
        .map(|response| response.control)
    }

    pub async fn control_result(
        &self,
        command: &McpControlCommand,
        ok: bool,
        error: Option<String>,
    ) -> Result<(), McpSyncError> {
        validate_identifier("connection_id", &command.connection_id)?;
        validate_generation(command.generation)?;
        if command.command_id.is_nil() {
            return Err(McpSyncError::InvalidConfig(
                "command_id must not be a nil UUID".into(),
            ));
        }
        if ok && error.is_some() {
            return Err(McpSyncError::InvalidConfig(
                "a successful control result must not include an error".into(),
            ));
        }
        if !ok && error.as_deref().is_none_or(str::is_empty) {
            return Err(McpSyncError::InvalidConfig(
                "a failed control result must include an error".into(),
            ));
        }
        if error
            .as_ref()
            .is_some_and(|message| message.len() > MAX_CONTROL_ERROR_BYTES)
        {
            return Err(McpSyncError::InvalidConfig(format!(
                "control result error must not exceed {MAX_CONTROL_ERROR_BYTES} bytes"
            )));
        }
        self.send(McpSyncRequest::ControlResult {
            context: self.context(),
            command_id: command.command_id,
            connection_id: command.connection_id.clone(),
            generation: command.generation,
            ok,
            error,
        })
        .await
        .map(|_| ())
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
    InvalidResponse(String),
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
            Self::InvalidConfig(message) | Self::InvalidResponse(message) => {
                formatter.write_str(message)
            }
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
            "MCP sync {field} must not be empty"
        )))
    } else {
        Ok(())
    }
}

fn validate_identifier(field: &str, value: &str) -> Result<(), McpSyncError> {
    validate_non_empty(field, value)?;
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(McpSyncError::InvalidConfig(format!(
            "MCP sync {field} must not exceed {MAX_IDENTIFIER_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_generation(generation: u64) -> Result<(), McpSyncError> {
    if generation == 0 {
        Err(McpSyncError::InvalidConfig(
            "MCP sync generation must be greater than zero".into(),
        ))
    } else {
        Ok(())
    }
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

    fn context() -> McpSyncContext {
        McpSyncContext {
            protocol_version: PROTOCOL_VERSION,
            service_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
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
    fn protocol_transfers_only_shared_ids_revisions_generations_and_control_state() {
        let request = McpSyncRequest::Acquire {
            context: context(),
            connection_id: "analytics".into(),
            profile_revision: 7,
        };
        let value = serde_json::to_value(request).expect("serialize sync request");
        assert_eq!(value["event"], "acquire");
        assert_eq!(value["connection_id"], "analytics");
        assert_eq!(value["profile_revision"], 7);
        let serialized = serde_json::to_string(&value).expect("serialize JSON");
        for forbidden in [
            "password",
            "password_env",
            "username",
            "host",
            "database",
            "credential",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "protocol leaked forbidden field {forbidden}"
            );
        }
    }

    #[test]
    fn request_variants_have_stable_event_names() {
        let command = McpControlCommand {
            command_id: Uuid::new_v4(),
            connection_id: "one".into(),
            generation: 1,
        };
        let requests = [
            (
                McpSyncRequest::Acquire {
                    context: context(),
                    connection_id: "one".into(),
                    profile_revision: 1,
                },
                "acquire",
            ),
            (
                McpSyncRequest::Connected {
                    context: context(),
                    connection_id: "one".into(),
                    generation: 1,
                },
                "connected",
            ),
            (
                McpSyncRequest::Released {
                    context: context(),
                    connection_id: "one".into(),
                    generation: 1,
                },
                "released",
            ),
            (
                McpSyncRequest::PollControl { context: context() },
                "poll_control",
            ),
            (
                McpSyncRequest::ControlResult {
                    context: context(),
                    command_id: command.command_id,
                    connection_id: command.connection_id,
                    generation: command.generation,
                    ok: true,
                    error: None,
                },
                "control_result",
            ),
            (
                McpSyncRequest::SessionClosed { context: context() },
                "session_closed",
            ),
        ];
        for (request, expected) in requests {
            assert_eq!(
                serde_json::to_value(request).expect("serialize request")["event"],
                Value::String(expected.into())
            );
        }
    }

    #[test]
    fn response_omits_absent_control_and_generation() {
        let value = serde_json::to_value(McpSyncResponse {
            ok: true,
            error: None,
            generation: None,
            control: None,
        })
        .expect("serialize response");
        let object = value.as_object().expect("response object");
        assert_eq!(object.get("ok"), Some(&Value::Bool(true)));
        assert!(!object.contains_key("generation"));
        assert!(!object.contains_key("control"));
    }
}
