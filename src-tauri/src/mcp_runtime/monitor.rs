use std::future::Future;

use tokio::sync::oneshot;

use crate::platform::SidecarEvent;

const READY_PREFIX: &str = "ASTESIA_MCP_READY";

pub(super) fn spawn_event_monitor<F, Fut>(
    mut receiver: tokio::sync::mpsc::Receiver<SidecarEvent>,
    expected_endpoint: String,
    ready_sender: Option<oneshot::Sender<Result<(), String>>>,
    on_terminated: F,
) where
    F: FnOnce(String, bool) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let mut ready_sender = ready_sender;
        let mut last_stderr = None;
        let mut stderr = LineBuffer::default();

        while let Some(event) = receiver.recv().await {
            match event {
                SidecarEvent::Stdout(bytes) => log_output("stdout", &bytes),
                SidecarEvent::Stderr(bytes) => {
                    for line in stderr.push(&bytes) {
                        if is_ready_line(&line, &expected_endpoint) {
                            complete_readiness(&mut ready_sender, || Ok(()));
                            continue;
                        }
                        if !line.is_empty() {
                            last_stderr = Some(line.clone());
                        }
                        log::debug!("MCP sidecar stderr: {line}");
                    }
                    if stderr
                        .pending_line()
                        .is_some_and(|line| is_ready_line(line, &expected_endpoint))
                    {
                        complete_readiness(&mut ready_sender, || Ok(()));
                    }
                }
                SidecarEvent::Error(error) => {
                    complete_readiness(&mut ready_sender, || {
                        Err(format!("MCP sidecar output error: {error}"))
                    });
                    log::warn!("MCP sidecar output error: {error}");
                }
                SidecarEvent::Terminated { code, signal } => {
                    if let Some(line) = stderr.finish() {
                        if !is_ready_line(&line, &expected_endpoint) && !line.is_empty() {
                            last_stderr = Some(line);
                        }
                    }
                    let mut message = match (code, signal) {
                        (Some(code), _) => format!("MCP sidecar exited with code {code}"),
                        (None, Some(signal)) => {
                            format!("MCP sidecar terminated by signal {signal}")
                        }
                        (None, None) => "MCP sidecar terminated unexpectedly".to_string(),
                    };
                    if let Some(detail) = last_stderr.as_deref() {
                        message.push_str(": ");
                        message.push_str(detail);
                    }
                    complete_readiness(&mut ready_sender, || Err(message.clone()));
                    on_terminated(message, false).await;
                    return;
                }
            }
        }

        let message = "MCP sidecar output channel closed unexpectedly".to_string();
        complete_readiness(&mut ready_sender, || Err(message.clone()));
        on_terminated(message, true).await;
    });
}

fn complete_readiness(
    sender: &mut Option<oneshot::Sender<Result<(), String>>>,
    result: impl FnOnce() -> Result<(), String>,
) {
    if let Some(sender) = sender.take() {
        let _ = sender.send(result());
    }
}

fn log_output(stream: &str, bytes: &[u8]) {
    let line = String::from_utf8_lossy(bytes);
    log::debug!(
        "MCP sidecar {stream}: {}",
        line.trim_end_matches(['\r', '\n'])
    );
}

#[derive(Default)]
struct LineBuffer {
    pending: Vec<u8>,
}

impl LineBuffer {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(bytes);
        let Some(last_newline) = self.pending.iter().rposition(|byte| *byte == b'\n') else {
            return Vec::new();
        };
        let complete = self.pending.drain(..=last_newline).collect::<Vec<_>>();
        String::from_utf8_lossy(&complete)
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .map(str::to_string)
            .collect()
    }

    fn pending_line(&self) -> Option<&str> {
        std::str::from_utf8(&self.pending)
            .ok()
            .map(|line| line.trim_end_matches('\r'))
    }

    fn finish(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        Some(
            String::from_utf8_lossy(&std::mem::take(&mut self.pending))
                .trim_end_matches('\r')
                .to_string(),
        )
    }
}

pub(super) fn is_ready_line(line: &str, expected_endpoint: &str) -> bool {
    line.strip_prefix(READY_PREFIX)
        .and_then(|suffix| suffix.strip_prefix(' '))
        == Some(expected_endpoint)
}
