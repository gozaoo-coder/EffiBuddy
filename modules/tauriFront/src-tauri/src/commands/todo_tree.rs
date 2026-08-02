//! todoTree 命令：每会话任务清单的读取 / 保存 / 清除。
//!
//! todoTree 由 agent 的 `todo_write` 工具在任务执行中维护，也可由用户在右栏
//! "概览"卡片手动编辑。两者都落盘到 `TodoStore`（`<appdata>/todos/<conv_id>.json`），
//! 并常驻注入到该会话的上下文 prompt（`[当前任务清单]` 段）。
//!
//! 前端编辑后调用 `save_todo_tree` 保存；保存成功后 emit `todo-tree-updated`
//! 事件，agent 侧与其它页签据此刷新展示。

use effisuite_agent::tools::{TodoItem, TodoPriority, TodoStatus};
use tauri::Emitter;

use crate::state::AppState;

/// 获取指定会话的 todoTree（扁平 TodoItem 列表，前端还原为树）。
/// 会话不存在或无任务时返回空数组。
#[tauri::command]
pub(crate) async fn get_todo_tree(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<TodoItem>, String> {
    let items = state
        .todo_store
        .load(&conversation_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(items.unwrap_or_default())
}

/// 保存指定会话的 todoTree（整体替换）。前端编辑 / 新增 / 删除后调用。
/// 保存成功后 emit `todo-tree-updated` 事件，通知 agent 侧会话上下文刷新。
#[tauri::command]
pub(crate) async fn save_todo_tree(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
    todos: Vec<TodoItem>,
) -> Result<Vec<TodoItem>, String> {
    // 输入清洗：去空 content、校正同一时间仅一个 in_progress、非 completed 清空 summary
    let cleaned: Vec<TodoItem> = todos
        .into_iter()
        .filter(|t| !t.content.trim().is_empty())
        .map(|mut t| {
            if t.status != TodoStatus::Completed {
                t.summary = None;
            }
            t
        })
        .collect();
    let mut seen_in_progress = false;
    let cleaned: Vec<TodoItem> = cleaned
        .into_iter()
        .map(|mut t| {
            if t.status == TodoStatus::InProgress {
                if seen_in_progress {
                    t.status = TodoStatus::Pending;
                } else {
                    seen_in_progress = true;
                }
            }
            t
        })
        .collect();

    state
        .todo_store
        .save(&conversation_id, &cleaned)
        .await
        .map_err(|e| e.to_string())?;

    let _ = app_handle.emit(
        "todo-tree-updated",
        &serde_json::json!({ "conversation_id": conversation_id }),
    );
    Ok(cleaned)
}

/// 清除指定会话的 todoTree（恢复空任务清单）。
#[tauri::command]
pub(crate) async fn clear_todo_tree(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
) -> Result<(), String> {
    state
        .todo_store
        .delete(&conversation_id)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit(
        "todo-tree-updated",
        &serde_json::json!({ "conversation_id": conversation_id }),
    );
    Ok(())
}

// 让 TodoPriority 在模块内可见（供未来扩展 / 校验使用）
#[allow(dead_code)]
fn _validate_priority(p: TodoPriority) -> bool {
    matches!(p, TodoPriority::High | TodoPriority::Medium | TodoPriority::Low)
}
