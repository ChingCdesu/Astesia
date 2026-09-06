use std::sync::Arc;

use tokio::sync::broadcast;

use crate::{mcp_sync_server::McpConnectionsSnapshot, tasks::BackgroundTask};

#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    TaskProgress {
        id: String,
        progress: f32,
        message: String,
    },
    TaskCompleted {
        task: Arc<BackgroundTask>,
    },
    McpConnectionsChanged(McpConnectionsSnapshot),
}

pub trait UiEventSink: Send + Sync {
    fn emit(&self, event: UiEvent);
}

pub type UiEventSinkHandle = Arc<dyn UiEventSink>;

#[derive(Clone)]
pub struct UiEventBus {
    sender: broadcast::Sender<UiEvent>,
}

impl UiEventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(64);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<UiEvent> {
        self.sender.subscribe()
    }
}

impl Default for UiEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl UiEventSink for UiEventBus {
    fn emit(&self, event: UiEvent) {
        let _ = self.sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribers_receive_typed_events() {
        let bus = UiEventBus::new();
        let mut events = bus.subscribe();

        let task = Arc::new(BackgroundTask {
            id: "backup-1".to_string(),
            name: "Backup".to_string(),
            status: crate::tasks::TaskStatus::Completed,
            progress: 1.0,
            message: "Done".to_string(),
            created_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
        });
        bus.emit(UiEvent::TaskCompleted { task: task.clone() });

        assert_eq!(
            events.recv().await.expect("event"),
            UiEvent::TaskCompleted { task }
        );
    }
}
