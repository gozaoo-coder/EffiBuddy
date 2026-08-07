//! todo_write 工具：结构化待办列表管理
//!
//! 让 agent 在执行复杂多步任务时维护一份可演进的待办列表，显式跟踪
//! "正在做什么 / 接下来做什么 / 已完成什么"。
//!
//! # 核心语义
//!
//! - **替换模式**（`merge=false`，默认）：用传入的 `todos` 整体替换现有列表
//! - **合并模式**（`merge=true`）：按 `id` 合并——已存在的 id 更新字段，
//!   新 id 追加，未提及的 id 原样保留
//!
//! # 写入后自动校正的不变量
//!
//! 1. 同一时刻至多一个 `in_progress`：若传入多个，只保留第一个（按写入顺序），
//!    其余降级为 `pending`
//! 2. `summary` 仅在 `completed` 时有意义；非 completed 的 summary 会被清空
//! 3. 列表始终按 `status (in_progress > pending > completed)` →
//!    `priority (high > medium > low)` → 写入顺序 稳定排序
//!
//! # 共享状态与锁
//!
//! 列表存于 `Arc<RwLock<Vec<TodoItem>>>`：读多写少，agent 可通过 [`TodoWriteTool::state`]
//! 拿到同一份句柄，把当前待办注入到 prompt 上下文。锁临界区仅做数据替换/合并/排序与
//! 一次轻量格式化（待办列表天然很小，微秒级），不做任何 I/O。
//!
//! # id 生成
//!
//! 传入 `id` 为空时自动生成递增数字 id（取现有 + 传入中最大数字 id + 1），
//! 保证 merge 时可被 LLM 稳定引用。

use std::sync::Arc;

use rig_core::tool::Tool;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

// =========================================================
// 数据模型
// =========================================================

/// 任务优先级
///
/// 派生 `Ord` 后 `High < Medium < Low`（按声明顺序）；排序时升序取，
/// 自然得到 high → medium → low 的展示顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TodoPriority {
    High,
    Medium,
    Low,
}

/// 任务状态
///
/// serde 用 `snake_case`，使 `InProgress` 序列化为 `in_progress`，
/// 与输出展示串一致，LLM 传参与回显统一。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

/// 单个待办项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub priority: TodoPriority,
    pub status: TodoStatus,
    /// 仅在标记为 completed 时可选填入的完成总结
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// 父任务 id（树形层级）。None 表示根任务；Some(id) 表示该 id 任务的子任务。
    /// 旧数据无此字段时默认 None（根任务），保证向后兼容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,

    // ===== 高级待办字段（v2） =====

    /// 紧急程度: "urgent" | "not_urgent"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub urgency: Option<String>,

    /// 截止日期（ISO 8601 日期字符串，如 "2026-08-15"）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,

    /// 截止时间（ISO 8601 时间字符串，如 "14:30"）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_time: Option<String>,

    /// 提前提醒分钟数（如 30 表示提前30分钟提醒）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder_minutes_before: Option<u32>,

    /// 重复类型: "once" | "daily" | "cumulative"，默认 "once"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_type: Option<String>,

    /// 累计次数（仅 repeat_type="cumulative" 时有效）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_count: Option<u32>,

    /// 打卡日期列表（ISO 8601 日期字符串数组）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_in_dates: Option<Vec<String>>,

    /// 是否启用提醒
    #[serde(default)]
    pub reminder_enabled: bool,

    /// 标签列表
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// 工具参数
#[derive(Deserialize)]
pub struct TodoWriteArgs {
    /// 待办列表（merge=false 时替换全部，merge=true 时按 id 合并）
    pub todos: Vec<TodoItem>,
    /// 是否合并到现有列表，默认 false（替换）
    #[serde(default)]
    pub merge: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
#[error("todo_write error: {0}")]
pub struct TodoWriteError(String);

/// 工具结构体 - 持有共享的待办列表
///
/// `Arc<RwLock<..>>` 内部已是堆上的共享句柄，拷贝仅增加一个引用计数（原子递增）。
pub struct TodoWriteTool {
    todos: Arc<RwLock<Vec<TodoItem>>>,
    /// 可选：每会话 TodoStore 句柄，写入后同步持久化到当前会话
    store: Option<crate::todo_store::TodoStore>,
    /// 可选：当前会话 id 句柄（与 RigAgent 共享），持久化 key 用
    conv_id: Option<Arc<RwLock<Option<String>>>>,
    /// 可选：事件总线，写入后 emit todo-tree-updated 通知前端刷新
    event_bus: Option<Arc<effisuite_core::EventBus>>,
}

impl TodoWriteTool {
    /// 创建一个持有空待办列表的工具
    pub fn new() -> Self {
        Self {
            todos: Arc::new(RwLock::new(Vec::new())),
            store: None,
            conv_id: None,
            event_bus: None,
        }
    }

