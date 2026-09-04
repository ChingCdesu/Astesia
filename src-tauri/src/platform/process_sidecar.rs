use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex, Weak},
    thread,
    time::Duration,
};

use tokio::sync::mpsc;

use crate::mcp_sync::{MCP_AUTH_TOKEN_ENV, SYNC_ENDPOINT_ENV, SYNC_SERVICE_ID_ENV, SYNC_TOKEN_ENV};

use super::sidecar::{
    SidecarControl, SidecarControlHandle, SidecarEvent, SidecarHost, SidecarInstallation,
    SidecarRequest, SpawnedSidecar,
};

#[derive(Clone, Debug)]
pub(crate) struct ProcessSidecarHost {
    executable_path: Option<PathBuf>,
}

impl ProcessSidecarHost {
    pub(crate) fn discover() -> Self {
        Self {
            executable_path: discover_sidecar(),
        }
    }

    #[cfg(test)]
    fn with_executable(executable_path: PathBuf) -> Self {
        Self {
            executable_path: Some(executable_path),
        }
    }
}

impl SidecarHost for ProcessSidecarHost {
    fn installation(&self) -> SidecarInstallation {
        SidecarInstallation {
            executable_path: self.executable_path.clone(),
        }
    }

    fn spawn(&self, request: SidecarRequest) -> Result<SpawnedSidecar, String> {
        let executable = self
            .executable_path
            .as_ref()
            .filter(|path| path.is_file())
            .ok_or_else(|| "The astesia-mcp executable is not installed".to_string())?;
        let mut command = Command::new(executable);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_request(&mut command, request);

        let mut child = command
            .spawn()
            .map_err(|error| format!("Unable to launch {}: {error}", executable.display()))?;
        let pid = child.id();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "The MCP sidecar stdout pipe is unavailable".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "The MCP sidecar stderr pipe is unavailable".to_string())?;
        let stdin = child.stdin.take();
        let process = Arc::new(ProcessControl {
            state: Mutex::new(ProcessState {
                child: Some(child),
                stdin,
            }),
        });
        let control: SidecarControlHandle = process.clone();
        let (events, receiver) = mpsc::channel(64);
        forward_output(stdout, SidecarStream::Stdout, events.clone());
        forward_output(stderr, SidecarStream::Stderr, events.clone());
        monitor_process(Arc::downgrade(&process), events);
        Ok(SpawnedSidecar {
            pid,
            control,
            events: receiver,
        })
    }
}

struct ProcessState {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
}

struct ProcessControl {
    state: Mutex<ProcessState>,
}

impl SidecarControl for ProcessControl {
    fn terminate(&self) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.stdin.take();
        let Some(child) = state.child.as_mut() else {
            return Ok(());
        };
        if child
            .try_wait()
            .map_err(|error| format!("Unable to inspect MCP sidecar: {error}"))?
            .is_some()
        {
            state.child.take();
            return Ok(());
        }
        child
            .kill()
            .map_err(|error| format!("Unable to terminate MCP sidecar: {error}"))
    }
}

impl Drop for ProcessControl {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        state.stdin.take();
        if let Some(mut child) = state.child.take() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

enum SidecarStream {
    Stdout,
    Stderr,
}

fn forward_output(
    mut output: impl Read + Send + 'static,
    stream: SidecarStream,
    events: mpsc::Sender<SidecarEvent>,
) {
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match output.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    let event = match stream {
                        SidecarStream::Stdout => SidecarEvent::Stdout(buffer[..count].to_vec()),
                        SidecarStream::Stderr => SidecarEvent::Stderr(buffer[..count].to_vec()),
                    };
                    if events.blocking_send(event).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = events.blocking_send(SidecarEvent::Error(error.to_string()));
                    break;
                }
            }
        }
    });
}

