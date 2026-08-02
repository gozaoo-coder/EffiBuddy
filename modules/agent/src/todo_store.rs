//! 每会话 TodoTree 存储：把任务清单按会话持久化到磁盘，并常驻注入到该会话的上下文。
//!
//! # 设计
//!
//! - 存储与 [`CompressionStore`] 同构：每会话一个 JSON 文件，`RwLock<()>` 仅做并发同步。
//! - 数据模型复用 `todo_write` 工具的 [`TodoItem`]（扁平列表 + `parent_id` 指针），
//!   由 [`build_todo_tree`] 还原为树、由 [`format_todo_tree`] 格式化为带缩进的树形文本。
//! - 树形语义：`parent_id == None` 为根任务；`parent_id == Some(id)` 为该 id 任务的子任务。
//!   同层任务按写入顺序并列（平行 / 顺序任务），子任务总是排在父任务之后、作为从属。
//! - prompt 注入：`RigAgent::build_context_parts` 读取当前会话的任务清单，注入
//!   `[当前任务清单]` 段，让 LLM 每轮都看得到任务树，永久安排在本会话上下文里。
//!
//! # 为什么放在 agent crate 而不是 core
//!
//! 数据模型 [`TodoItem`] 本就定义在 agent crate（`tools::todo_write`），Tauri 命令层
//! 也依赖 agent crate；放这里可避免把类型搬进 core 造成的大范围引用改动，且无循环依赖。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use effisuite_core::{CoreError, Result};
use tokio::sync::RwLock;

use crate::tools::{TodoItem, TodoStatus};

/// `/` 在文件名中的转义序列（与 [`effisuite_core::PluginStore`] 一致）
const SLUG_SEP: &str = "__";

/// 树节点：把扁平 [`TodoItem`] 列表还原为树后的节点
#[derive(Debug, Clone)]
pub struct TodoNode {
    /// 对应 TodoItem 的字段
    pub id: String,
    pub content: String,
    pub priority: crate::tools::TodoPriority,
    pub status: TodoStatus,
    pub summary: Option<String>,
    /// 子任务
    pub children: Vec<TodoNode>,
}

impl TodoNode {
    #[inline]
    pub fn status_rank(&self) -> u8 {
        match self.status {
            TodoStatus::InProgress => 0,
            TodoStatus::Pending => 1,
            TodoStatus::Completed => 2,
        }
    }
}

/// 每会话 TodoTree 存储，线程安全可廉价 clone（内部 `Arc<RwLock<()>>`，同构于 CompressionStore）
#[derive(Clone)]
pub struct TodoStore {
    root: PathBuf,
    _lock: Arc<RwLock<()>>,
}

impl TodoStore {
    /// 创建存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            _lock: Arc::new(RwLock::new(())),
        })
    }

    /// 将 conversation id 转换为安全文件名：`/` → `__`
    #[inline]
    fn id_to_file_name(id: &str) -> String {
        id.replace('/', SLUG_SEP)
    }

    /// 状态文件路径：`<root>/<safe_id>.json`
    fn path_for(&self, id: &str) -> PathBuf {
        let safe = Self::id_to_file_name(id);
        let file_name = Path::new(&safe)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(&safe));
        self.root.join(file_name).with_extension("json")
    }

    /// 加载指定会话的任务清单，不存在返回 None
    pub async fn load(&self, conversation_id: &str) -> Result<Option<Vec<TodoItem>>> {
        let path = self.path_for(conversation_id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        let items: Vec<TodoItem> = serde_json::from_slice(&bytes).map_err(CoreError::Serde)?;
        Ok(Some(items))
    }

    /// 保存（或覆盖）指定会话的任务清单
    pub async fn save(&self, conversation_id: &str, items: &[TodoItem]) -> Result<()> {
        let path = self.path_for(conversation_id);
        let bytes = serde_json::to_vec(items).map_err(CoreError::Serde)?;
        let _guard = self._lock.write().await;
        tokio::fs::write(&path, bytes).await.map_err(CoreError::Io)?;
        Ok(())
    }

    /// 删除指定会话的任务清单，不存在返回 Ok(())
    pub async fn delete(&self, conversation_id: &str) -> Result<()> {
        let path = self.path_for(conversation_id);
        if path.exists() {
            tokio::fs::remove_file(&path).await.map_err(CoreError::Io)?;
        }
        Ok(())
    }
}

/// 把扁平 `TodoItem` 列表还原为树。
///
/// - `parent_id == None`（或指向不存在的 id / 指向自己）视为根任务，按写入顺序并列
/// - `parent_id == Some(id)` 挂到对应节点的 children（不存在时降级为根）
/// - 每个节点的 children 保持写入顺序（平行 / 顺序任务并列）
pub fn build_todo_tree(items: &[TodoItem]) -> Vec<TodoNode> {
    let nodes: Vec<TodoNode> = items
        .iter()
        .map(|t| TodoNode {
            id: t.id.clone(),
            content: t.content.clone(),
            priority: t.priority,
            status: t.status,
            summary: t.summary.clone(),
            children: Vec::new(),
        })
        .collect();

    // id → index 索引
    let mut index: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        index.insert(n.id.as_str(), i);
    }

    // 根节点：parent_id 为空、指向不存在、或指向自身
    let roots: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(i, t)| {
            match &t.parent_id {
                None => true,
                Some(p) => index.get(p.as_str()).map(|&pi| pi == *i).unwrap_or(true),
            }
        })
        .map(|(i, _)| i)
        .collect();

    // 从根出发递归挂载子树
    fn fill(
        idx: usize,
        nodes: &[TodoNode],
        items: &[TodoItem],
        index: &std::collections::HashMap<&str, usize>,
        visited: &mut std::collections::HashSet<usize>,
    ) -> TodoNode {
        let mut node = nodes[idx].clone();
        // 防止环：只挂载未访问过的子节点
        for (ci, child) in items.iter().enumerate() {
            if let Some(p) = &child.parent_id {
                if let Some(&pi) = index.get(p.as_str()) {
                    if pi == idx && !visited.contains(&ci) {
                        visited.insert(ci);
                        node.children.push(fill(ci, nodes, items, index, visited));
                    }
                }
            }
        }
        node
    }

    let mut visited: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for &r in &roots {
        visited.insert(r);
    }
    let mut tree = Vec::with_capacity(roots.len());
    for r in roots {
        tree.push(fill(r, &nodes, items, &index, &mut visited));
    }
    tree
}

