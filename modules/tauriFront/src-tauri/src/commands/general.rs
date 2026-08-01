//! 通用命令：问候语与 agent 后端查询。

use crate::state::AppState;

#[tauri::command]
pub(crate) fn greet(name: String) -> String {
    format!("Hello, {}! EffiSuite 已就绪。", name)
}

#[tauri::command]
pub(crate) async fn get_agent_backend(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let agent = state.agent.read().await.clone();
    Ok(agent.backend().to_string())
}
