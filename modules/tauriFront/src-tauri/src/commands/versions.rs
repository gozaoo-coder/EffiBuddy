//! 会话版本控制 Tauri 命令层（git 风格：分支 / 临时版本 / 回溯 / 撤回 / 检出）
//!
//! 薄封装：参数解析 + 转发到 [`ConversationStore`] 的 `version_*` 委托方法，
//! 业务逻辑全部在 `effisuite_core::versions` 模块。所有破坏性操作
//! （回溯/撤回/检出）在前端先弹确认框，后端也内置检查点（reflog 语义）兜底。

use effisuite_core::{RefSummary, VersionList, VersionOpResult};
use tauri::State;

use crate::state::{now_ms, AppState};

/// 会话版本列表（当前分支提交链 + 全部引用）
#[tauri::command]
pub(crate) async fn version_list(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<VersionList, String> {
    state
        .store
        .version_list(&conversation_id)
        .await
        .map_err(|e| e.to_string())
}

/// 开启分支：从包含 `message_id` 的消息点创建新分支并切换 HEAD，
/// 工作区同步为该消息点快照（其后的消息被留在原分支）。
#[tauri::command]
pub(crate) async fn version_create_branch(
    state: State<'_, AppState>,
    conversation_id: String,
    message_id: String,
) -> Result<VersionOpResult, String> {
    state
        .store
        .version_create_branch(&conversation_id, &message_id, now_ms())
        .await
        .map_err(|e| e.to_string())
}

/// 保存临时版本：在包含 `message_id` 的消息点打 `temp-*` 书签（不移动 HEAD）
#[tauri::command]
pub(crate) async fn version_save_temp(
    state: State<'_, AppState>,
    conversation_id: String,
    message_id: String,
    note: String,
) -> Result<RefSummary, String> {
    state
        .store
        .version_save_temp(&conversation_id, &message_id, note, now_ms())
        .await
        .map_err(|e| e.to_string())
}

/// 回溯版本：重置 HEAD 到包含 `message_id` 的提交（丢弃其后消息）
#[tauri::command]
pub(crate) async fn version_rollback(
    state: State<'_, AppState>,
    conversation_id: String,
    message_id: String,
) -> Result<VersionOpResult, String> {
    state
        .store
        .version_rollback(&conversation_id, &message_id, now_ms())
        .await
        .map_err(|e| e.to_string())
}

/// 撤回至此消息前：重置 HEAD 到该消息提交的父提交（丢弃该消息及其后全部）
#[tauri::command]
pub(crate) async fn version_undo_before(
    state: State<'_, AppState>,
    conversation_id: String,
    message_id: String,
) -> Result<VersionOpResult, String> {
    state
        .store
        .version_undo_before(&conversation_id, &message_id, now_ms())
        .await
        .map_err(|e| e.to_string())
}

/// 检出到指定引用（分支 / 临时版本 / 检查点），工作区同步为对应快照
#[tauri::command]
pub(crate) async fn version_checkout(
    state: State<'_, AppState>,
    conversation_id: String,
    ref_name: String,
) -> Result<VersionOpResult, String> {
    state
        .store
        .version_checkout(&conversation_id, &ref_name, now_ms())
        .await
        .map_err(|e| e.to_string())
}

/// 删除引用（临时版本 / 检查点 / 分支；main 不可删除）
#[tauri::command]
pub(crate) async fn version_delete_ref(
    state: State<'_, AppState>,
    conversation_id: String,
    ref_name: String,
) -> Result<(), String> {
    state
        .store
        .version_delete_ref(&conversation_id, &ref_name)
        .await
        .map_err(|e| e.to_string())
}
