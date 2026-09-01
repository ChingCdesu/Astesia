use std::fmt;

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct McpConnectionSnapshot {
    /// Canonical identifier from the shared connection repository.
    pub id: String,
    pub profile_revision: i64,
    pub mcp_in_use: bool,
    pub mcp_connected: bool,
    pub mcp_session_count: usize,
    pub disconnecting: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct McpConnectionsSnapshot {
    pub revision: u64,
    pub connections: Vec<McpConnectionSnapshot>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ForceDisconnectResult {
    pub requested: usize,
    pub completed: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ForceDisconnectError {
    pub requested: usize,
    pub completed: usize,
    pub error: String,
}

impl fmt::Display for ForceDisconnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Unable to disconnect all Streamable HTTP MCP sessions ({}/{} completed): {}",
            self.completed, self.requested, self.error
        )
    }
}
