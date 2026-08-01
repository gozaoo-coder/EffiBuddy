//! 会话管理命令：列表、读取、创建、删除、重命名、置顶、搜索。

use effisuite_core::{Conversation, ConversationMeta, SearchHit};

use crate::state::{now_ms, AppState};

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
