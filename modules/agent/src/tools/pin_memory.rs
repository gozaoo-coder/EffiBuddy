//! 永久记忆工具集：让 LLM 在用户明确要求时管理永久上下文
//!
//! 三个独立工具（rig `Tool` trait），共享同一份 `PinnedMemoryStore`：
//!
//! - [`PinMemoryTool`]：把一条内容永久加入上下文（用户说"请记住..."时调用）
//! - [`ListPinnedMemoriesTool`]：列出当前已永久记忆的内容（让 LLM 自查避免重复）
//! - [`DeletePinnedMemoryTool`]：按 id 删除某条永久记忆（用户说"忘掉..."时调用）
//!
//! # 与 RAG `search_memory` 的区别
//!
//! - `search_memory`：跨会话语义检索，**按相关性**注入；可能不命中
//! - `pin_memory`：**永久**注入到每轮 prompt 的 `[永久记忆]` 段，不依赖检索
//!
//! # 设计要点（对齐 user_rules）
//!
//! - 工具本身无状态，所有数据在共享 `Arc<PinnedMemoryStore>` 中
//! - `now_ms` 在工具内计算，避免 Tauri 层注入时间源
//! - 返回纯文本，流式友好；错误以 `"Error: ..."` 前缀标记（与 SearchMemoryTool 一致）

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use effisuite_core::{PinnedMemorySource, PinnedMemoryStore};
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

/// 当前 Unix 毫秒时间戳；失败回退为 0
#[inline]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// =========================================================
// PinMemoryTool：新增一条永久记忆
// =========================================================

/// 工具参数
#[derive(Deserialize)]
pub struct PinMemoryArgs {
    /// 要永久记住的内容（必填）
    pub content: String,
    /// 可选分类标签，如 "preference" / "fact" / "instruction"
    #[serde(default)]
    pub category: Option<String>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("pin memory error: {0}")]
pub struct PinMemoryError(String);

/// 永久记忆新增工具
///
/// 持有：
/// - `store`：共享存储句柄（与 `RigAgent`、Tauri 命令层共享同一份）
/// - `current_conversation_id`：当前会话 id，用于审计回溯
pub struct PinMemoryTool {
    store: Arc<PinnedMemoryStore>,
    current_conversation_id: Arc<RwLock<Option<String>>>,
}

impl PinMemoryTool {
    pub fn new(
        store: Arc<PinnedMemoryStore>,
        current_conversation_id: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            store,
            current_conversation_id,
        }
    }
}

impl Tool for PinMemoryTool {
    const NAME: &'static str = "pin_memory";

    type Error = PinMemoryError;
    type Args = PinMemoryArgs;
    type Output = String;

    fn description(&self) -> String {
        "把一条内容永久加入上下文记忆。当用户明确要求「请记住」「以后都要记得」「记住这一点」\
         等时调用。被记忆的内容会在之后所有对话中自动注入到上下文，无需检索。\
         不要用来记忆临时信息或大段对话，仅用于用户希望长期生效的偏好/事实/指令。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "要永久记住的内容，简洁清晰的一句话或一段说明"
                },
                "category": {
                    "type": "string",
                    "description": "可选分类标签，如 preference(偏好)/fact(事实)/instruction(指令)",
                    "default": null
                }
            },
            "required": ["content"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let content = args.content.trim();
        if content.is_empty() {
            return Err(PinMemoryError("content 不能为空".to_string()));
        }
        // 限制单条长度，避免恶意/误用撑爆上下文
        const MAX_CONTENT_CHARS: usize = 2000;
        if content.chars().count() > MAX_CONTENT_CHARS {
            return Err(PinMemoryError(format!(
                "content 过长（>{MAX_CONTENT_CHARS} 字符），请精简后再记忆"
            )));
        }

        let source_conv = self.current_conversation_id.read().await.clone();
        let id = self
            .store
            .add_simple(
                content,
                args.category,
                PinnedMemorySource::UserRequest,
                source_conv,
                now_ms(),
            )
            .await
            .map_err(|e| PinMemoryError(e.to_string()))?;

        Ok(format!(
            "已永久记住（id={}）。这条内容会在之后所有对话中自动注入上下文。",
            short_id(&id)
        ))
    }
}

