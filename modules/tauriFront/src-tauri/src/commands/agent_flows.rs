//! Agent Flow（智能体流程）命令：ComfyUI 风格可视化节点流程的增删改查
//!
//! 流程由节点（FlowNode）+ 连线（FlowEdge）组成，每个节点有输入/输出类型与参数。
//! 命令层仅做持久化转译，执行引擎后续在 agent 侧实现。

use effisuite_core::AgentFlow;
use tauri::State;

use crate::state::{now_ms, AppState};

/// 列出全部流程（按 updated_at 降序）
#[tauri::command]
pub(crate) async fn list_agent_flows(
    state: State<'_, AppState>,
) -> Result<Vec<AgentFlow>, String> {
    state
        .agent_flow_store
        .list()
        .await
        .map_err(|e| e.to_string())
}

/// 获取单个流程
#[tauri::command]
pub(crate) async fn get_agent_flow(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<AgentFlow>, String> {
    state.agent_flow_store.get(&id).await.map_err(|e| e.to_string())
}

/// 创建（或更新）一个流程。id 为空时自动生成。
#[tauri::command]
pub(crate) async fn save_agent_flow(
    state: State<'_, AppState>,
    flow: AgentFlow,
) -> Result<AgentFlow, String> {
    let now = now_ms();
    let mut flow = flow;
    if flow.id.is_empty() {
        flow.id = uuid::Uuid::new_v4().to_string();
        flow.created_at = now;
    }
    flow.updated_at = now;
    state
        .agent_flow_store
        .save(&flow)
        .await
        .map_err(|e| e.to_string())?;
    Ok(flow)
}

/// 删除一个流程
#[tauri::command]
pub(crate) async fn delete_agent_flow(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .agent_flow_store
        .delete(&id)
        .await
        .map_err(|e| e.to_string())
}