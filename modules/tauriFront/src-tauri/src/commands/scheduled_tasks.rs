//! 定时任务（ScheduledTask）管理命令。

use effisuite_core::ScheduledTask;

use crate::state::{now_ms, AppState};

#[tauri::command]
pub(crate) async fn list_scheduled_tasks(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ScheduledTask>, String> {
    state.schedule_store.list().await.map_err(|e| e.to_string())
}

/// 创建定时任务，返回 id。空 id 自动生成
#[tauri::command]
pub(crate) async fn create_scheduled_task(
    state: tauri::State<'_, AppState>,
    mut task: ScheduledTask,
) -> Result<String, String> {
    if task.id.is_empty() {
        task.id = uuid::Uuid::new_v4().to_string();
    }
    if task.created_at == 0 {
        task.created_at = now_ms();
    }
    let id = task.id.clone();
    state
        .schedule_store
        .save(&task)
        .await
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub(crate) async fn delete_scheduled_task(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .schedule_store
        .delete(&id)
        .await
        .map_err(|e| e.to_string())
}

/// 启用/停用定时任务
#[tauri::command]
pub(crate) async fn toggle_scheduled_task(
    state: tauri::State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut task = state
        .schedule_store
        .get(&id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("定时任务 {} 不存在", id))?;
    task.enabled = enabled;
    state
        .schedule_store
        .save(&task)
        .await
        .map_err(|e| e.to_string())
}
