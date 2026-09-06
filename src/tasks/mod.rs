use std::{collections::HashMap, future::Future, sync::Arc};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::platform::{UiEvent, UiEventSinkHandle};

pub(crate) const MAX_COMPLETED_TASKS: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

impl TaskStatus {
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Partial | Self::Failed | Self::Cancelled
        )
    }
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

    fn complete(&mut self, outcome: TaskOutcome) -> Option<UiEvent> {
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
        Some(UiEvent::TaskCompleted {
            task: Arc::new(self.task.clone()),
        })
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
            entry.task.progress = entry.task.progress.max(progress.clamp(0.0, 1.0));
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
            let event = entry.complete(outcome);
            prune_completed(&mut entries);
            event
        };
        if let Some(event) = event {
            self.inner.events.emit(event);
        }
    }
}

fn prune_completed(entries: &mut HashMap<String, TaskEntry>) {
    let mut completed = entries
        .values()
        .filter_map(|entry| {
            entry
                .task
                .completed_at
                .map(|at| (at, entry.task.id.clone()))
        })
        .collect::<Vec<_>>();
    completed.sort_unstable();
    let remove_count = completed.len().saturating_sub(MAX_COMPLETED_TASKS);
    for (_, id) in completed.into_iter().take(remove_count) {
        entries.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::oneshot;

    use super::*;
    use crate::platform::{UiEvent, UiEventBus};

    #[tokio::test]
    async fn history_budget_keeps_active_tasks_and_newest_terminal_outcomes() {
        let manager = TaskManager::new(Arc::new(UiEventBus::new()));
        let now = Utc::now();
        let mut entries = manager.inner.entries.lock().await;
        for index in 0..MAX_COMPLETED_TASKS + 5 {
            let id = index.to_string();
            entries.insert(
                id.clone(),
                TaskEntry {
                    task: BackgroundTask {
                        id,
                        name: "History".into(),
                        status: TaskStatus::Failed,
                        progress: 0.0,
                        message: "Failure remains inspectable".into(),
                        created_at: now,
                        completed_at: Some(now + chrono::Duration::seconds(index as i64)),
                    },
                    cancellation: None,
                    completion_event_sent: true,
                },
            );
        }
        let mut active = entries.get("0").unwrap().task.clone();
        active.id = "active".into();
        active.status = TaskStatus::Cancelling;
        active.completed_at = None;
        entries.insert(
            active.id.clone(),
            TaskEntry {
                task: active,
                cancellation: Some(CancellationToken::new()),
                completion_event_sent: false,
            },
        );
        prune_completed(&mut entries);
        assert_eq!(entries.len(), MAX_COMPLETED_TASKS + 1);
        assert!(entries.contains_key("active"));
        assert!(!entries.contains_key("4"));
        assert!(entries.contains_key("5"));
        assert!(entries.contains_key(&(MAX_COMPLETED_TASKS + 4).to_string()));
    }

    #[tokio::test]
    async fn completion_events_preserve_terminal_details_after_history_eviction() {
        for (outcome, status) in [
            (
                TaskOutcome::Completed("Done".to_string()),
                TaskStatus::Completed,
            ),
            (
                TaskOutcome::Partial("Some changes applied".to_string()),
                TaskStatus::Partial,
            ),
            (
                TaskOutcome::Failed("Write failed".to_string()),
                TaskStatus::Failed,
            ),
            (
                TaskOutcome::Cancelled("Cancelled".to_string()),
                TaskStatus::Cancelled,
            ),
        ] {
            let events = UiEventBus::new();
            let mut receiver = events.subscribe();
            let manager = TaskManager::new(Arc::new(events));
            manager.inner.entries.lock().await.insert(
                "first".to_string(),
                TaskEntry {
                    task: BackgroundTask {
                        id: "first".to_string(),
                        name: "Restore".to_string(),
                        status: TaskStatus::Running,
                        progress: 0.5,
                        message: "Working".to_string(),
                        created_at: Utc::now(),
                        completed_at: None,
                    },
                    cancellation: None,
                    completion_event_sent: false,
                },
            );
            manager.finish("first", outcome).await;
            let completed = manager.get_task("first").await.unwrap();
            {
                let mut entries = manager.inner.entries.lock().await;
                for index in 0..MAX_COMPLETED_TASKS {
                    let mut newer = completed.clone();
                    newer.id = format!("newer-{index}");
                    newer.completed_at = completed
                        .completed_at
                        .map(|at| at + chrono::Duration::seconds(index as i64 + 1));
                    entries.insert(
                        newer.id.clone(),
                        TaskEntry {
                            task: newer,
                            cancellation: None,
                            completion_event_sent: true,
                        },
                    );
                }
                prune_completed(&mut entries);
            }
            assert!(manager.get_task("first").await.is_none());
            let UiEvent::TaskCompleted { task } = receiver.recv().await.unwrap() else {
                panic!("expected terminal event");
            };
            assert_eq!(*task, completed);
            assert_eq!(task.status, status);
            assert_eq!(task.name, "Restore");
            assert!(task.completed_at.is_some());
        }
    }

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
            UiEvent::TaskCompleted {
                task: Arc::new(manager.get_task(&id).await.expect("retained task"))
            }
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
            UiEvent::TaskCompleted {
                task: Arc::new(manager.get_task(&id).await.expect("retained task"))
            }
        );

        let task = manager.get_task(&id).await.expect("task");
        assert_eq!(task.status, TaskStatus::Partial);
        assert_eq!(task.progress, 1.0);
        assert_eq!(task.message, "4 succeeded, 1 failed");
        assert!(task.completed_at.is_some());
    }

    #[tokio::test]
    async fn progress_events_never_move_backwards() {
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
                    task.progress(0.7, "Later").await;
                    task.progress(0.2, "Delayed update").await;
                    TaskOutcome::Completed("Done".to_string())
                },
            )
            .await;

        let _ = receiver.recv().await.expect("initial event");
        let _ = receiver.recv().await.expect("first progress event");
        assert_eq!(
            receiver.recv().await.expect("monotonic progress event"),
            UiEvent::TaskProgress {
                id,
                progress: 0.7,
                message: "Delayed update".to_string(),
            }
        );
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
            UiEvent::TaskCompleted {
                task: Arc::new(manager.get_task(&id).await.expect("retained task"))
            }
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
            UiEvent::TaskCompleted {
                task: Arc::new(manager.get_task(&id).await.expect("retained task"))
            }
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
