use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::connection_repository::{ConnectionRepositoryError, CredentialVerificationReport};
use crate::mcp::CREDENTIAL_VERIFY_MARKER;
use crate::platform::{SidecarControlHandle, SidecarEvent, SidecarHostHandle, SidecarRequest};

use super::reaper::{lock_unpoisoned, ProcessReaper};

const VERIFY_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_OUTPUT: usize = 16_384;

#[derive(Default)]
struct VerifierLifecycle {
    shutdown_requested: bool,
    active: Option<SidecarControlHandle>,
}

struct TerminationGuard {
    control: Option<SidecarControlHandle>,
    reaper: ProcessReaper,
    lifecycle: Arc<Mutex<VerifierLifecycle>>,
}

impl TerminationGuard {
    fn new(
        control: SidecarControlHandle,
        reaper: ProcessReaper,
        lifecycle: Arc<Mutex<VerifierLifecycle>>,
    ) -> Self {
        Self {
            control: Some(control),
            reaper,
            lifecycle,
        }
    }

    fn terminate(&mut self) -> Option<String> {
        let control = self.control.take()?;
        let error = control.terminate().err();
        if error.is_some() {
            self.reaper.retain(Arc::clone(&control));
        }
        self.clear_active(&control);
        error
    }

    fn disarm(&mut self) {
        if let Some(control) = self.control.take() {
            self.clear_active(&control);
        }
    }

    fn clear_active(&self, control: &SidecarControlHandle) {
        let mut lifecycle = lock_unpoisoned(&self.lifecycle);
        if lifecycle
            .active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, control))
        {
            lifecycle.active = None;
        }
    }
}

impl Drop for TerminationGuard {
    fn drop(&mut self) {
        if let Some(error) = self.terminate() {
            log::warn!("Unable to terminate a cancelled MCP credential verifier: {error}");
        }
    }
}

#[derive(Clone)]
pub(super) struct CredentialVerifier {
    host: SidecarHostHandle,
    lifecycle: Arc<Mutex<VerifierLifecycle>>,
    reaper: ProcessReaper,
}

impl CredentialVerifier {
    pub(super) fn new(host: SidecarHostHandle) -> Self {
        Self {
            host,
            lifecycle: Arc::new(Mutex::new(VerifierLifecycle::default())),
            reaper: ProcessReaper::default(),
        }
    }

    pub(super) async fn verify(
        &self,
    ) -> Result<CredentialVerificationReport, ConnectionRepositoryError> {
        let (mut events, mut guard) = self.spawn()?;
        let deadline = tokio::time::Instant::now() + VERIFY_TIMEOUT;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = loop {
            let event = match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(event) => event,
                Err(_) => {
                    let termination_error = guard
                        .terminate()
                        .map(|error| format!("；终止验证进程也失败：{error}"))
                        .unwrap_or_default();
                    return Err(ConnectionRepositoryError::credential_verifier_unavailable(
                        format!(
                            "astesia-mcp 在 120 秒内未完成系统凭据库验证；操作系统授权提示可能未被处理{termination_error}"
                        ),
                    ));
                }
            };
            match event {
                Some(SidecarEvent::Stdout(bytes)) => append_bounded(&mut stdout, &bytes),
                Some(SidecarEvent::Stderr(bytes)) => append_bounded(&mut stderr, &bytes),
                Some(SidecarEvent::Error(error)) => {
                    append_bounded(&mut stderr, error.as_bytes());
                    append_bounded(&mut stderr, b"\n");
                }
                Some(SidecarEvent::Terminated { code, .. }) => {
                    guard.disarm();
                    break code;
                }
                None => {
                    let termination_error = guard
                        .terminate()
                        .map(|error| format!("；终止进程也失败：{error}"))
                        .unwrap_or_default();
                    return Err(ConnectionRepositoryError::credential_verifier_unavailable(
                        format!("astesia-mcp 凭据验证输出通道意外关闭{termination_error}"),
                    ));
                }
            }
        };

        if exit_code != Some(0) {
            let stderr_detail = trimmed_output(&stderr);
            return Err(ConnectionRepositoryError::credential_verifier_unavailable(
                if stderr_detail.is_empty() {
                    format!("astesia-mcp 凭据验证进程异常退出：{exit_code:?}")
                } else {
                    format!("astesia-mcp 凭据验证进程异常退出：{stderr_detail}")
                },
            ));
        }

        let stdout = String::from_utf8_lossy(&stdout);
        let report = stdout
            .lines()
            .find_map(|line| line.strip_prefix(CREDENTIAL_VERIFY_MARKER))
            .and_then(|json| serde_json::from_str::<CredentialVerificationReport>(json).ok());
        report.ok_or_else(|| {
            let stderr_detail = trimmed_output(&stderr);
            let detail = if stderr_detail.is_empty() {
                format!("进程退出码为 {exit_code:?}")
            } else {
                stderr_detail
            };
            ConnectionRepositoryError::credential_verifier_unavailable(format!(
                "astesia-mcp 未返回有效的凭据验证结果：{detail}"
            ))
        })
    }

    fn spawn(
        &self,
    ) -> Result<
        (tokio::sync::mpsc::Receiver<SidecarEvent>, TerminationGuard),
        ConnectionRepositoryError,
    > {
        let mut lifecycle = lock_unpoisoned(&self.lifecycle);
        if lifecycle.shutdown_requested {
            return Err(ConnectionRepositoryError::credential_verifier_unavailable(
                "Astesia is shutting down and cannot start credential verification",
            ));
        }
        if lifecycle.active.is_some() {
            return Err(ConnectionRepositoryError::credential_verifier_unavailable(
                "An MCP credential verification is already running",
            ));
        }
        let pending_errors = self.reaper.retry();
        if !pending_errors.is_empty() {
            return Err(ConnectionRepositoryError::credential_verifier_unavailable(
                format!(
                    "无法清理上一次 astesia-mcp 凭据验证进程：{}",
                    pending_errors.join("; ")
                ),
            ));
        }
        let process = self
            .host
            .spawn(SidecarRequest::VerifySharedCredentials)
            .map_err(|error| {
                ConnectionRepositoryError::credential_verifier_unavailable(format!(
                    "无法启动打包的 astesia-mcp 凭据验证程序：{error}"
                ))
            })?;
        lifecycle.active = Some(Arc::clone(&process.control));
        let guard = TerminationGuard::new(
            process.control,
            self.reaper.clone(),
            Arc::clone(&self.lifecycle),
        );
        Ok((process.events, guard))
    }

    pub(super) fn request_shutdown(&self) {
        let active = {
            let mut lifecycle = lock_unpoisoned(&self.lifecycle);
            lifecycle.shutdown_requested = true;
            lifecycle.active.clone()
        };
        if let Some(control) = active {
            if let Err(error) = control.terminate() {
                log::warn!("Unable to terminate the active MCP credential verifier: {error}");
                self.reaper.retain(control);
            }
        }
    }

    pub(super) fn retry_pending_terminations(&self) -> Vec<String> {
        self.reaper.retry()
    }

    #[cfg(test)]
    pub(super) fn pending_terminations(&self) -> usize {
        self.reaper.pending_len()
    }

    #[cfg(test)]
    pub(super) fn has_active_process(&self) -> bool {
        lock_unpoisoned(&self.lifecycle).active.is_some()
    }
}

fn append_bounded(target: &mut Vec<u8>, bytes: &[u8]) {
    let remaining = MAX_OUTPUT.saturating_sub(target.len());
    target.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
}

fn trimmed_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}
