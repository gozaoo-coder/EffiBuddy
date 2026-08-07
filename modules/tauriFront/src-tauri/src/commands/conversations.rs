//! 会话管理命令：列表、读取、创建、删除、重命名、置顶、搜索、自动归类。

use std::sync::Arc;

use effisuite_agent::{build_auto_classify_prompt, call_auto_classify_agent, AutoClassifyResult};
use effisuite_core::{AgentConfig, Conversation, ConversationMeta, ConversationStore, SearchHit};
use tauri::Emitter;
use tokio::sync::RwLock;

use crate::state::{now_ms, AppState};

/// conversation-title-updated 事件 payload（与 chat::payloads::ConversationTitlePayload 对齐）
///
/// 自动归类命令成功更新标题后 emit，前端立即刷新列表。
#[derive(Debug, serde::Serialize)]
struct TitleUpdatedPayload<'a> {
    conversation_id: &'a str,
    title: &'a str,
}

/// 自动归类核心逻辑：加载会话 → 校验配置 → 调用归类 agent → 落盘标题 → emit 事件。
///
/// 从 `auto_classify_conversation` 命令抽取，供命令层手动归类与 `send_message_stream`
/// 首次消息后台命名复用，避免在聊天命令里重复实现一遍归类流程。
///
/// - `store` / `config`：会话存储与配置句柄（Arc 克隆廉价）
/// - `existing_folders`：已有文件夹列表（首次消息命名传空，仅命名不改文件夹）
pub(crate) async fn run_auto_classify(
    store: &ConversationStore,
    config: &Arc<RwLock<Arc<AgentConfig>>>,
    app_handle: &tauri::AppHandle,
    conversation_id: &str,
    existing_folders: &[String],
) -> Result<AutoClassifyResult, String> {
    // 1. 加载会话
    let conv = store
        .load(conversation_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("会话 {conversation_id} 不存在"))?;

    if conv.messages.is_empty() {
        return Err("会话无消息，无法归类".to_string());
    }

    // 2. 读取配置快照（Arc clone 廉价，不再深拷贝 AgentConfig）
    let config = config.read().await.clone();
    if !config.is_rig_ready() {
        return Err("未配置 api_key 或 backend 非 openai，无法调用归类 agent".to_string());
    }

    // 3. 构造归类 prompt
    let prompt = build_auto_classify_prompt(&conv.messages, existing_folders);

    // 4. 调用归类 agent（优先使用 title_model_id，回退到 active_model_id）
    let (api_key, base_url, model_name) = config
        .resolve_title_model()
        .ok_or_else(|| "未配置命名模型".to_string())?;
    let result = call_auto_classify_agent(
        &api_key,
        &base_url,
        &model_name,
        &prompt,
        existing_folders,
    )
    .await
    .map_err(|e| e.to_string())?;

    // 5. 持久化标题
    store
        .rename(conversation_id, result.title.clone())
        .await
        .map_err(|e| e.to_string())?;

    // 6. emit 标题更新事件（与 set_title 工具的事件名一致）
    let _ = app_handle.emit(
        "conversation-title-updated",
        &TitleUpdatedPayload {
            conversation_id,
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

/// 首次消息自动命名：新对话发出第一条 prompt 后，后台为它生成一次标题。
///
/// 触发条件：`conv.title.is_none()` 且 `conv.messages.len() == 1`（刚 append 的首条用户消息）。
/// 满足条件才 spawn 后台任务，且只做命名（existing_folders 传空，不改文件夹归类）。
/// 命名失败仅记录 warn，不阻塞 / 不回抛给发送流。
pub(crate) fn maybe_auto_title_first_message(
    store: Arc<ConversationStore>,
    config: Arc<RwLock<Arc<AgentConfig>>>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
    is_first_message: bool,
) {
    if !is_first_message {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_auto_classify(&store, &config, &app_handle, &conversation_id, &[])
            .await
        {
            tracing::warn!(
                conversation_id = %conversation_id,
                error = %e,
                "首次消息自动命名失败"
            );
        }
    });
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
    // 删除会话的同时清理其交流池条目（含子 agent），避免残留"进行中"状态
    state
        .agent_pool
        .remove_by_conversation(&id)
        .await
        .map_err(|e| e.to_string())?;
    state.store.delete(&id).await.map_err(|e| e.to_string())
}

/// 批量删除会话结果：成功数 + 失败 id 列表（部分失败时前端据此提示）
#[derive(Debug, serde::Serialize)]
pub(crate) struct BatchDeleteResult {
    /// 成功删除的会话数
    pub success: usize,
    /// 删除失败的会话 id（IO 错误等，不短路其余删除）
    pub failed: Vec<String>,
}

/// 批量删除多个会话。
///
/// 相比循环调用 `delete_conversation`：
/// - 交流池清理：单次写锁 + 最多一次持久化（`remove_by_conversations`）
/// - 会话文件删除：逐个执行但不短路，收集失败项继续删除剩余会话
/// - 返回 `BatchDeleteResult`，前端据此展示「成功 N 条 / 失败 M 条」
#[tauri::command]
pub(crate) async fn delete_conversations(
    state: tauri::State<'_, AppState>,
    ids: Vec<String>,
) -> Result<BatchDeleteResult, String> {
    if ids.is_empty() {
        return Ok(BatchDeleteResult {
            success: 0,
            failed: vec![],
        });
    }

    // 1. 批量清理交流池条目（单次锁临界区）
    state
        .agent_pool
        .remove_by_conversations(&ids)
        .await
        .map_err(|e| e.to_string())?;

    // 2. 逐个删除会话文件；不短路，收集失败项
    let store = state.store.clone();
    let mut success = 0usize;
    // 预分配容量，多数情况下失败为 0
    let mut failed = Vec::with_capacity(0);
    for id in &ids {
        match store.delete(id).await {
            Ok(()) => success += 1,
            Err(e) => {
                tracing::warn!(conversation_id = %id, error = %e, "批量删除会话失败");
                failed.push(id.clone());
            }
        }
    }

    tracing::info!(
        total = ids.len(),
        success,
        failed = failed.len(),
        "批量删除会话完成"
    );

    Ok(BatchDeleteResult { success, failed })
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
/// 委托给 [`run_auto_classify`] 核心逻辑。文件夹映射存储在前端 localStorage，
/// 后端不感知；folder 字段为已有文件夹名或 None（若 LLM 建议不在已有列表中，
/// parse 阶段已降级为 None）。
#[tauri::command]
pub(crate) async fn auto_classify_conversation(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
    existing_folders: Vec<String>,
) -> Result<AutoClassifyResult, String> {
    run_auto_classify(
        &state.store,
        &state.config,
        &app_handle,
        &conversation_id,
        &existing_folders,
    )
    .await
}
