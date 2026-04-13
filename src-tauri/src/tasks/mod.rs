use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

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
    Completed,
    Failed,
    Cancelled,
}

pub struct TaskManager {
    pub tasks: Arc<Mutex<HashMap<String, BackgroundTask>>>,
    pub cancellation_tokens: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            cancellation_tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a cancellation token for a task so it can be cancelled via cancel_task.
    pub async fn register_token(&self, task_id: &str, token: CancellationToken) {
        let mut tokens = self.cancellation_tokens.lock().await;
        tokens.insert(task_id.to_string(), token);
    }

    pub async fn list_tasks(&self) -> Vec<BackgroundTask> {
        let tasks = self.tasks.lock().await;
        let mut list: Vec<BackgroundTask> = tasks.values().cloned().collect();
        list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        list
    }

    pub async fn get_task(&self, id: &str) -> Option<BackgroundTask> {
        let tasks = self.tasks.lock().await;
        tasks.get(id).cloned()
    }

    pub async fn cancel_task(&self, id: &str) -> Result<(), String> {
        let tokens = self.cancellation_tokens.lock().await;
        if let Some(token) = tokens.get(id) {
            token.cancel();
            drop(tokens);
            let mut tasks = self.tasks.lock().await;
            if let Some(task) = tasks.get_mut(id) {
                task.status = TaskStatus::Cancelled;
                task.message = "已取消".to_string();
                task.completed_at = Some(Utc::now());
            }
            Ok(())
        } else {
            Err("任务不存在或已完成".to_string())
        }
    }
}
