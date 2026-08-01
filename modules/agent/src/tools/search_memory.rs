//! search_memory 工具：跨会话历史记忆检索
//!
//! 与 `SearchHistoryTool`（仅检索当前会话）不同，本工具通过 [`MemoryIndex`]
//! 检索**所有**历史对话记忆，是"模型主动搜索全部历史对话记忆"的核心入口。
//!
//! # 三种检索模式
//!
//! - `lexical`：词法 BM25（默认回退，无需网络）
//! - `vector`：向量 embedding 余弦相似度（需配置 OpenAI 兼容 provider）
//! - `hybrid`：RRF 融合（默认推荐，自动降级为 lexical 当向量路不可用）
//!
//! # 自动排除当前会话
//!
//! 工具持有 `current_conversation_id` 句柄（由 `RigAgent` 在每次调用前更新），
//! 检索时自动排除当前会话的条目，避免与已注入的对话历史重复。
//!
//! # 设计要点（对齐 user_rules）
//!
//! - 工具本身无状态，所有索引数据在 `Arc<MemoryIndex>` 中
//! - 检索锁内仅读，零 IO；向量计算的 HTTP 在锁外
//! - 返回字符串以流式友好的纯文本格式，避免序列化开销

use std::sync::Arc;

use effisuite_core::{MemoryIndex, SearchMode, SNIPPET_MAX_CHARS};
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

/// 工具参数
#[derive(Deserialize)]
pub struct SearchMemoryArgs {
    /// 搜索查询字符串
    pub query: String,
    /// 返回最多多少条结果，默认 5
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// 检索模式：lexical / vector / hybrid（默认 hybrid）
    #[serde(default)]
    pub mode: Option<String>,
}

fn default_limit() -> usize {
    5
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("search memory error: {0}")]
pub struct SearchMemoryError(String);

/// 跨会话历史记忆检索工具
///
/// 持有：
/// - `MemoryIndex` 共享句柄（与 `RigAgent` 共享同一份索引）
/// - `current_conversation_id` 句柄（由 agent 在每次对话前更新，检索时排除）
pub struct SearchMemoryTool {
    index: Arc<MemoryIndex>,
    current_conversation_id: Arc<RwLock<Option<String>>>,
}

impl SearchMemoryTool {
    pub fn new(
        index: Arc<MemoryIndex>,
        current_conversation_id: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            index,
            current_conversation_id,
        }
    }
}

/// 解析 mode 字符串到 SearchMode 枚举
fn parse_mode(s: &Option<String>) -> SearchMode {
    match s.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("lexical") | Some("bm25") | Some("keyword") => SearchMode::Lexical,
        Some("vector") | Some("embedding") | Some("semantic") => SearchMode::Vector,
        // 默认 hybrid（含 None / "hybrid" / 未知值）
        _ => SearchMode::Hybrid,
    }
}

impl Tool for SearchMemoryTool {
    const NAME: &'static str = "search_memory";

    type Error = SearchMemoryError;
    type Args = SearchMemoryArgs;
    type Output = String;

    fn description(&self) -> String {
        "跨所有历史会话检索相关记忆（不仅限当前对话）。当用户提到过去讨论过的话题、\
         需要回顾跨会话信息、或当前对话历史不足以回答时调用。\
         支持三种模式：lexical（词法匹配）、vector（语义向量）、hybrid（混合，默认）。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜索查询字符串，可以是关键词或完整问题"
                },
                "limit": {
                    "type": "integer",
                    "description": "最多返回的结果条数，默认 5",
                    "default": 5
                },
                "mode": {
                    "type": "string",
                    "enum": ["lexical", "vector", "hybrid"],
                    "description": "检索模式：lexical=词法BM25，vector=语义向量，hybrid=混合（默认）",
                    "default": "hybrid"
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let query = args.query.trim();
        if query.is_empty() {
            return Ok("查询为空，未检索到记忆。".to_string());
        }

        let limit = if args.limit == 0 { 5 } else { args.limit };
        let mode = parse_mode(&args.mode);

        // 读出当前会话 id（短暂读锁）
        let exclude_conv = self.current_conversation_id.read().await.clone();

        let hits = self
            .index
            .search(query, limit, mode, exclude_conv.as_deref())
            .await;

        if hits.is_empty() {
            return Ok(format!("未找到与「{}」相关的历史记忆。", query));
        }

        // 格式化结果（预分配：每条 ≈ SNIPPET_MAX_CHARS + 序号/会话id/角色等开销 ≈ 48B）
        let mut out = String::with_capacity(hits.len() * (SNIPPET_MAX_CHARS + 48));
        out.push_str(&format!("找到 {} 条相关历史记忆：\n", hits.len()));
        for (i, hit) in hits.iter().enumerate() {
            let role = match hit.role {
                effisuite_core::Role::User => "用户",
                effisuite_core::Role::Assistant => "助手",
                effisuite_core::Role::System => "系统",
            };
            out.push_str(&format!(
                "{}. [会话{}] [{}] {}\n",
                i + 1,
                short_id(&hit.conversation_id),
                role,
                hit.snippet
            ));
        }
        Ok(out)
    }
}

