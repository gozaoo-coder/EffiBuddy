//! 常用工作区（Favorite Workspace）命令：用户在「会话工作区」面板收藏/切换/删除常用目录。
//!
//! - `list_favorite_workspaces`：列出全部收藏（按 created_at 降序，新的在前）
//! - `add_favorite_workspace`：收藏一个工作区路径（相同路径幂等，返回既有 id）
//! - `delete_favorite_workspace`：删除指定收藏（不存在视为成功）

use effisuite_core::FavoriteWorkspace;

use crate::state::AppState;

/// 列出全部常用工作区（按 created_at 降序，新的在前）
#[tauri::command]
pub(crate) async fn list_favorite_workspaces(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<FavoriteWorkspace>, String> {
    Ok(state.favorite_workspaces.list().await)
}

/// 收藏一个工作区路径，返回 id。
///
/// 相同路径已收藏时幂等返回既有 id（不重复收藏）。
#[tauri::command]
pub(crate) async fn add_favorite_workspace(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    state
        .favorite_workspaces
        .add(path)
        .await
        .map_err(|e| e.to_string())
}

/// 删除指定 id 的收藏；不存在视为成功
#[tauri::command]
pub(crate) async fn delete_favorite_workspace(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .favorite_workspaces
        .delete(&id)
        .await
        .map_err(|e| e.to_string())
}