fn monitor_process(process: Weak<ProcessControl>, events: mpsc::Sender<SidecarEvent>) {
    thread::spawn(move || loop {
        let Some(process) = process.upgrade() else {
            return;
        };
        let status = {
            let mut state = process
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let status = match state.child.as_mut() {
                Some(child) => child.try_wait(),
                None => return,
            };
            match status {
                Ok(Some(status)) => {
                    state.child.take();
                    state.stdin.take();
                    Some(Ok(status))
                }
                Ok(None) => None,
                Err(error) => {
                    state.child.take();
                    state.stdin.take();
                    Some(Err(error))
                }
            }
        };
        match status {
            Some(Ok(status)) => {
                let _ = events.blocking_send(SidecarEvent::Terminated {
                    code: status.code(),
                    signal: exit_signal(&status),
                });
                return;
            }
            Some(Err(error)) => {
                let _ = events.blocking_send(SidecarEvent::Error(error.to_string()));
                return;
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    });
}

fn configure_request(command: &mut Command, request: SidecarRequest) {
    match request {
        SidecarRequest::Serve {
            http_port,
            auth_token,
            sync_endpoint,
            sync_token,
            sync_service_id,
        } => {
            command
                .arg("--http-port")
                .arg(http_port.to_string())
                .env(MCP_AUTH_TOKEN_ENV, auth_token)
                .env(SYNC_ENDPOINT_ENV, sync_endpoint)
                .env(SYNC_TOKEN_ENV, sync_token)
                .env(SYNC_SERVICE_ID_ENV, sync_service_id);
        }
        SidecarRequest::VerifySharedCredentials => {
            command.arg("--verify-shared-credentials");
        }
    }
}

fn discover_sidecar() -> Option<PathBuf> {
    let executable_name = if cfg!(windows) {
        "astesia-mcp.exe"
    } else {
        "astesia-mcp"
    };
    let current_executable = std::env::current_exe().ok();
    sidecar_candidates(
        current_executable.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
        executable_name,
    )
    .into_iter()
    .find(|path| path.is_file())
}

fn sidecar_candidates(
    current_executable: Option<&Path>,
    manifest: &Path,
    executable_name: &str,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(current_executable) = current_executable {
        if let Some(directory) = current_executable.parent() {
            candidates.push(directory.join(executable_name));
        }
    }

    candidates.push(manifest.join("target/debug").join(executable_name));
    candidates.push(manifest.join("target/release").join(executable_name));
    collect_target_candidates(&manifest.join("target"), executable_name, &mut candidates);
    candidates
}

fn collect_target_candidates(
    directory: &Path,
    executable_name: &str,
    candidates: &mut Vec<PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut targets = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .flat_map(|path| {
            [
                path.join("debug").join(executable_name),
                path.join("release").join(executable_name),
            ]
        })
        .collect::<Vec<_>>();
    targets.sort();
    candidates.extend(targets);
}

#[cfg(unix)]
fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn exit_signal(_: &std::process::ExitStatus) -> Option<i32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_reports_the_exact_discovered_executable() {
        let path = PathBuf::from("/tmp/astesia-mcp-test");
        let host = ProcessSidecarHost::with_executable(path.clone());
        assert_eq!(host.installation().executable_path, Some(path));
    }

    #[test]
    fn serve_secrets_are_environment_only() {
        let mut command = Command::new("astesia-mcp");
        configure_request(
            &mut command,
            SidecarRequest::Serve {
                http_port: 42_000,
                auth_token: "auth-secret".to_string(),
                sync_endpoint: "http://127.0.0.1:1/v1/sync".to_string(),
                sync_token: "sync-secret".to_string(),
                sync_service_id: "service-id".to_string(),
            },
        );
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy())
            .collect::<Vec<_>>();
        assert_eq!(arguments, ["--http-port", "42000"]);
        assert!(!arguments.iter().any(|argument| argument.contains("secret")));
    }

    #[test]
    fn packaged_sidecar_beside_the_application_is_the_first_candidate() {
        let candidates = sidecar_candidates(
            Some(Path::new("/opt/Astesia/bin/astesia")),
            Path::new("/source/src-tauri"),
            "astesia-mcp",
        );

        assert_eq!(
            candidates.first(),
            Some(&PathBuf::from("/opt/Astesia/bin/astesia-mcp"))
        );
        assert!(candidates
            .iter()
            .all(|candidate| !candidate.to_string_lossy().contains("/binaries/")));
    }
}
