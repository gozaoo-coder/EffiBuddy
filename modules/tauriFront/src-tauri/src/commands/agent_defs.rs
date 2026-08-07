//! 自定义智能体（AgentDef）命令：建立 / 管理智能体
//!
//! 用户可在「自动化 → 建立智能体」中创建自定义 agent（角色、系统提示词、模型），
//! 主 agent / 子 agent 在召唤 sub-agent 时可指定某个自定义智能体定义，注入其
//! 系统提示词与模型，从而"召唤某个自定义智能体"。

use effisuite_core::AgentDef;
use tauri::State;

use crate::state::{now_ms, AppState};

/// 列出全部自定义智能体定义（按 created_at 降序）
#[tauri::command]
pub(crate) async fn list_agent_defs(
    state: State<'_, AppState>,
) -> Result<Vec<AgentDef>, String> {
    state
        .agent_def_store
        .list()
        .await
        .map_err(|e| e.to_string())
}

/// 获取单个自定义智能体定义
#[tauri::command]
pub(crate) async fn get_agent_def(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<AgentDef>, String> {
    state
        .agent_def_store
        .get(&id)
        .await
        .map_err(|e| e.to_string())
}

/// 创建（或更新）一个自定义智能体定义。id 为空时自动生成。
#[tauri::command]
pub(crate) async fn save_agent_def(
    state: State<'_, AppState>,
    def: AgentDef,
) -> Result<AgentDef, String> {
    let now = now_ms();
    let mut def = def;
    if def.id.is_empty() {
        def.id = uuid::Uuid::new_v4().to_string();
        def.created_at = now;
    }
    def.updated_at = now;
    state
        .agent_def_store
        .save(&def)
        .await
        .map_err(|e| e.to_string())?;
    Ok(def)
}

/// 删除一个自定义智能体定义
#[tauri::command]
pub(crate) async fn delete_agent_def(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .agent_def_store
        .delete(&id)
        .await
        .map_err(|e| e.to_string())
}