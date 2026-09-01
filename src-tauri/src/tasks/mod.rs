use std::{collections::HashMap, future::Future, sync::Arc};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::platform::{UiEvent, UiEventSinkHandle};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundTask {
    pub id: String,
    pub name: String,
    pub status: TaskStatus,
    pub progress: f32,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Cancelling,
    Completed,
    Partial,
    Failed,
    Cancelled,
}

pub(crate) struct NewTask {
    pub name: String,
    pub initial_message: String,
}

pub(crate) enum TaskOutcome {
    Completed(String),
    Partial(String),
    Failed(String),
    Cancelled(String),
}

#[derive(Clone)]
pub(crate) struct TaskContext {
    id: String,
    cancellation: CancellationToken,
    manager: TaskManager,
}

impl TaskContext {
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub async fn progress(&self, progress: f32, message: impl Into<String>) {
        self.manager
            .update_progress(&self.id, progress, message.into())
            .await;
    }
}

#[derive(Clone)]
pub struct TaskManager {
    inner: Arc<TaskManagerInner>,
}

struct TaskManagerInner {
    entries: Mutex<HashMap<String, TaskEntry>>,
    events: UiEventSinkHandle,
}

struct TaskEntry {
    task: BackgroundTask,
    cancellation: Option<CancellationToken>,
    completion_event_sent: bool,
}

impl TaskEntry {
    fn is_terminal(&self) -> bool {
        self.task.completed_at.is_some()
    }

    fn mark_terminal(&mut self, status: TaskStatus, message: String) {
        self.task.status = status;
        self.task.message = message;
        self.task.completed_at = Some(Utc::now());
    }

    fn complete(&mut self, id: &str, outcome: TaskOutcome) -> Option<UiEvent> {
        self.cancellation = None;
        if self.completion_event_sent {
            return None;
        }
        if !self.is_terminal() {
            let (status, message) = match outcome {
                TaskOutcome::Completed(message) => {
                    self.task.progress = 1.0;
                    (TaskStatus::Completed, message)
                }
                TaskOutcome::Partial(message) => {
                    self.task.progress = 1.0;
                    (TaskStatus::Partial, message)
                }
                TaskOutcome::Failed(message) => (TaskStatus::Failed, message),
                TaskOutcome::Cancelled(message) => (TaskStatus::Cancelled, message),
            };
            self.mark_terminal(status, message);
        }
        self.completion_event_sent = true;
        Some(UiEvent::TaskCompleted { id: id.to_string() })
    }
}

impl TaskManager {
    pub fn new(events: UiEventSinkHandle) -> Self {
        Self {
            inner: Arc::new(TaskManagerInner {
                entries: Mutex::new(HashMap::new()),
                events,
            }),
        }
    }

    pub(crate) async fn spawn<F, Fut>(&self, new_task: NewTask, run: F) -> String
    where
        F: FnOnce(TaskContext) -> Fut + Send + 'static,
        Fut: Future<Output = TaskOutcome> + Send + 'static,
    {
        let id = uuid::Uuid::new_v4().to_string();
        let cancellation = CancellationToken::new();
        let background_task = BackgroundTask {
            id: id.clone(),
            name: new_task.name,
            status: TaskStatus::Running,
            progress: 0.0,
            message: new_task.initial_message,
            created_at: Utc::now(),
            completed_at: None,
        };
        let initial_event = UiEvent::TaskProgress {
            id: id.clone(),
            progress: background_task.progress,
            message: background_task.message.clone(),
        };
        {
            let mut entries = self.inner.entries.lock().await;
            entries.insert(
                id.clone(),
                TaskEntry {
                    task: background_task,
                    cancellation: Some(cancellation.clone()),
                    completion_event_sent: false,
                },
            );
        }
        self.inner.events.emit(initial_event);

        let context = TaskContext {
            id: id.clone(),
            cancellation,
            manager: self.clone(),
        };
        let worker = tokio::spawn(async move { run(context).await });
        let manager = self.clone();
        let worker_id = id.clone();
        tokio::spawn(async move {
            let outcome = match worker.await {
                Ok(outcome) => outcome,
                Err(error) => TaskOutcome::Failed(format!("任务异常终止: {error}")),
            };
            manager.finish(&worker_id, outcome).await;
        });
        id
    }

    pub async fn list_tasks(&self) -> Vec<BackgroundTask> {
        let entries = self.inner.entries.lock().await;
        let mut tasks = entries
            .values()
            .map(|entry| entry.task.clone())
            .collect::<Vec<_>>();
        tasks.sort_by_key(|task| std::cmp::Reverse(task.created_at));
        tasks
    }

    pub async fn get_task(&self, id: &str) -> Option<BackgroundTask> {
        self.inner
            .entries
            .lock()
            .await
            .get(id)
            .map(|entry| entry.task.clone())
    }

    pub async fn cancel_task(&self, id: &str) -> Result<(), String> {
        {
            let mut entries = self.inner.entries.lock().await;
            let entry = entries
                .get_mut(id)
                .ok_or_else(|| "任务不存在或已完成".to_string())?;
            if entry.is_terminal() {
                return Err("任务不存在或已完成".to_string());
            }
            let token = entry
                .cancellation
                .as_ref()
                .ok_or_else(|| "任务不存在或已完成".to_string())?;
            token.cancel();
            entry.task.status = TaskStatus::Cancelling;
            entry.task.message = "正在取消".to_string();
        }
        Ok(())
    }