    /// 用已存在的共享句柄构造（多组件共享同一份列表时使用）
    pub fn with_state(todos: Arc<RwLock<Vec<TodoItem>>>) -> Self {
        Self {
            todos,
            store: None,
            conv_id: None,
            event_bus: None,
        }
    }

    /// 注入每会话持久化存储 + 会话 id 句柄 + 事件总线：
    /// 每次写入后同步持久化到当前会话，并通知前端刷新 todoTree 卡片。
    pub fn with_persistence(
        mut self,
        store: crate::todo_store::TodoStore,
        conv_id: Arc<RwLock<Option<String>>>,
        event_bus: Option<Arc<effisuite_core::EventBus>>,
    ) -> Self {
        self.store = Some(store);
        self.conv_id = Some(conv_id);
        self.event_bus = event_bus;
        self
    }

    /// 返回内部共享句柄的克隆，供 agent 把当前待办注入 prompt 上下文
    pub fn state(&self) -> Arc<RwLock<Vec<TodoItem>>> {
        Arc::clone(&self.todos)
    }
}

impl Default for TodoWriteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for TodoWriteTool {
    const NAME: &'static str = "todo_write";

    type Error = TodoWriteError;
    type Args = TodoWriteArgs;
    type Output = String;

  fn description(&self) -> String {
      "创建、更新并管理结构化待办列表（todoTree），跟踪复杂多步任务进度。\
       3 步以上复杂任务开始前建清单，每完成一步或转移焦点时更新；简单任务不用。\
       merge=false（默认）替换整个列表；merge=true 按 id 合并（已有更新、新 id 追加、未提及保留）。\
       parent_id：None=根任务，Some(id)=该 id 任务的子任务。\
       同一时间只允许一个 in_progress（多个时自动保留第一个）；列表自动按状态与优先级排序。\
       清单永久保存在本会话上下文，每轮对话注入。"
          .to_string()
  }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                      "description": "待办列表；merge=false 替换全部，merge=true 按 id 合并。id 空时自动生成递增数字 id",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "唯一 id；空时自动生成递增数字 id",
                            },
                            "content": {
                                "type": "string",
                                "description": "任务内容描述（不可为空）"
                            },
                            "priority": {
                                "type": "string",
                                "enum": ["high", "medium", "low"],
                                "description": "优先级"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"],
                                "description": "任务状态；同一时间只允许一个 in_progress"
                            },
                            "summary": {
                                "type": "string",
                                "description": "可选；仅 completed 时显示的完成总结",
                            },
                            "parent_id": {
                                "type": "string",
                                "description": "可选；父任务 id，缺省=根任务",
                            },
                            "urgency": {
                                "type": "string",
                                "enum": ["urgent", "not_urgent"],
                                "description": "紧急程度：urgent 或 not_urgent",
                            },
                            "due_date": {
                                "type": "string",
                                "description": "截止日期（ISO 8601）",
                            },
                            "due_time": {
                                "type": "string",
                                "description": "截止时间（HH:mm）",
                            },
                            "reminder_minutes_before": {
                                "type": "integer",
                                "description": "提前提醒分钟数"
                            },
                            "repeat_type": {
                                "type": "string",
                                "enum": ["once", "daily", "cumulative"],
                                "description": "重复类型：once/daily/cumulative",
                            },
                            "repeat_count": {
                                "type": "integer",
                                "description": "累计目标次数（仅 cumulative 时有效）"
                            },
                            "check_in_dates": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "可选；打卡日期列表（ISO 8601）",
                            },
                            "reminder_enabled": {
                                "type": "boolean",
                                "description": "是否启用提醒"
                            },
                            "tags": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "标签列表"
                            }
                        },
                        "required": ["id", "content", "priority", "status"]
                    }
                },
                "merge": {
                    "type": "boolean",
                      "description": "是否合并到现有列表，默认 false（替换）",
                    "default": false
                }
            },
            "required": ["todos"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 1. 输入校验（锁外做，避免锁内做无关计算）
        for t in &args.todos {
            if t.content.trim().is_empty() {
                return Err(TodoWriteError(
                    "待办项 content 不能为空".to_string(),
                ));
            }
        }
        let merge = args.merge.unwrap_or(false);

        // 2. 进入写锁：仅做数据合并 / 校正 / 排序 + 一次轻量格式化
        let out = {
            let mut guard = self.todos.write().await;
            let mut incoming = args.todos;
            // 用现有列表的最大数字 id 为空 id 补值，避免 merge 时 id 冲突
            fill_empty_ids(&guard, &mut incoming);

            let mut new_list = if merge {
                merge_into(&guard, incoming)
            } else {
                incoming
            };

            normalize_summaries(&mut new_list);
            auto_correct_in_progress(&mut new_list);
            sort_todos(&mut new_list);

            *guard = new_list;
            format_todos(&*guard)
        };

        // 3. 持久化到当前会话（每会话 todoTree）+ 通知前端刷新
        //    持久化失败仅记录日志，不影响工具主流程（agent 仍可继续任务）。
        self.persist().await;

        Ok(out)
    }
}

