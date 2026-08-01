//! RAG 记忆增强检索、永久记忆管理与上下文注入预览命令。
//!
//! - `search_memory`：跨会话历史记忆检索（BM25 词法 + 向量 embedding + RRF 混合）。
//! - `list/add/update/delete/clear_pinned_memories`：永久记忆 CRUD（用户主动"记住"的内容）。
//! - `get_context_preview`：返回当前 agent 对指定会话的上下文注入预览。

use effisuite_agent::ContextPreview;
use effisuite_core::{
    MemoryHit, MemoryStats, PinnedMemory, PinnedMemorySource, SearchMode,
};

use crate::state::{now_ms, AppState};

/// 跨会话历史记忆检索（RAG：BM25 词法 + 向量 embedding + RRF 混合）
///
/// 与 `search_conversations`（存储层简单关键词匹配）不同，本命令走 memory index：
/// - `lexical`：BM25 + IDF 加权，倒排表加速
/// - `vector`：embedding 余弦相似度（需配置 OpenAI 兼容 provider）
/// - `hybrid`：RRF 融合两路（默认推荐）
///
/// 自动排除当前活跃会话（若已通过 send_message 设置）。
#[tauri::command]
pub(crate) async fn search_memory(
    state: tauri::State<'_, AppState>,
    query: String,
    mode: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<MemoryHit>, String> {
    let mode = parse_search_mode(mode.as_deref());
    let limit = limit.unwrap_or(5).clamp(1, 50);
    // 读出当前会话 id（短暂读锁），检索时排除
    let exclude = state.current_conversation_id.read().await.clone();
    let hits = state
        .memory
        .search(&query, limit, mode, exclude.as_deref())
        .await;
    Ok(hits)
}

/// 返回 memory index 统计信息（条目数、唯一 token 数、已嵌入条目数、平均文档长度）
#[tauri::command]
pub(crate) async fn get_memory_stats(state: tauri::State<'_, AppState>) -> Result<MemoryStats, String> {
    Ok(state.memory.stats().await)
}

/// 解析前端传入的检索模式字符串为 SearchMode 枚举
fn parse_search_mode(s: Option<&str>) -> SearchMode {
    match s.map(str::to_ascii_lowercase).as_deref() {
        Some("lexical") | Some("bm25") | Some("keyword") => SearchMode::Lexical,
        Some("vector") | Some("embedding") | Some("semantic") => SearchMode::Vector,
        _ => SearchMode::Hybrid,
    }
}

/// 列出全部永久记忆（按 created_at 降序，新的在前）
#[tauri::command]
pub(crate) async fn list_pinned_memories(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<PinnedMemory>, String> {
    Ok(state.pinned_memory.list().await)
}

/// 新增一条永久记忆（来源固定为 Manual），返回新 id
#[tauri::command]
pub(crate) async fn add_pinned_memory(
    state: tauri::State<'_, AppState>,
    content: String,
    category: Option<String>,
) -> Result<String, String> {
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err("content 不能为空".to_string());
    }
    if content.chars().count() > 2000 {
        return Err("content 过长（>2000 字符），请精简后再添加".to_string());
    }
    state
        .pinned_memory
        .add_simple(
            content,
            category,
            PinnedMemorySource::Manual,
            None,
            now_ms(),
        )
        .await
        .map_err(|e| e.to_string())
}

/// 更新指定 id 的永久记忆内容与/或分类。
/// `category` 为 `null` 表示不变；为空字符串表示清空分类（前端约定）。
#[tauri::command]
pub(crate) async fn update_pinned_memory(
    state: tauri::State<'_, AppState>,
    id: String,
    content: Option<String>,
    category: Option<String>,
) -> Result<(), String> {
    // 区分"未提供 category"（None = 不变）与"清空 category"（Some("") = 清空）
    let category_opt = match category {
        None => None,
        Some(s) if s.is_empty() => Some(None),
        Some(s) => Some(Some(s)),
    };
    state
        .pinned_memory
        .update(&id, content, category_opt)
        .await
        .map_err(|e| e.to_string())
}

/// 删除指定 id 的永久记忆
#[tauri::command]
pub(crate) async fn delete_pinned_memory(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .pinned_memory
        .delete(&id)
        .await
        .map_err(|e| e.to_string())
}

/// 清空所有永久记忆（危险操作，前端应有二次确认）
#[tauri::command]
pub(crate) async fn clear_pinned_memories(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.pinned_memory.clear().await.map_err(|e| e.to_string())
}

/// 返回当前 agent 对指定会话的上下文注入预览。
///
/// - 加载该会话的完整消息历史
/// - 调用 `agent.context_preview(&messages)` 拿到结构化预览
/// - 返回 `Some(ContextPreview)` 或 `None`（MockAgent 后端）
#[tauri::command]
pub(crate) async fn get_context_preview(
    state: tauri::State<'_, AppState>,
    conversation_id: Option<String>,
) -> Result<Option<ContextPreview>, String> {
    let agent = state.agent.read().await.clone();
    let messages = if let Some(id) = conversation_id.as_deref() {
        // 加载指定会话的完整消息历史；不存在或加载失败视为空列表
        match state.store.load(id).await {
            Ok(Some(conv)) => conv.messages,
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };
    Ok(agent.context_preview(&messages).await)
}
