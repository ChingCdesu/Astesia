mod approval;
mod catalog;
mod connection_tools;
mod execution;
mod failure;
mod object_tools;
mod policy;
mod protocol;
mod query_tools;
mod row_tools;
mod session;
mod sql;
mod transport;

use rmcp::{handler::server::router::tool::ToolRouter, tool_handler, ServerHandler};

pub use session::AstesiaMcp;
pub(crate) use transport::CREDENTIAL_VERIFY_MARKER;
pub use transport::{run_http, run_stdio, verify_shared_credentials};

impl AstesiaMcp {
    fn tool_router() -> ToolRouter<Self> {
        Self::connection_tools_router()
            + Self::object_tools_router()
            + Self::query_tools_router()
            + Self::row_tools_router()
    }
}

#[tool_handler(
    name = "astesia-mcp",
    instructions = "Use explicit connection and saved-query identifiers. Never pass credentials directly. Destructive operations require server-side user confirmation."
)]
impl ServerHandler for AstesiaMcp {}

#[cfg(test)]
mod tests;