// =========================================================
// ListPinnedMemoriesTool：列出全部永久记忆
// =========================================================

/// 无参数（rig 要求 Args 类型，用空 struct + 自定义 deserialize）
#[derive(Deserialize, Default)]
pub struct ListPinnedMemoriesArgs {}

/// 列表工具错误
#[derive(Debug, thiserror::Error)]
#[error("list pinned memories error: {0}")]
pub struct ListPinnedMemoriesError(String);

pub struct ListPinnedMemoriesTool {
    store: Arc<PinnedMemoryStore>,
}

impl ListPinnedMemoriesTool {
    pub fn new(store: Arc<PinnedMemoryStore>) -> Self {
        Self { store }
    }
}

impl Tool for ListPinnedMemoriesTool {
    const NAME: &'static str = "list_pinned_memories";

    type Error = ListPinnedMemoriesError;
    type Args = ListPinnedMemoriesArgs;
    type Output = String;

    fn description(&self) -> String {
        "列出当前所有已永久记住的内容。在用户询问「你记住了什么」或准备添加新记忆前\
         检查是否重复时调用。".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let list = self.store.list().await;
        if list.is_empty() {
            return Ok("当前没有任何永久记忆。".to_string());
        }
        let mut out = String::with_capacity(list.len() * 96);
        out.push_str(&format!("当前共 {} 条永久记忆：\n", list.len()));
        for (i, m) in list.iter().enumerate() {
            let cat = m
                .category
                .as_deref()
                .map(|c| format!("[{}] ", c))
                .unwrap_or_default();
            out.push_str(&format!("{}. (id={}) {}{}\n", i + 1, short_id(&m.id), cat, m.content));
        }
        Ok(out)
    }
}

// =========================================================
// DeletePinnedMemoryTool：按 id 删除
// =========================================================

#[derive(Deserialize)]
pub struct DeletePinnedMemoryArgs {
    /// 要删除的永久记忆 id（完整或前 8 字符均可匹配）
    pub id: String,
}

#[derive(Debug, thiserror::Error)]
#[error("delete pinned memory error: {0}")]
pub struct DeletePinnedMemoryError(String);

pub struct DeletePinnedMemoryTool {
    store: Arc<PinnedMemoryStore>,
}

impl DeletePinnedMemoryTool {
    pub fn new(store: Arc<PinnedMemoryStore>) -> Self {
        Self { store }
    }
}

impl Tool for DeletePinnedMemoryTool {
    const NAME: &'static str = "delete_pinned_memory";

    type Error = DeletePinnedMemoryError;
    type Args = DeletePinnedMemoryArgs;
    type Output = String;

    fn description(&self) -> String {
        "按 id 删除一条永久记忆。当用户说「忘掉这条」「不要再记这个」「删除刚才记住的」\
         等时调用。id 可从 list_pinned_memories 工具的结果中获取，支持前缀匹配。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "要删除的永久记忆 id（完整或前 8 字符前缀）"
                }
            },
            "required": ["id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = args.id.trim();
        if target.is_empty() {
            return Err(DeletePinnedMemoryError("id 不能为空".to_string()));
        }

        // 先尝试精确匹配，再尝试前缀匹配
        let list = self.store.list().await;
        let matched: Vec<&effisuite_core::PinnedMemory> = list
            .iter()
            .filter(|m| m.id == target || m.id.starts_with(target))
            .collect();

        if matched.is_empty() {
            return Ok(format!("未找到 id 包含「{}」的永久记忆。", target));
        }
        if matched.len() > 1 {
            return Ok(format!(
                "id「{}」匹配到 {} 条永久记忆，请提供更长的前缀以唯一定位。",
                target,
                matched.len()
            ));
        }
        let id_to_delete = matched[0].id.clone();
        self.store
            .delete(&id_to_delete)
            .await
            .map_err(|e| DeletePinnedMemoryError(e.to_string()))?;
        Ok(format!("已删除永久记忆（id={}）。", short_id(&id_to_delete)))
    }
}