    async fn update_progress(&self, id: &str, progress: f32, message: String) {
        let event = {
            let mut entries = self.inner.entries.lock().await;
            let Some(entry) = entries.get_mut(id) else {
                return;
            };
            if entry.is_terminal() || entry.task.status == TaskStatus::Cancelling {
                return;
            }
            entry.task.progress = progress.clamp(0.0, 1.0);
            entry.task.message = message;
            UiEvent::TaskProgress {
                id: id.to_string(),
                progress: entry.task.progress,
                message: entry.task.message.clone(),
            }
        };
        self.inner.events.emit(event);
    }

    async fn finish(&self, id: &str, outcome: TaskOutcome) {
        let event = {
            let mut entries = self.inner.entries.lock().await;
            let Some(entry) = entries.get_mut(id) else {
                return;
            };
            entry.complete(id, outcome)
        };
        if let Some(event) = event {
            self.inner.events.emit(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::oneshot;

    use super::*;
    use crate::platform::{UiEvent, UiEventBus};

    #[tokio::test]
    async fn spawn_owns_progress_and_completion_state() {
        let events = UiEventBus::new();
        let mut receiver = events.subscribe();
        let manager = TaskManager::new(Arc::new(events));

        let id = manager
            .spawn(
                NewTask {
                    name: "Backup".to_string(),
                    initial_message: "Starting".to_string(),
                },
                |task| async move {
                    task.progress(0.4, "Working").await;
                    TaskOutcome::Completed("Done".to_string())
                },
            )
            .await;

        assert_eq!(
            receiver.recv().await.expect("initial event"),
            UiEvent::TaskProgress {
                id: id.clone(),
                progress: 0.0,
                message: "Starting".to_string(),
            }
        );
        assert_eq!(
            receiver.recv().await.expect("progress event"),
            UiEvent::TaskProgress {
                id: id.clone(),
                progress: 0.4,
                message: "Working".to_string(),
            }
        );
        assert_eq!(
            receiver.recv().await.expect("completion event"),
            UiEvent::TaskCompleted { id: id.clone() }
        );

        let task = manager.get_task(&id).await.expect("task");
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.progress, 1.0);
        assert_eq!(task.message, "Done");
        assert!(task.completed_at.is_some());
    }

    #[tokio::test]
    async fn partial_outcomes_are_terminal_and_complete_progress() {
        let events = UiEventBus::new();
        let mut receiver = events.subscribe();
        let manager = TaskManager::new(Arc::new(events));

        let id = manager
            .spawn(
                NewTask {
                    name: "Restore".to_string(),
                    initial_message: "Starting".to_string(),
                },
                |_| async { TaskOutcome::Partial("4 succeeded, 1 failed".to_string()) },
            )
            .await;

        assert!(matches!(
            receiver.recv().await.expect("initial event"),
            UiEvent::TaskProgress { .. }
        ));
        assert_eq!(
            receiver.recv().await.expect("completion event"),
            UiEvent::TaskCompleted { id: id.clone() }
        );

        let task = manager.get_task(&id).await.expect("task");
        assert_eq!(task.status, TaskStatus::Partial);
        assert_eq!(task.progress, 1.0);
        assert_eq!(task.message, "4 succeeded, 1 failed");
        assert!(task.completed_at.is_some());
    }

    #[tokio::test]
    async fn cancellation_stays_nonterminal_until_the_worker_finishes() {
        let events = UiEventBus::new();
        let mut receiver = events.subscribe();
        let manager = TaskManager::new(Arc::new(events));
        let (progress_tx, progress_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();

        let id = manager
            .spawn(
                NewTask {
                    name: "Copy".to_string(),
                    initial_message: "Starting".to_string(),
                },
                move |task| async move {
                    task.progress(0.5, "Halfway").await;
                    let _ = progress_tx.send(());
                    let _ = release_rx.await;
                    assert!(task.is_cancelled());
                    TaskOutcome::Cancelled("已取消".to_string())
                },
            )
            .await;
        progress_rx.await.expect("worker progress");

        assert_eq!(
            receiver.recv().await.expect("initial event"),
            UiEvent::TaskProgress {
                id: id.clone(),
                progress: 0.0,
                message: "Starting".to_string(),
            }
        );
        assert_eq!(
            receiver.recv().await.expect("progress event"),
            UiEvent::TaskProgress {
                id: id.clone(),
                progress: 0.5,
                message: "Halfway".to_string(),
            }
        );

        manager.cancel_task(&id).await.expect("cancel task");

        let task = manager.get_task(&id).await.expect("task");
        assert_eq!(task.status, TaskStatus::Cancelling);
        assert_eq!(task.message, "正在取消");
        assert!(task.completed_at.is_none());
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));

        let _ = release_tx.send(());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), receiver.recv())
                .await
                .expect("completion event timeout")
                .expect("cancellation completion event"),
            UiEvent::TaskCompleted { id: id.clone() }
        );

        let task = manager.get_task(&id).await.expect("task");
        assert_eq!(task.status, TaskStatus::Cancelled);
        assert_eq!(task.message, "已取消");
        assert!(task.completed_at.is_some());
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn worker_panics_become_failed_tasks() {
        let events = UiEventBus::new();
        let mut receiver = events.subscribe();
        let manager = TaskManager::new(Arc::new(events));

        let id = manager
            .spawn(
                NewTask {
                    name: "Backup".to_string(),
                    initial_message: "Starting".to_string(),
                },
                |_| async { panic!("worker exploded") },
            )
            .await;

        assert!(matches!(
            receiver.recv().await.expect("initial event"),
            UiEvent::TaskProgress { .. }
        ));
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), receiver.recv())
                .await
                .expect("completion event timeout")
                .expect("completion event"),
            UiEvent::TaskCompleted { id: id.clone() }
        );

        let task = manager.get_task(&id).await.expect("task");
        assert_eq!(task.status, TaskStatus::Failed);
        assert!(task.message.starts_with("任务异常终止:"));
        assert!(task.completed_at.is_some());
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }
}