impl TodoWriteTool {
    /// 把当前内存清单持久化到当前会话的 todoTree 存储，并通知前端刷新。
    ///
    /// - 无 store 或当前无会话 id 时静默跳过（与旧的纯内存模式兼容）
    /// - 事件总线存在时 emit `todo-tree-updated`，前端据此刷新右栏 todoTree 卡片
    async fn persist(&self) {
        let Some(store) = &self.store else { return };
        let Some(conv_handle) = &self.conv_id else { return };
        let conv_id = match conv_handle.read().await.as_deref() {
            Some(id) => id.to_string(),
            None => return,
        };
        let snapshot = self.todos.read().await.clone();
        if let Err(e) = store.save(&conv_id, &snapshot).await {
            tracing::warn!(error = %e, conversation_id = %conv_id, "持久化 todoTree 失败");
            return;
        }
        if let Some(bus) = &self.event_bus {
            bus.publish(effisuite_core::BusEvent::TodoTreeUpdated {
                conversation_id: conv_id,
            });
        }
    }
}

// =========================================================
// 内部纯函数：合并 / 校正 / 排序 / 格式化
// =========================================================

/// 把 `incoming` 中 id 为空的项补上递增数字 id。
///
/// 起始值 = 现有列表与传入项中所有数字 id 的最大值 + 1，保证不与已有项冲突。
fn fill_empty_ids(existing: &[TodoItem], incoming: &mut [TodoItem]) {
    let mut max: u64 = 0;
    for t in existing {
        if let Ok(n) = t.id.parse::<u64>() {
            max = max.max(n);
        }
    }
    for t in incoming.iter() {
        if !t.id.is_empty() {
            if let Ok(n) = t.id.parse::<u64>() {
                max = max.max(n);
            }
        }
    }

    let mut next = max.saturating_add(1);
    for t in incoming.iter_mut() {
        if t.id.is_empty() {
            t.id = next.to_string();
            next = next.saturating_add(1);
        }
    }
}

