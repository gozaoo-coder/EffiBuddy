//! 会话版本管理——自研快照 Tauri 命令层（薄封装）
//!
//! 与 `git_context` 命令的区别：这里**不依赖 git**，也不触碰工作区里用户自己的
//! `.git` 仓库（快照数据全部存放在 `<appdata>/effisuite/session_versions/`），
//! 因此对任意目录（哪怕是已有 git 仓库的项目）都安全无冲突。
//!
//! 命令一览：
//! - `snapshot_save`     手动保存当前工作区快照
//! - `snapshot_list`     快照列表（最新在前）
//! - `snapshot_status`   工作区与最新快照的差异状态（dirty / changes）
//! - `snapshot_restore`  恢复到指定快照（dry_run 可预览；恢复前自动保护快照）
//! - `snapshot_delete`   删除指定快照（不可删最新一条）
//!
//! 实际快照逻辑（内容寻址 / 忽略规则 / 去重 / 保留上限）全部在 `crate::snapshot_service`。

use std::path::PathBuf;

use tauri::State;

use crate::snapshot_service::{self, RestoreResult, SnapshotMeta, SnapshotSource, SnapshotStatus};
use crate::state::AppState;

/// 解析会话工作区目录（快照引擎只作用于会话级工作区）
async fn resolve_workspace_dir(
    state: &State<'_, AppState>,
    conversation_id: &str,
) -> Result<PathBuf, String> {
    let conv = state
        .store
        .load(conversation_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "会话不存在".to_string())?;
    conv.working_dir
        .map(PathBuf::from)
        .ok_or_else(|| {
            "该会话未设置工作区（可在聊天窗口的「工作区」入口设置目录），无法使用会话版本管理".to_string()
        })
}

/// 在阻塞线程池执行快照操作（文件扫描 / 复制为阻塞调用，避免卡住异步 runtime）
async fn run_blocking<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("快照任务执行失败：{e}"))?
}

/// 手动保存当前工作区快照；无改动时返回 `None`（不产生空快照）
#[tauri::command]
pub(crate) async fn snapshot_save(
    state: State<'_, AppState>,
    conversation_id: String,
    message: String,
) -> Result<Option<SnapshotMeta>, String> {
    let dir = resolve_workspace_dir(&state, &conversation_id).await?;
    run_blocking(move || {
        snapshot_service::save_snapshot(&conversation_id, &dir, &message, SnapshotSource::Manual)
    })
    .await
}

/// 快照列表（最新在前），供前端时间线渲染
#[tauri::command]
pub(crate) async fn snapshot_list(conversation_id: String) -> Result<Vec<SnapshotMeta>, String> {
    Ok(snapshot_service::list_snapshots(&conversation_id))
}

/// 工作区与最新快照的差异状态（含新增 / 修改 / 删除明细）
#[tauri::command]
pub(crate) async fn snapshot_status(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<SnapshotStatus, String> {
    let dir = resolve_workspace_dir(&state, &conversation_id).await?;
    run_blocking(move || Ok(snapshot_service::snapshot_status(&conversation_id, &dir))).await
}

/// 恢复到指定快照；`dry_run=true` 只预览将要发生的操作，不写工作区
#[tauri::command]
pub(crate) async fn snapshot_restore(
    state: State<'_, AppState>,
    conversation_id: String,
    snapshot_id: String,
    dry_run: bool,
) -> Result<RestoreResult, String> {
    let dir = resolve_workspace_dir(&state, &conversation_id).await?;
    run_blocking(move || {
        snapshot_service::restore_snapshot(&conversation_id, &snapshot_id, &dir, dry_run)
    })
    .await
}

/// 删除指定快照（保护：不可删除最新一条，保证始终可回溯）
#[tauri::command]
pub(crate) async fn snapshot_delete(
    conversation_id: String,
    snapshot_id: String,
) -> Result<(), String> {
    snapshot_service::delete_snapshot(&conversation_id, &snapshot_id)
}