/// 把任务清单格式化为给 LLM / UI 看的树形文本。
///
/// 缩进规则：
/// - 根任务：`- [状态] [优先级] id. content`
/// - 子任务：两级缩进（两个空格一层），用 `-` 表示从属
///
/// 顺序规则（对齐 todo_write 的排序语义）：
/// 同层按 `in_progress → pending → completed`、同级内保持写入顺序。
/// 为保持用户"平行/顺序并列、子任务从属"的直觉，树内不重排写入顺序，
/// 仅在文本里标注状态；排序交由 UI / todo_write 工具层处理。
pub fn format_todo_tree(items: &[TodoItem]) -> String {
    if items.is_empty() {
        return "[当前任务清单]\n（无任务）".to_string();
    }
    let roots = build_todo_tree(items);
    let mut out = String::with_capacity(items.len() * 96);
    out.push_str("[当前任务清单]\n");
    for r in roots {
        append_node(&mut out, &r, 0);
    }
    out
}

fn append_node(out: &mut String, node: &TodoNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let icon = match node.status {
        TodoStatus::InProgress => "►",
        TodoStatus::Pending => "○",
        TodoStatus::Completed => "✓",
    };
    let status = match node.status {
        TodoStatus::InProgress => "in_progress",
        TodoStatus::Pending => "pending",
        TodoStatus::Completed => "completed",
    };
    out.push_str(&indent);
    out.push_str(icon);
    out.push(' ');
    out.push_str(&node.content);
    out.push_str(" (");
    out.push_str(status);
    out.push(')');
    if node.status == TodoStatus::Completed {
        if let Some(s) = &node.summary {
            out.push_str(" - ");
            out.push_str(s);
        }
    }
    out.push('\n');
    for ch in &node.children {
        append_node(out, ch, depth + 1);
    }
}

/// 树节点的计数统计（UI 用）：总数 / 进行中 / 待办 / 已完成
pub fn todo_tree_stats(items: &[TodoItem]) -> (usize, usize, usize, usize) {
    let mut total = 0;
    let mut in_progress = 0;
    let mut pending = 0;
    let mut completed = 0;
    for t in items {
        total += 1;
        match t.status {
            TodoStatus::InProgress => in_progress += 1,
            TodoStatus::Pending => pending += 1,
            TodoStatus::Completed => completed += 1,
        }
    }
    (total, in_progress, pending, completed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{TodoItem, TodoPriority};

    fn item(id: &str, content: &str, parent: Option<&str>, status: TodoStatus) -> TodoItem {
        TodoItem {
            id: id.to_string(),
            content: content.to_string(),
            priority: TodoPriority::Medium,
            status,
            summary: None,
            parent_id: parent.map(|p| p.to_string()),
        }
    }

    #[test]
    fn build_tree_hierarchy() {
        let items = vec![
            item("1", "根任务A", None, TodoStatus::InProgress),
            item("2", "根任务B", None, TodoStatus::Pending),
            item("1a", "A的子任务1", Some("1"), TodoStatus::Pending),
            item("1b", "A的子任务2", Some("1"), TodoStatus::Completed),
            item("1a1", "子任务的子任务", Some("1a"), TodoStatus::Pending),
        ];
        let tree = build_todo_tree(&items);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].id, "1");
        assert_eq!(tree[0].children.len(), 2);
        assert_eq!(tree[0].children[0].id, "1a");
        assert_eq!(tree[0].children[0].children.len(), 1);
        assert_eq!(tree[0].children[0].children[0].id, "1a1");
    }

    #[test]
    fn format_tree_indents_children() {
        let items = vec![
            item("1", "根", None, TodoStatus::InProgress),
            item("1a", "子", Some("1"), TodoStatus::Pending),
        ];
        let text = format_todo_tree(&items);
        assert!(text.contains("[当前任务清单]"));
        assert!(text.contains("► 根 (in_progress)"));
        assert!(text.contains("  ○ 子 (pending)"));
    }

    #[tokio::test]
    async fn store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("effisuite-todo-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = TodoStore::new(&dir).unwrap();
        assert!(store.load("conv1").await.unwrap().is_none());

        let items = vec![item("1", "任务", None, TodoStatus::Pending)];
        store.save("conv1", &items).await.unwrap();
        let loaded = store.load("conv1").await.unwrap().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].content, "任务");

        store.delete("conv1").await.unwrap();
        assert!(store.load("conv1").await.unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
