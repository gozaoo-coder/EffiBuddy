//! 已落盘子 agent 会话命令：前端据此提供独立历史入口（查看 / 继续 / 删除）。
//!
//! 子 agent 会话由 `SubAgentManager` 在执行过程中实时增量落盘到
//! `SubAgentStore`（`<appdata>/subagents/<session_id>.json`），本模块提供：
//! - `list_sub_agent_sessions`：列出全部已落盘会话元信息（历史列表）。
//! - `get_sub_agent_session`：按 session_id 取完整会话文档（前端 hydrate 渲染）。
//! - `delete_sub_agent_session`：删除指定已落盘会话。

use effisuite_core::{SubAgentSessionDoc, SubAgentSessionMeta, SubAgentStore};
use tauri::State;

use crate::state::AppState;

/// 列出全部已落盘子 agent 会话元信息（不含消息体，按最近更新倒序）。
#[tauri::command]
pub(crate) async fn list_sub_agent_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<SubAgentSessionMeta>, String> {
    state
        .sub_agent_store
        .list_meta()
        .await
        .map_err(|e| e.to_string())
}

/// 按 session_id 取完整已落盘子 agent 会话（含续聊历史消息），不存在返回 None。
#[tauri::command]
pub(crate) async fn get_sub_agent_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<SubAgentSessionDoc>, String> {
    state
        .sub_agent_store
        .load(&session_id)
        .await
        .map_err(|e| e.to_string())
}

/// 删除指定已落盘子 agent 会话（不存在返回 Ok）。
#[tauri::command]
pub(crate) async fn delete_sub_agent_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    state
        .sub_agent_store
        .delete(&session_id)
        .await
        .map_err(|e| e.to_string())
}