//! git 上下文版本管理 Tauri 命令层（薄封装）
//!
//! 职责：
//! 1. 把前端参数（`scope`: chat | workspace + `conversation_id`）解析为本地目录；
//! 2. 转调 `crate::git_service` 的同步 git 逻辑（用 `spawn_blocking` 隔离阻塞调用）；
//! 3. 统一返回结构化 `GitRepoInfo` / `GitSaveResult`，错误信息透传清晰文案。
//!
//! 命令一览：
//! - `git_context_status`  查询仓库状态（分支/未提交改动/历史）
//! - `git_context_init`    初始化仓库（git init + 首次提交）
//! - `git_context_branch`  开分支并切换
//! - `git_context_save`    保存快照（add -A . + commit）
//! - `git_context_revert`  撤回（撤销最近提交 / 恢复到指定提交的文件状态）
//! - `git_context_checkout` 回溯（检出到指定提交，detached HEAD）
//! - `git_context_history` 历史列表（含状态）
//!
//! 实际 git 逻辑与安全边界全部在 `git_service`，本文件只做参数解析与转发。

use std::path::PathBuf;

use tauri::State;

use crate::git_service::{self, GitRepoInfo, GitSaveResult};
use crate::state::AppState;

/// 解析 scope 到目标目录：
/// - `chat`：`<appdata>/effisuite/conversations`（聊天记录目录本身，不污染其他数据目录）
/// - `workspace`：会话级 working_dir（未设置则报错并给出提示）
async fn resolve_dir(
    state: &State<'_, AppState>,
    scope: &str,
    conversation_id: &str,
) -> Result<PathBuf, String> {
    match scope {
        "chat" => Ok(crate::paths::conversations_dir()),
        "workspace" => {
            let conv = state
                .store
                .load(conversation_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "会话不存在".to_string())?;
            conv.working_dir
                .map(PathBuf::from)
                .ok_or_else(|| {
                    "该会话未设置工作区（可在聊天窗口的「工作区」入口设置目录），无法使用工作区版本管理".to_string()
                })
        }
        other => Err(format!(
            "未知的仓库范围：{other}（仅支持 chat / workspace）"
        )),
    }
}

/// 在阻塞线程池执行 git 操作（git CLI 为同步阻塞调用，避免卡住异步 runtime）
async fn run_blocking<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("git 任务执行失败：{e}"))?
}

/// 查询仓库状态（未初始化时返回 `is_repo=false`，不自动 init）
#[tauri::command]
pub(crate) async fn git_context_status(
    state: State<'_, AppState>,
    scope: String,
    conversation_id: String,
) -> Result<GitRepoInfo, String> {
    let dir = resolve_dir(&state, &scope, &conversation_id).await?;
    run_blocking(move || git_service::status(&dir)).await
}

/// 初始化仓库：git init（若已在其他 git 仓库内则复用），无提交时做首次提交
#[tauri::command]
pub(crate) async fn git_context_init(
    state: State<'_, AppState>,
    scope: String,
    conversation_id: String,
) -> Result<GitRepoInfo, String> {
    let dir = resolve_dir(&state, &scope, &conversation_id).await?;
    run_blocking(move || {
        git_service::ensure_repo(&dir)?;
        git_service::status(&dir)
    })
    .await
}

/// 开分支并切换（`git checkout -b <name>`）
#[tauri::command]
pub(crate) async fn git_context_branch(
    state: State<'_, AppState>,
    scope: String,
    conversation_id: String,
    name: String,
) -> Result<GitRepoInfo, String> {
    let dir = resolve_dir(&state, &scope, &conversation_id).await?;
    run_blocking(move || {
        git_service::ensure_repo(&dir)?;
        git_service::create_branch(&dir, &name)?;
        git_service::status(&dir)
    })
    .await
}

/// 保存快照：`git add -A .` + commit（paths 限定在当前目录）
#[tauri::command]
pub(crate) async fn git_context_save(
    state: State<'_, AppState>,
    scope: String,
    conversation_id: String,
    message: String,
) -> Result<GitSaveResult, String> {
    let dir = resolve_dir(&state, &scope, &conversation_id).await?;
    run_blocking(move || {
        git_service::ensure_repo(&dir)?;
        git_service::save(&dir, &message)
    })
    .await
}

/// 撤回：commit=None 撤销最近一次提交（soft，改动保留）；commit=Some(hash) 恢复到该提交
#[tauri::command]
pub(crate) async fn git_context_revert(
    state: State<'_, AppState>,
    scope: String,
    conversation_id: String,
    commit: Option<String>,
) -> Result<GitRepoInfo, String> {
    let dir = resolve_dir(&state, &scope, &conversation_id).await?;
    run_blocking(move || {
        git_service::ensure_repo(&dir)?;
        git_service::revert(&dir, commit)?;
        git_service::status(&dir)
    })
    .await
}

/// 回溯：检出到指定提交（detached HEAD），前置校验无未提交改动
#[tauri::command]
pub(crate) async fn git_context_checkout(
    state: State<'_, AppState>,
    scope: String,
    conversation_id: String,
    commit: String,
) -> Result<GitRepoInfo, String> {
    let dir = resolve_dir(&state, &scope, &conversation_id).await?;
    run_blocking(move || {
        git_service::ensure_repo(&dir)?;
        git_service::checkout(&dir, &commit)?;
        git_service::status(&dir)
    })
    .await
}

/// 历史列表（含仓库状态，与 status 同构返回）
#[tauri::command]
pub(crate) async fn git_context_history(
    state: State<'_, AppState>,
    scope: String,
    conversation_id: String,
) -> Result<GitRepoInfo, String> {
    let dir = resolve_dir(&state, &scope, &conversation_id).await?;
    run_blocking(move || git_service::status(&dir)).await
}
