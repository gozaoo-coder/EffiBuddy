//! 命令会话命令：前端底栏便签的列表恢复 / 手动结束。
//!
//! 会话本身由 agent 的 `shell_session_*` 工具创建与交互（写命令、读输出），
//! 前端只读展示工作状态。这里提供两个薄命令：
//! - `list_shell_sessions`：返回全部会话（短 ID / 名称 / 是否运行 / 最近命令），
//!   前端底栏组件挂载时据此恢复已有会话（避免漏掉组件挂载前启动的会话）。
//! - `kill_shell_session`：手动结束指定会话（如用户不想等它跑完）。

use effisuite_agent::ShellSessionInfo;
use tauri::State;

use crate::state::AppState;

/// 列出全部命令会话（按最近活跃倒序）。
#[tauri::command]
pub(crate) async fn list_shell_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<ShellSessionInfo>, String> {
    Ok(state.shell_sessions.list().await)
}

/// 结束指定命令会话。
#[tauri::command]
pub(crate) async fn kill_shell_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<String, String> {
    state.shell_sessions.kill(&session_id).await
}