/// merge=true：以 `existing` 为底，用 `incoming` 按 id 合并。
///
/// - 已存在 id → 整项替换为 incoming 的版本
/// - 新 id → 追加到末尾
/// - 未提及的现有 id → 原样保留
fn merge_into(existing: &[TodoItem], incoming: Vec<TodoItem>) -> Vec<TodoItem> {
    let mut result: Vec<TodoItem> = existing.to_vec();
    // 预留容量，减少可能的 reallocate
    result.reserve(incoming.len());

    for item in incoming {
        if let Some(idx) = result.iter().position(|t| t.id == item.id) {
            result[idx] = item;
        } else {
            result.push(item);
        }
    }
    result
}

/// 非 completed 项的 summary 无意义，统一清空。
fn normalize_summaries(list: &mut [TodoItem]) {
    for t in list.iter_mut() {
        if t.status != TodoStatus::Completed {
            t.summary = None;
        }
    }
}

/// 至多保留一个 in_progress：按列表顺序保留第一个，其余降级为 pending。
///
/// 在排序前调用，"第一个" 即写入顺序，语义为"最早开始的那一项"。
fn auto_correct_in_progress(list: &mut [TodoItem]) {
    let mut seen = false;
    for t in list.iter_mut() {
        if t.status == TodoStatus::InProgress {
            if seen {
                t.status = TodoStatus::Pending;
            } else {
                seen = true;
            }
        }
    }
}

/// 排序键的状态分量：in_progress(0) < pending(1) < completed(2)，
/// 升序后即 in_progress → pending → completed。
#[inline]
fn status_rank(s: TodoStatus) -> u8 {
    match s {
        TodoStatus::InProgress => 0,
        TodoStatus::Pending => 1,
        TodoStatus::Completed => 2,
    }
}

/// 按 (status_rank, priority) 稳定排序，同组内保留写入顺序。
fn sort_todos(list: &mut [TodoItem]) {
    list.sort_by_key(|t| (status_rank(t.status), t.priority));
}

/// 状态展示图标
#[inline]
fn status_icon(s: TodoStatus) -> &'static str {
    match s {
        TodoStatus::InProgress => "►",
        TodoStatus::Pending => "○",
        TodoStatus::Completed => "✓",
    }
}

/// 优先级中文标签
#[inline]
fn priority_label(p: TodoPriority) -> &'static str {
    match p {
        TodoPriority::High => "高",
        TodoPriority::Medium => "中",
        TodoPriority::Low => "低",
    }
}

/// 状态展示串（与 serde snake_case 一致）
#[inline]
fn status_label(s: TodoStatus) -> &'static str {
    match s {
        TodoStatus::InProgress => "in_progress",
        TodoStatus::Pending => "pending",
        TodoStatus::Completed => "completed",
    }
}

