//! search_history 工具：在历史对话中按关键词检索
//!
//! 实现 EffiSuite 的 RAG 索引式调用：当用户提问涉及历史话题时，
//! LLM 可主动调用此工具检索相关历史消息，作为回答上下文。
//!
//! 索引算法：简单词频（TF）匹配
//! - 把 query 复用 `effisuite_core::tokenize`（CJK 单字+bigram 拆分），得到关键词集合
//! - 对每条历史消息计算关键词命中数（不区分大小写）
//! - 按命中数降序取前 N 条返回
//!
//! 避免引入向量数据库依赖，零外部成本即可获得基础 RAG 能力。
//! 后续可替换为基于嵌入向量的检索而不改 trait。

use effisuite_core::{make_snippet, Message, Role, tokenize, SNIPPET_MAX_CHARS};
use rig_core::tool::Tool;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 工具参数
#[derive(Deserialize)]
pub struct SearchHistoryArgs {
    /// 搜索查询关键词
    pub query: String,
    /// 返回最多多少条结果，默认 5
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    5
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("search history error: {0}")]
pub struct SearchHistoryError(String);

/// 历史检索工具
///
/// 持有当前 conversation 的历史快照（Arc<RwLock<Vec<Message>>>）。
/// 每次调用时短暂持读锁遍历，锁内仅做读取，无 IO。
pub struct SearchHistoryTool {
    history: Arc<RwLock<Vec<Message>>>,
}

impl SearchHistoryTool {
    pub fn new(history: Arc<RwLock<Vec<Message>>>) -> Self {
        Self { history }
    }
}

impl Tool for SearchHistoryTool {
    const NAME: &'static str = "search_history";

    type Error = SearchHistoryError;
    type Args = SearchHistoryArgs;
    type Output = String;

    fn description(&self) -> String {
        "在当前会话的历史消息中按关键词检索相关内容。当用户提到之前讨论过的话题、\
         或需要回顾历史信息时调用。返回相关消息的摘要。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜索关键词，多个词以空格分隔"
                },
                "limit": {
                    "type": "integer",
                    "description": "最多返回的结果条数，默认 5",
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let history = self.history.read().await;
        let keywords = tokenize(&args.query);
        if keywords.is_empty() {
            return Ok("无有效关键词，未检索到历史。".to_string());
        }

        // 计算每条消息的相关性得分（关键词命中数），过滤 0 分
        let limit = if args.limit == 0 { 5 } else { args.limit };
        let mut scored: Vec<(usize, &Message)> = history
            .iter()
            .filter_map(|m| {
                let score = score_message(m, &keywords);
                if score > 0 { Some((score, m)) } else { None }
            })
            .collect();

        // 按得分降序，得分相同则保持原顺序（稳定排序）
        scored.sort_by_key(|b| std::cmp::Reverse(b.0));
        let results: Vec<&Message> = scored.iter().take(limit).map(|(_, m)| *m).collect();

        if results.is_empty() {
            return Ok(format!("未找到与「{}」相关的历史消息。", args.query));
        }

        // 序列化结果摘要（复用 core 的 make_snippet，UTF-8 边界安全截断）
        let mut out = String::with_capacity(results.len() * (SNIPPET_MAX_CHARS + 48));
        out.push_str(&format!("找到 {} 条相关历史消息：\n", results.len()));
        for (i, m) in results.iter().enumerate() {
            let role = match m.role {
                Role::User => "用户",
                Role::Assistant => "助手",
                Role::System => "系统",
            };
            let preview = make_snippet(&m.content, SNIPPET_MAX_CHARS);
            out.push_str(&format!("{}. [{}] {}\n", i + 1, role, preview));
        }
        Ok(out)
    }
}

/// 计算单条消息的关键词命中数（不区分大小写）
fn score_message(msg: &Message, keywords: &[String]) -> usize {
    if msg.content.is_empty() {
        return 0;
    }
    let lower = msg.content.to_lowercase();
    keywords.iter().filter(|kw| lower.contains(kw.as_str())).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_counts_keyword_hits() {
        let msg = Message::new("m1", Role::User, "Rust 是一门系统编程语言，rust 好", 0);
        let keywords = vec!["rust".to_string(), "python".to_string()];
        let score = score_message(&msg, &keywords);
        assert_eq!(score, 1, "应只计 1 个独立关键词命中（contains 不计次数）");
    }

    #[test]
    fn tokenize_cjk_short_query_scores_hit() {
        // 回归：中文短查询经 tokenize 后应能命中长文本
        let msg = Message::new("m1", Role::User, "我们讨论过异步编程的优缺点", 0);
        let keywords = tokenize("异步");
        assert!(keywords.contains(&"异步".to_string()));
        assert!(score_message(&msg, &keywords) > 0);
    }

    #[tokio::test]
    async fn search_returns_relevant_messages() {
        let history = vec![
            Message::new("m1", Role::User, "今天我们聊 Rust 编程", 1),
            Message::new("m2", Role::Assistant, "Rust 是系统语言", 2),
            Message::new("m3", Role::User, "明天天气如何", 3),
        ];
        let tool = SearchHistoryTool::new(Arc::new(RwLock::new(history)));
        let args = SearchHistoryArgs {
            query: "Rust".to_string(),
            limit: 5,
        };
        let result = tool.call(args).await.unwrap();
        assert!(result.contains("2 条"));
        assert!(result.contains("Rust"));
        assert!(!result.contains("天气"));
    }

    #[tokio::test]
    async fn search_returns_no_results_when_unmatched() {
        let history = vec![Message::new("m1", Role::User, "hello", 1)];
        let tool = SearchHistoryTool::new(Arc::new(RwLock::new(history)));
        let args = SearchHistoryArgs {
            query: "rust".to_string(),
            limit: 5,
        };
        let result = tool.call(args).await.unwrap();
        assert!(result.contains("未找到"));
    }
}
