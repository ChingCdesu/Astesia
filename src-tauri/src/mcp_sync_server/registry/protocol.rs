use uuid::Uuid;

use crate::mcp_sync::{
    McpControlCommand, McpSyncContext, McpSyncRequest, McpSyncResponse, PROTOCOL_VERSION,
};

const MAX_IDENTIFIER_BYTES: usize = 256;

pub(super) fn validate_connection_id(connection_id: &str) -> Result<(), String> {
    if connection_id.trim().is_empty() {
        return Err("MCP connection identifier must not be empty".into());
    }
    if connection_id.len() > MAX_IDENTIFIER_BYTES {
        return Err(format!(
            "MCP connection identifier must not exceed {MAX_IDENTIFIER_BYTES} bytes"
        ));
    }
    Ok(())
}

pub(in crate::mcp_sync_server) fn validate_context(
    context: &McpSyncContext,
    expected_service_id: Uuid,
) -> Result<(), String> {
    if context.protocol_version != PROTOCOL_VERSION {
        return Err("Unsupported MCP synchronization protocol version".into());
    }
    if context.service_id != expected_service_id {
        return Err("MCP synchronization service identifier does not match".into());
    }
    if context.session_id.is_nil() || context.operation_id.is_nil() {
        return Err("MCP synchronization identifiers must not be nil UUIDs".into());
    }
    Ok(())
}

pub(super) fn request_context(request: &McpSyncRequest) -> &McpSyncContext {
    match request {
        McpSyncRequest::Acquire { context, .. }
        | McpSyncRequest::Connected { context, .. }
        | McpSyncRequest::Released { context, .. }
        | McpSyncRequest::PollControl { context }
        | McpSyncRequest::ControlResult { context, .. }
        | McpSyncRequest::SessionClosed { context } => context,
    }
}

pub(in crate::mcp_sync_server) fn success(
    generation: Option<u64>,
    control: Option<McpControlCommand>,
) -> McpSyncResponse {
    McpSyncResponse {
        ok: true,
        error: None,
        generation,
        control,
    }
}

pub(in crate::mcp_sync_server) fn failure(message: impl Into<String>) -> McpSyncResponse {
    McpSyncResponse {
        ok: false,
        error: Some(message.into()),
        generation: None,
        control: None,
    }
}