/// 截断 id 用于显示（取前 8 字符，UTF-8 边界安全）
#[inline]
fn short_id(id: &str) -> String {
    if id.len() <= 8 {
        id.to_string()
    } else {
        id[..id.ceil_char_boundary(8)].to_string()
    }
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "effisuite-pin-tool-test-{}.json",
            uuid::Uuid::new_v4()
        ))
    }

    #[tokio::test]
    async fn pin_memory_adds_and_returns_id() {
        let store = Arc::new(PinnedMemoryStore::new(tmp_path()).unwrap());
        let cur = Arc::new(RwLock::new(Some("conv-1".to_string())));
        let tool = PinMemoryTool::new(Arc::clone(&store), cur);

        let args = PinMemoryArgs {
            content: "我喜欢用 Rust 写代码".to_string(),
            category: Some("preference".to_string()),
        };
        let result = tool.call(args).await.unwrap();
        assert!(result.contains("已永久记住"));
        assert_eq!(store.list().await.len(), 1);
    }

    #[tokio::test]
    async fn pin_memory_rejects_empty_content() {
        let store = Arc::new(PinnedMemoryStore::new(tmp_path()).unwrap());
        let cur = Arc::new(RwLock::new(None));
        let tool = PinMemoryTool::new(store, cur);
        let args = PinMemoryArgs {
            content: "   ".to_string(),
            category: None,
        };
        assert!(tool.call(args).await.is_err());
    }

    #[tokio::test]
    async fn list_pinned_memories_formats_output() {
        let store = Arc::new(PinnedMemoryStore::new(tmp_path()).unwrap());
        store
            .add_simple(
                "事实A",
                Some("fact".into()),
                PinnedMemorySource::Manual,
                None,
                1,
            )
            .await
            .unwrap();
        store
            .add_simple("偏好B", None, PinnedMemorySource::Manual, None, 2)
            .await
            .unwrap();

        let tool = ListPinnedMemoriesTool::new(Arc::clone(&store));
        let out = tool.call(ListPinnedMemoriesArgs {}).await.unwrap();
        assert!(out.contains("2 条永久记忆"));
        assert!(out.contains("事实A"));
        assert!(out.contains("偏好B"));
        assert!(out.contains("[fact]"));
    }

    #[tokio::test]
    async fn list_pinned_memories_empty_message() {
        let store = Arc::new(PinnedMemoryStore::new(tmp_path()).unwrap());
        let tool = ListPinnedMemoriesTool::new(store);
        let out = tool.call(ListPinnedMemoriesArgs {}).await.unwrap();
        assert!(out.contains("没有任何永久记忆"));
    }

    #[tokio::test]
    async fn delete_pinned_memory_by_prefix() {
        let store = Arc::new(PinnedMemoryStore::new(tmp_path()).unwrap());
        let id = store
            .add_simple(
                "test content",
                None,
                PinnedMemorySource::Manual,
                None,
                1,
            )
            .await
            .unwrap();
        assert_eq!(store.list().await.len(), 1);

        let tool = DeletePinnedMemoryTool::new(Arc::clone(&store));
        // 用前 8 字符前缀匹配
        let prefix: String = id.chars().take(8).collect();
        let out = tool
            .call(DeletePinnedMemoryArgs { id: prefix })
            .await
            .unwrap();
        assert!(out.contains("已删除"));
        assert!(store.list().await.is_empty());
    }

    #[tokio::test]
    async fn delete_pinned_memory_not_found_message() {
        let store = Arc::new(PinnedMemoryStore::new(tmp_path()).unwrap());
        let tool = DeletePinnedMemoryTool::new(store);
        let out = tool
            .call(DeletePinnedMemoryArgs {
                id: "nonexistent".to_string(),
            })
            .await
            .unwrap();
        assert!(out.contains("未找到"));
    }
}
