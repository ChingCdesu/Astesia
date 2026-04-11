use tauri::State;
use crate::state::AppState;
use crate::tasks::BackgroundTask;

#[tauri::command]
pub async fn list_tasks(state: State<'_, AppState>) -> Result<Vec<BackgroundTask>, String> {
    Ok(state.task_manager.list_tasks().await)
}

#[tauri::command]
pub async fn get_task(state: State<'_, AppState>, task_id: String) -> Result<Option<BackgroundTask>, String> {
    Ok(state.task_manager.get_task(&task_id).await)
}

#[tauri::command]
pub async fn cancel_task(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.task_manager.cancel_task(&task_id).await
}
