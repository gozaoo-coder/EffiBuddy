//! 运行时 agent 公共会话交流池命令：前端会话列表据此展示各会话 agent 的运行状态。
//!
//! 交流池由 agent 的 `pool_report` / `pool_at` / `pool_reply` 工具维护（长任务登记、
//! 状态上报、收件箱 @ 消息），本模块提供只读查询与清空能力：
//! - `list_pool`：列出全部条目（含已完成），前端按 conversation_id 聚合，
//!   在会话列表上展示「进行中 / 等待中 / 已完成」状态标记。
//! - `get_pool_entry`：按会话 id 查询主 agent 条目（含收件箱 @ 消息）。
//! - `clear_pool`：清空交流池（调试 / 管理用）。

use effisuite_agent::PoolEntry;
use tauri::State;

use crate::state::AppState;

/// 列出交流池全部条目（含已完成；按最近更新倒序）。
///
/// 前端会话列表按 `conversation_id` 聚合：一个会话（含其子 agent）对应
/// 若干条目，取其中活跃状态（进行中 / 等待中）展示为会话运行状态。
#[tauri::command]
pub(crate) async fn list_pool(state: State<'_, AppState>) -> Result<Vec<PoolEntry>, String> {
    Ok(state.agent_pool.list().await)
}

/// 按会话 id 查询主 agent 的交流池条目（未登记返回 None）。
#[tauri::command]
pub(crate) async fn get_pool_entry(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Option<PoolEntry>, String> {
    Ok(state.agent_pool.find(&conversation_id).await)
}

/// 清空交流池全部条目（调试 / 管理用；不影响会话本身）。
#[tauri::command]
pub(crate) async fn clear_pool(state: State<'_, AppState>) -> Result<(), String> {
    for e in state.agent_pool.list().await {
        state
            .agent_pool
            .remove(&e.agent_id)
            .await
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}
