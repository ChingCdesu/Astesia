use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use chrono::{DateTime, Utc};
use tauri::Emitter;

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
    cancellation_tokens: Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            cancellation_tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn submit_task<F>(
        &self,
        name: String,
        app_handle: tauri::AppHandle,
        task_fn: F,
    ) -> String
    where
        F: FnOnce(String, tauri::AppHandle, tokio_util::sync::CancellationToken) -> tokio::task::JoinHandle<Result<(), String>>
            + Send
            + 'static,
    {
        let id = uuid::Uuid::new_v4().to_string();
        let token = tokio_util::sync::CancellationToken::new();

        let task = BackgroundTask {
            id: id.clone(),
            name: name.clone(),
            status: TaskStatus::Running,
            progress: 0.0,
            message: "开始执行...".to_string(),
            created_at: Utc::now(),
            completed_at: None,
        };

        {
            let mut tasks = self.tasks.lock().await;
            tasks.insert(id.clone(), task);
        }
        {
            let mut tokens = self.cancellation_tokens.lock().await;
            tokens.insert(id.clone(), token.clone());
        }

        let tasks_ref = self.tasks.clone();
        let tokens_ref = self.cancellation_tokens.clone();
        let task_id = id.clone();
        let app = app_handle.clone();

        let handle = task_fn(task_id.clone(), app_handle, token);

        tokio::spawn(async move {
            let result = handle.await;
            let mut tasks = tasks_ref.lock().await;
            if let Some(task) = tasks.get_mut(&task_id) {
                match result {
                    Ok(Ok(())) => {
                        task.status = TaskStatus::Completed;
                        task.progress = 1.0;
                        task.message = "完成".to_string();
                    }
                    Ok(Err(e)) => {
                        task.status = TaskStatus::Failed;
                        task.message = e;
                    }
                    Err(e) => {
                        task.status = TaskStatus::Failed;
                        task.message = format!("任务异常: {}", e);
                    }
                }
                task.completed_at = Some(Utc::now());
            }
            let _ = app.emit("task-complete", serde_json::json!({
                "id": task_id,
            }));
            // Cleanup token
            let mut tokens = tokens_ref.lock().await;
            tokens.remove(&task_id);
        });

        id
    }

    pub async fn update_progress(
        &self,
        task_id: &str,
        progress: f32,
        message: String,
        app_handle: &tauri::AppHandle,
    ) {
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.progress = progress;
            task.message = message.clone();
        }
        let _ = app_handle.emit("task-progress", serde_json::json!({
            "id": task_id,
            "progress": progress,
            "message": message,
        }));
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
