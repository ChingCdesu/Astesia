mod config;
mod monitor;
mod reaper;
mod service;
mod state;
mod status;
mod termination;
mod verifier;

#[cfg(test)]
mod tests;

use crate::connection_repository::{ConnectionRepositoryError, CredentialVerificationReport};
use crate::mcp_sync_server::McpSyncRegistry;
use crate::platform::SidecarHostHandle;

pub use state::{McpServicePhase, McpServiceStatus};

use service::ManagedMcpService;
use verifier::CredentialVerifier;

#[derive(Clone)]
pub struct McpRuntime {
    service: ManagedMcpService,
    verifier: CredentialVerifier,
}

impl McpRuntime {
    pub fn new(sidecar_host: SidecarHostHandle, sync_registry: McpSyncRegistry) -> Self {
        Self {
            service: ManagedMcpService::new(sidecar_host.clone(), sync_registry),
            verifier: CredentialVerifier::new(sidecar_host),
        }
    }

    pub async fn status(&self) -> McpServiceStatus {
        self.service.status().await
    }

    pub async fn verify_shared_credentials(
        &self,
    ) -> Result<CredentialVerificationReport, ConnectionRepositoryError> {
        self.verifier.verify().await
    }

    pub async fn start(&self, port: u16, auth_token: String) -> Result<McpServiceStatus, String> {
        self.service.start(port, auth_token).await
    }

    pub async fn stop(&self) -> Result<McpServiceStatus, String> {
        self.service.stop().await
    }

    pub async fn restart(&self, port: u16, auth_token: String) -> Result<McpServiceStatus, String> {
        self.service.restart(port, auth_token).await
    }

    pub async fn shutdown(&self) {
        self.verifier.request_shutdown();
        if let Err(error) = self.service.stop().await {
            log::warn!("Unable to stop MCP sidecar during shutdown: {error}");
        }
        for error in self.verifier.retry_pending_terminations() {
            log::warn!("Unable to terminate a pending MCP sidecar during shutdown: {error}");
        }
    }
}
