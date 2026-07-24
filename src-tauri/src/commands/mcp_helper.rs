use tauri::State;

use crate::mcp_helper::{McpHelperState, McpServiceStatus};
use crate::mcp_sync_server::McpConnectionsSnapshot;

#[tauri::command]
pub async fn mcp_service_status(
    state: State<'_, McpHelperState>,
) -> Result<McpServiceStatus, String> {
    Ok(state.status().await)
}

#[tauri::command]
pub async fn mcp_synced_connections(
    state: State<'_, McpHelperState>,
) -> Result<McpConnectionsSnapshot, String> {
    Ok(state.synced_connections().await)
}

#[tauri::command]
pub async fn start_mcp_service(
    state: State<'_, McpHelperState>,
    port: u16,
    auth_token: String,
) -> Result<McpServiceStatus, String> {
    state.start(port, auth_token).await
}

#[tauri::command]
pub async fn stop_mcp_service(
    state: State<'_, McpHelperState>,
) -> Result<McpServiceStatus, String> {
    state.stop().await
}

#[tauri::command]
pub async fn restart_mcp_service(
    state: State<'_, McpHelperState>,
    port: u16,
    auth_token: String,
) -> Result<McpServiceStatus, String> {
    state.restart(port, auth_token).await
}