/// 把待办列表格式化为给 LLM 看的纯文本。
///
/// 单次遍历同时完成计数与正文构建，避免二次扫描。
fn format_todos(list: &[TodoItem]) -> String {
    if list.is_empty() {
        return "待办列表为空。".to_string();
    }

    let mut body = String::with_capacity(list.len() * 64);
    let mut n_in_progress: usize = 0;
    let mut n_pending: usize = 0;
    let mut n_completed: usize = 0;

    for t in list {
        match t.status {
            TodoStatus::InProgress => n_in_progress += 1,
            TodoStatus::Pending => n_pending += 1,
            TodoStatus::Completed => n_completed += 1,
        }

        body.push_str(status_icon(t.status));
        body.push_str(" [");
        body.push_str(priority_label(t.priority));
        body.push_str("] ");
        body.push_str(&t.id);
        body.push_str(". ");
        body.push_str(&t.content);
        body.push_str(" (");
        body.push_str(status_label(t.status));
        body.push(')');

        // summary 仅在 completed 时展示
        if t.status == TodoStatus::Completed {
            if let Some(s) = &t.summary {
                body.push_str(" - ");
                body.push_str(s);
            }
        }
        body.push('\n');
    }

    let total = list.len();
    let mut out =
        String::with_capacity(body.len() + 80);
    out.push_str(&format!(
        "待办列表（共 {total} 项，{n_in_progress} 进行中，{n_pending} 待办，{n_completed} 已完成）：\n"
    ));
    out.push_str(&body);
    // 去掉末尾多余换行
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个待办项（summary 默认 None，高级字段默认 None/false）
    fn todo(id: &str, content: &str, p: TodoPriority, s: TodoStatus) -> TodoItem {
        TodoItem {
            id: id.to_string(),
            content: content.to_string(),
            priority: p,
            status: s,
            summary: None,
            parent_id: None,
            urgency: None,
            due_date: None,
            due_time: None,
            reminder_minutes_before: None,
            repeat_type: None,
            repeat_count: None,
            check_in_dates: None,
            reminder_enabled: false,
            tags: None,
        }
    }

    /// 读取工具当前持有的列表快照（克隆出来便于断言）
    async fn snapshot(tool: &TodoWriteTool) -> Vec<TodoItem> {
        tool.todos.read().await.clone()
    }

    #[tokio::test]
    async fn create_new_list_replace_mode() {
        let tool = TodoWriteTool::new();
        let args = TodoWriteArgs {
            todos: vec![
                todo("1", "实现用户登录 API", TodoPriority::High, TodoStatus::InProgress),
                todo("2", "编写集成测试", TodoPriority::High, TodoStatus::Pending),
                todo("", "更新文档", TodoPriority::Medium, TodoStatus::Pending),
            ],
            merge: Some(false),
        };
        let out = tool.call(args).await.unwrap();

        // 空 id 被自动补为 3（现有最大数字 id=1? 实际 max(1,2)=2 → 3）
        assert!(out.contains("共 3 项"));
        assert!(out.contains("1 进行中"));
        assert!(out.contains("2 待办"));
        assert!(out.contains("3. 更新文档"));

        let list = snapshot(&tool).await;
        assert_eq!(list.len(), 3);
        assert_eq!(list[2].id, "3"); // 空 id → 自动生成 3
    }

    #[tokio::test]
    async fn merge_updates_existing_item_status() {
        let tool = TodoWriteTool::new();
        // 先建列表
        tool.call(TodoWriteArgs {
            todos: vec![
                todo("1", "任务A", TodoPriority::High, TodoStatus::Pending),
                todo("2", "任务B", TodoPriority::Medium, TodoStatus::InProgress),
            ],
            merge: Some(false),
        })
        .await
        .unwrap();

        // merge：把任务A 改为 completed，并加总结；任务B 未提及应保留
        let out = tool
            .call(TodoWriteArgs {
                todos: vec![TodoItem {
                    id: "1".to_string(),
                    content: "任务A".to_string(),
                    priority: TodoPriority::High,
                    status: TodoStatus::Completed,
                    summary: Some("已完成A".to_string()),
                    parent_id: None,
                    urgency: None,
                    due_date: None,
                    due_time: None,
                    reminder_minutes_before: None,
                    repeat_type: None,
                    repeat_count: None,
                    check_in_dates: None,
                    reminder_enabled: false,
                    tags: None,
                }],
                merge: Some(true),
            })
            .await
            .unwrap();

        let list = snapshot(&tool).await;
        assert_eq!(list.len(), 2, "未提及的 id=2 应保留");
        // 排序后：in_progress(任务B) 在前，completed(任务A) 在后
        assert_eq!(list[0].id, "2");
        assert_eq!(list[0].status, TodoStatus::InProgress);
        assert_eq!(list[1].id, "1");
        assert_eq!(list[1].status, TodoStatus::Completed);
        assert_eq!(list[1].summary.as_deref(), Some("已完成A"));
        // 输出应含 summary
        assert!(out.contains("- 已完成A"));
    }

    #[tokio::test]
    async fn auto_correct_multiple_in_progress() {
        let tool = TodoWriteTool::new();
        tool.call(TodoWriteArgs {
            todos: vec![
                todo("1", "第一个进行中", TodoPriority::High, TodoStatus::InProgress),
                todo("2", "第二个进行中", TodoPriority::Medium, TodoStatus::InProgress),
                todo("3", "待办", TodoPriority::Low, TodoStatus::Pending),
            ],
            merge: Some(false),
        })
        .await
        .unwrap();

        let list = snapshot(&tool).await;
        // 只保留第一个 in_progress，第二个降级为 pending
        let in_progress: Vec<&TodoItem> = list.iter().filter(|t| t.status == TodoStatus::InProgress).collect();
        assert_eq!(in_progress.len(), 1);
        assert_eq!(in_progress[0].id, "1");
        // id=2 应被降级为 pending
        let item2 = list.iter().find(|t| t.id == "2").unwrap();
        assert_eq!(item2.status, TodoStatus::Pending);
    }

    #[tokio::test]
    async fn sort_verification() {
        let tool = TodoWriteTool::new();
        // 故意打乱：completed低 / pending中 / in_progress高 / pending高
        tool.call(TodoWriteArgs {
            todos: vec![
                todo("1", "已完成低优", TodoPriority::Low, TodoStatus::Completed),
                todo("2", "待办中优", TodoPriority::Medium, TodoStatus::Pending),
                todo("3", "进行中高优", TodoPriority::High, TodoStatus::InProgress),
                todo("4", "待办高优", TodoPriority::High, TodoStatus::Pending),
            ],
            merge: Some(false),
        })
        .await
        .unwrap();

        let list = snapshot(&tool).await;
        // 期望顺序：in_progress(3) → pending high(4) → pending medium(2) → completed(1)
        let ids: Vec<&str> = list.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["3", "4", "2", "1"]);
    }

    #[tokio::test]
    async fn empty_list_handling_replace_clears() {
        let tool = TodoWriteTool::new();
        // 先建非空列表
        tool.call(TodoWriteArgs {
            todos: vec![todo("1", "任务", TodoPriority::High, TodoStatus::Pending)],
            merge: Some(false),
        })
        .await
        .unwrap();
        assert_eq!(snapshot(&tool).await.len(), 1);

        // merge=false + 空列表 → 清空
        let out = tool
            .call(TodoWriteArgs {
                todos: vec![],
                merge: Some(false),
            })
            .await
            .unwrap();
        assert!(out.contains("待办列表为空"));
        assert!(snapshot(&tool).await.is_empty());
    }

    #[tokio::test]
    async fn empty_list_merge_keeps_existing() {
        let tool = TodoWriteTool::new();
        tool.call(TodoWriteArgs {
            todos: vec![todo("1", "任务", TodoPriority::High, TodoStatus::Pending)],
            merge: Some(false),
        })
        .await
        .unwrap();

        // merge=true + 空列表 → 不变
        let out = tool
            .call(TodoWriteArgs {
                todos: vec![],
                merge: Some(true),
            })
            .await
            .unwrap();
        assert!(out.contains("共 1 项"));
        assert_eq!(snapshot(&tool).await.len(), 1);
    }

    #[tokio::test]
    async fn summary_only_shown_when_completed() {
        let tool = TodoWriteTool::new();
        // pending 项带 summary → 应被清空且不展示
        let out = tool
            .call(TodoWriteArgs {
                todos: vec![TodoItem {
                    id: "1".to_string(),
                    content: "未完成项".to_string(),
                    priority: TodoPriority::High,
                    status: TodoStatus::Pending,
                    summary: Some("不该出现的总结".to_string()),
                    parent_id: None,
                    urgency: None,
                    due_date: None,
                    due_time: None,
                    reminder_minutes_before: None,
                    repeat_type: None,
                    repeat_count: None,
                    check_in_dates: None,
                    reminder_enabled: false,
                    tags: None,
                }],
                merge: Some(false),
            })
            .await
            .unwrap();
        assert!(!out.contains("不该出现的总结"));
        let list = snapshot(&tool).await;
        assert_eq!(list[0].summary, None);

        // 改为 completed 并带 summary → 应展示
        let out = tool
            .call(TodoWriteArgs {
                todos: vec![TodoItem {
                    id: "1".to_string(),
                    content: "未完成项".to_string(),
                    priority: TodoPriority::High,
                    status: TodoStatus::Completed,
                    summary: Some("已完成的总结".to_string()),
                    parent_id: None,
                    urgency: None,
                    due_date: None,
                    due_time: None,
                    reminder_minutes_before: None,
                    repeat_type: None,
                    repeat_count: None,
                    check_in_dates: None,
                    reminder_enabled: false,
                    tags: None,
                }],
                merge: Some(true),
            })
            .await
            .unwrap();
        assert!(out.contains("- 已完成的总结"));
    }

    #[tokio::test]
    async fn empty_id_auto_generates_distinct_sequential() {
        let tool = TodoWriteTool::new();
        tool.call(TodoWriteArgs {
            todos: vec![
                todo("", "空id项1", TodoPriority::High, TodoStatus::Pending),
                todo("", "空id项2", TodoPriority::High, TodoStatus::Pending),
                todo("5", "显式id5", TodoPriority::High, TodoStatus::Pending),
            ],
            merge: Some(false),
        })
        .await
        .unwrap();

        let list = snapshot(&tool).await;
        // 现有最大数字 id=5，空 id 应补为 6、7
        let ids: Vec<String> = list.iter().map(|t| t.id.clone()).collect();
        assert!(ids.contains(&"5".to_string()));
        assert!(ids.contains(&"6".to_string()));
        assert!(ids.contains(&"7".to_string()));
    }

    #[tokio::test]
    async fn merge_keeps_unmentioned_ids() {
        let tool = TodoWriteTool::new();
        tool.call(TodoWriteArgs {
            todos: vec![
                todo("1", "任务A", TodoPriority::High, TodoStatus::Pending),
                todo("2", "任务B", TodoPriority::High, TodoStatus::Pending),
                todo("3", "任务C", TodoPriority::High, TodoStatus::Pending),
            ],
            merge: Some(false),
        })
        .await
        .unwrap();

        // 只 merge 更新 id=2，id=1/3 应原样保留
        tool.call(TodoWriteArgs {
            todos: vec![todo("2", "任务B-改", TodoPriority::Medium, TodoStatus::InProgress)],
            merge: Some(true),
        })
        .await
        .unwrap();

        let list = snapshot(&tool).await;
        assert_eq!(list.len(), 3);
        let ids: Vec<&str> = list.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&"1"));
        assert!(ids.contains(&"3"));
        // id=2 字段被更新
        let item2 = list.iter().find(|t| t.id == "2").unwrap();
        assert_eq!(item2.content, "任务B-改");
        assert_eq!(item2.priority, TodoPriority::Medium);
        assert_eq!(item2.status, TodoStatus::InProgress);
    }

    #[tokio::test]
    async fn rejects_empty_content() {
        let tool = TodoWriteTool::new();
        let res = tool
            .call(TodoWriteArgs {
                todos: vec![todo("1", "   ", TodoPriority::High, TodoStatus::Pending)],
                merge: Some(false),
            })
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn default_merge_is_replace() {
        let tool = TodoWriteTool::new();
        tool.call(TodoWriteArgs {
            todos: vec![todo("1", "旧任务", TodoPriority::High, TodoStatus::Pending)],
            merge: None, // 不传 → 默认 false = 替换
        })
        .await
        .unwrap();

        let out = tool
            .call(TodoWriteArgs {
                todos: vec![todo("2", "新任务", TodoPriority::Medium, TodoStatus::InProgress)],
                merge: None,
            })
            .await
            .unwrap();
        // 替换后只剩 1 项
        assert!(out.contains("共 1 项"));
        assert!(out.contains("新任务"));
        assert!(!out.contains("旧任务"));
    }

    #[tokio::test]
    async fn shared_state_visible_across_handles() {
        let tool = TodoWriteTool::new();
        let shared = tool.state();
        tool.call(TodoWriteArgs {
            todos: vec![todo("1", "共享任务", TodoPriority::High, TodoStatus::InProgress)],
            merge: Some(false),
        })
        .await
        .unwrap();

        // 通过另一个句柄读到同一份数据
        let other = TodoWriteTool::with_state(Arc::clone(&shared));
        let list = other.todos.read().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].content, "共享任务");
    }
}
