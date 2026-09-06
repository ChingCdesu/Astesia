use std::path::PathBuf;

use super::config::TRANSPORT;
use super::state::{McpServiceStatus, RuntimeState};

pub(super) fn snapshot(binary_path: Option<&PathBuf>, state: &RuntimeState) -> McpServiceStatus {
    let available = binary_path.is_some_and(|path| path.is_file());
    McpServiceStatus {
        state: state.phase(),
        available,
        pid: state.pid(),
        endpoint: state.endpoint().map(str::to_string),
        transport: TRANSPORT,
        binary_path: binary_path.map(|path| path.to_string_lossy().into_owned()),
        version: available.then(|| env!("CARGO_PKG_VERSION").to_string()),
        started_at: state.started_at().map(str::to_string),
        last_error: state.error().map(str::to_string),
    }
}
