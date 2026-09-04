use crate::mcp_auth::{is_safe_token_byte, MAX_TOKEN_BYTES, MIN_TOKEN_BYTES};

pub(super) struct ServiceConfig {
    pub(super) port: u16,
    pub(super) auth_token: String,
}

pub(super) const TRANSPORT: &str = "streamable_http";

pub(super) fn validate_port(port: u16) -> Result<(), String> {
    if port < 1024 {
        return Err("MCP service port must be between 1024 and 65535".to_string());
    }
    Ok(())
}

pub(super) fn validate_auth_token(token: &str) -> Result<(), String> {
    if token.len() < MIN_TOKEN_BYTES {
        return Err("MCP authentication token must contain at least 32 characters".to_string());
    }
    if token.len() > MAX_TOKEN_BYTES {
        return Err("MCP authentication token must not exceed 256 characters".to_string());
    }
    if !token.bytes().all(is_safe_token_byte) {
        return Err(
            "MCP authentication token may only contain safe ASCII token characters".to_string(),
        );
    }
    Ok(())
}

pub(super) fn endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

#[cfg(test)]
pub(super) const TEST_TRANSPORT: &str = TRANSPORT;
