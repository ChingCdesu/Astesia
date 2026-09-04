use std::{fmt, sync::Arc, time::Duration};

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        HeaderMap, Request, Response, StatusCode,
    },
    routing::post,
    Router,
};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use uuid::Uuid;

use crate::{
    mcp_auth::constant_time_eq,
    mcp_sync::{McpSyncRequest, McpSyncResponse, SYNC_PATH},
};

use super::registry::{protocol::failure, McpSyncRegistry};

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct ServerContext {
    service_id: Uuid,
    bearer: Arc<str>,
    registry: McpSyncRegistry,
}

pub struct McpSyncServerHandle {
    endpoint: String,
    token: String,
    service_id: Uuid,
    registry: McpSyncRegistry,
    shutdown_sender: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), String>>>,
}

impl fmt::Debug for McpSyncServerHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpSyncServerHandle")
            .field("endpoint", &self.endpoint)
            .field("token", &"[redacted]")
            .field("service_id", &self.service_id)
            .finish_non_exhaustive()
    }
}

impl McpSyncServerHandle {
    pub async fn start(registry: McpSyncRegistry) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| format!("Unable to bind MCP synchronization server: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("Unable to inspect MCP synchronization server: {error}"))?;
        let (service_id, token) = generate_credentials();
        let context = ServerContext {
            service_id,
            bearer: Arc::from(format!("Bearer {token}")),
            registry: registry.clone(),
        };
        let router = Router::new()
            .route(SYNC_PATH, post(receive_sync))
            .with_state(context);
        let (shutdown_sender, shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_receiver.await;
                })
                .await
                .map_err(|error| format!("MCP synchronization server stopped: {error}"))
        });

        Ok(Self {
            endpoint: format!("http://127.0.0.1:{}{SYNC_PATH}", address.port()),
            token,
            service_id,
            registry,
            shutdown_sender: Some(shutdown_sender),
            task: Some(task),
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn service_id(&self) -> Uuid {
        self.service_id
    }

    pub async fn shutdown(mut self) -> Result<(), String> {
        if let Some(sender) = self.shutdown_sender.take() {
            let _ = sender.send(());
        }
        let server_result = match self.task.take() {
            Some(mut task) => {
                match tokio::time::timeout(SERVER_SHUTDOWN_TIMEOUT, &mut task).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(error)) => Err(format!("MCP synchronization task failed: {error}")),
                    Err(_) => {
                        task.abort();
                        let _ = task.await;
                        Err("MCP synchronization server did not stop within 2 seconds".into())
                    }
                }
            }
            None => Ok(()),
        };
        self.registry.reset_service(self.service_id).await;
        server_result
    }
}

impl Drop for McpSyncServerHandle {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown_sender.take() {
            let _ = sender.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let registry = self.registry.clone();
        let service_id = self.service_id;
        runtime.spawn(async move {
            registry.reset_service(service_id).await;
        });
    }
}

fn generate_credentials() -> (Uuid, String) {
    (
        Uuid::new_v4(),
        format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
    )
}

async fn receive_sync(
    State(context): State<ServerContext>,
    request: Request<Body>,
) -> Response<Body> {
    if !authorizes(request.headers(), context.bearer.as_bytes()) {
        return response(StatusCode::UNAUTHORIZED, failure("Unauthorized"));
    }
    let body = match to_bytes(request.into_body(), MAX_REQUEST_BYTES).await {
        Ok(body) => body,
        Err(_) => {
            return response(
                StatusCode::BAD_REQUEST,
                failure("Invalid MCP synchronization request body"),
            )
        }
    };
    let request = match serde_json::from_slice::<McpSyncRequest>(&body) {
        Ok(request) => request,
        Err(_) => {
            return response(
                StatusCode::BAD_REQUEST,
                failure("Invalid MCP synchronization request"),
            )
        }
    };
    let result = context.registry.apply(context.service_id, request).await;
    response(StatusCode::OK, result)
}

fn authorizes(headers: &HeaderMap, expected: &[u8]) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|actual| constant_time_eq(actual.as_bytes(), expected))
}

fn response(status: StatusCode, value: McpSyncResponse) -> Response<Body> {
    let body = serde_json::to_vec(&value)
        .unwrap_or_else(|_| br#"{"ok":false,"error":"Unable to serialize response"}"#.to_vec());
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("valid synchronization response")
}
