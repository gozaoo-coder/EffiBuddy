//! 会话管理命令：列表、读取、创建、删除、重命名、置顶、搜索、自动归类。

use effisuite_agent::{build_auto_classify_prompt, call_auto_classify_agent, AutoClassifyResult};
use effisuite_core::{Conversation, ConversationMeta, SearchHit};
use tauri::Emitter;

use crate::state::{now_ms, AppState};

/// conversation-title-updated 事件 payload（与 chat::payloads::ConversationTitlePayload 对齐）
///
/// 自动归类命令成功更新标题后 emit，前端立即刷新列表。
#[derive(Debug, serde::Serialize)]
struct TitleUpdatedPayload<'a> {
    conversation_id: &'a str,
    title: &'a str,
}

#[tauri::command]
pub(crate) async fn list_conversations(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ConversationMeta>, String> {
    let store = state.store.clone();
    store.list_meta().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn get_conversation(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Option<Conversation>, String> {
    let store = state.store.clone();
    store.load(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn create_conversation(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conv = Conversation::new(id.clone(), now_ms());
    state.store.save(&conv).await.map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub(crate) async fn delete_conversation(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state.store.delete(&id).await.map_err(|e| e.to_string())
}

/// 重命名会话标题
#[tauri::command]
pub(crate) async fn rename_conversation(
    state: tauri::State<'_, AppState>,
    id: String,
    title: String,
) -> Result<(), String> {
    state
        .store
        .rename(&id, title)
        .await
        .map_err(|e| e.to_string())
}

/// 置顶/取消置顶会话
#[tauri::command]
pub(crate) async fn toggle_pin_conversation(
    state: tauri::State<'_, AppState>,
    id: String,
    pinned: bool,
) -> Result<(), String> {
    state
        .store
        .set_pinned(&id, pinned, now_ms())
        .await
        .map_err(|e| e.to_string())
}

/// 跨会话搜索消息内容（基于存储层的简单关键词匹配）
#[tauri::command]
pub(crate) async fn search_conversations(
    state: tauri::State<'_, AppState>,
    query: String,
) -> Result<Vec<SearchHit>, String> {
    state.store.search(&query).await.map_err(|e| e.to_string())
}

/// 自动归类会话：调用 LLM 生成标题并建议归入哪个已有文件夹
///
/// 流程：
/// 1. 加载会话（不存在则返回 Err）
/// 2. 读取配置快照（锁临界区极短：仅 clone），校验 is_rig_ready
/// 3. 构造归类 prompt（最近 N 条消息 + 已有文件夹列表）
/// 4. 调用归类 agent（复用主 agent 的 api_key/base_url/model_name + 归类专用 preamble）
/// 5. 持久化标题到 store（store.rename）
/// 6. emit conversation-title-updated 事件，前端立即刷新列表
/// 7. 返回 AutoClassifyResult（title + folder），前端据此更新文件夹映射
///
/// 文件夹映射存储在前端 localStorage，后端不感知；folder 字段为已有文件夹名或 None。
/// 若 LLM 建议的文件夹不在已有列表中，parse 阶段已降级为 None。
#[tauri::command]
pub(crate) async fn auto_classify_conversation(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
    existing_folders: Vec<String>,
) -> Result<AutoClassifyResult, String> {
    // 1. 加载会话
    let conv = state
        .store
        .load(&conversation_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("会话 {conversation_id} 不存在"))?;

    if conv.messages.is_empty() {
        return Err("会话无消息，无法归类".to_string());
    }

    // 2. 读取配置快照（锁临界区极短：仅 clone）
    let config = state.config.read().await.clone();
    if !config.is_rig_ready() {
        return Err("未配置 api_key 或 backend 非 openai，无法调用归类 agent".to_string());
    }

    // 3. 构造归类 prompt
    let prompt = build_auto_classify_prompt(&conv.messages, &existing_folders);

    // 4. 调用归类 agent
    let result = call_auto_classify_agent(
        &config.api_key,
        &config.base_url,
        &config.model_name,
        &prompt,
        &existing_folders,
    )
    .await
    .map_err(|e| e.to_string())?;

    // 5. 持久化标题
    state
        .store
        .rename(&conversation_id, result.title.clone())
        .await
        .map_err(|e| e.to_string())?;

    // 6. emit 标题更新事件（与 set_title 工具的事件名一致）
    let _ = app_handle.emit(
        "conversation-title-updated",
        &TitleUpdatedPayload {
            conversation_id: &conversation_id,
            title: &result.title,
        },
    );

    tracing::info!(
        conversation_id = %conversation_id,
        title = %result.title,
        folder = ?result.folder,
        "自动归类完成"
    );

    Ok(result)
}