/// 截断会话 id 用于显示（取前 8 字符）
#[inline]
fn short_id(id: &str) -> &str {
    if id.len() <= 8 {
        id
    } else {
        &id[..id.ceil_char_boundary(8)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use effisuite_core::{Message, Role};

    #[tokio::test]
    async fn search_memory_returns_relevant_across_conversations() {
        let idx = std::sync::Arc::new(MemoryIndex::new());
        idx.add(
            "conv-a",
            Message::new("m1", Role::User, "我们讨论过 Rust 的异步编程", 1),
        )
        .await;
        idx.add(
            "conv-b",
            Message::new("m2", Role::Assistant, "Rust 的 async/await 基于 Future", 2),
        )
        .await;
        idx.add(
            "conv-c",
            Message::new("m3", Role::User, "今天天气真好", 3),
        )
        .await;

        let cur = Arc::new(RwLock::new(Some("conv-a".to_string())));
        let tool = SearchMemoryTool::new(idx, cur);
        let args = SearchMemoryArgs {
            query: "Rust 异步".to_string(),
            limit: 5,
            mode: Some("lexical".to_string()),
        };
        let result = tool.call(args).await.unwrap();
        // 应排除当前会话 conv-a，返回 conv-b 的相关消息
        assert!(result.contains("1 条"));
        assert!(result.contains("conv-b"));
        assert!(!result.contains("conv-a"));
        assert!(!result.contains("天气"));
    }

    #[tokio::test]
    async fn search_memory_handles_empty_query() {
        let idx = std::sync::Arc::new(MemoryIndex::new());
        let cur = Arc::new(RwLock::new(None));
        let tool = SearchMemoryTool::new(idx, cur);
        let args = SearchMemoryArgs {
            query: "   ".to_string(),
            limit: 5,
            mode: None,
        };
        let result = tool.call(args).await.unwrap();
        assert!(result.contains("查询为空"));
    }

    #[tokio::test]
    async fn search_memory_no_results_returns_message() {
        let idx = std::sync::Arc::new(MemoryIndex::new());
        idx.add(
            "c1",
            Message::new("m1", Role::User, "hello world", 1),
        )
        .await;
        let cur = Arc::new(RwLock::new(None));
        let tool = SearchMemoryTool::new(idx, cur);
        let args = SearchMemoryArgs {
            query: "nonexistent".to_string(),
            limit: 5,
            mode: None,
        };
        let result = tool.call(args).await.unwrap();
        assert!(result.contains("未找到"));
    }

    #[test]
    fn parse_mode_handles_various_inputs() {
        assert!(matches!(parse_mode(&None), SearchMode::Hybrid));
        assert!(matches!(
            parse_mode(&Some("lexical".into())),
            SearchMode::Lexical
        ));
        assert!(matches!(
            parse_mode(&Some("VECTOR".into())),
            SearchMode::Vector
        ));
        assert!(matches!(
            parse_mode(&Some("bm25".into())),
            SearchMode::Lexical
        ));
        assert!(matches!(
            parse_mode(&Some("semantic".into())),
            SearchMode::Vector
        ));
        assert!(matches!(
            parse_mode(&Some("unknown".into())),
            SearchMode::Hybrid
        ));
    }
}
