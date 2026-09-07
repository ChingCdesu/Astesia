use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{header::AUTHORIZATION, Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    Router,
};
use rmcp::{
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ServiceExt,
};
use tokio::io::AsyncReadExt;

use crate::{
    connection_repository::{CredentialVerificationReport, SharedConnectionRepository},
    mcp_auth::{constant_time_eq, has_safe_token_syntax},
    mcp_sync::McpSyncConfig,
};

use super::AstesiaMcp;

pub(crate) const CREDENTIAL_VERIFY_MARKER: &str = "ASTESIA_SHARED_CREDENTIALS_VERIFIED ";

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
pub(super) struct HttpAuth {
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

pub(super) fn validate_http_auth_token(token: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        has_safe_token_syntax(token),
        "ASTESIA_MCP_AUTH_TOKEN must contain 32-256 URL-safe ASCII characters"
    );
    Ok(())
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
