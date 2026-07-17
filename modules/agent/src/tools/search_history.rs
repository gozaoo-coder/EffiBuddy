//! search_history 工具：在历史对话中按关键词检索
//!
//! 实现 EffiSuite 的 RAG 索引式调用：当用户提问涉及历史话题时，
//! LLM 可主动调用此工具检索相关历史消息，作为回答上下文。
//!
//! 索引算法：简单词频（TF）匹配
//! - 把 query 按空白与标点分词，得到关键词集合
//! - 对每条历史消息计算关键词命中数（不区分大小写）
//! - 按命中数降序取前 N 条返回
//!
//! 避免引入向量数据库依赖，零外部成本即可获得基础 RAG 能力。
//! 后续可替换为基于嵌入向量的检索而不改 trait。

use effisuite_core::{Message, Role};
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
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        let results: Vec<&Message> = scored.iter().take(limit).map(|(_, m)| *m).collect();

        if results.is_empty() {
            return Ok(format!("未找到与「{}」相关的历史消息。", args.query));
        }

        // 序列化结果摘要
        let mut out = String::with_capacity(results.len() * 64);
        out.push_str(&format!("找到 {} 条相关历史消息：\n", results.len()));
        for (i, m) in results.iter().enumerate() {
            let role = match m.role {
                Role::User => "用户",
                Role::Assistant => "助手",
                Role::System => "系统",
            };
            // 截取前 100 字符避免上下文爆炸
            let preview = if m.content.len() > 100 {
                format!("{}…", &m.content[..m.content.ceil_char_boundary(100)])
            } else {
                m.content.clone()
            };
            out.push_str(&format!("{}. [{}] {}\n", i + 1, role, preview));
        }
        Ok(out)
    }
}

/// 简单分词：按空白与常见标点切分，转小写
fn tokenize(query: &str) -> Vec<String> {
    query
        .split(|c: char| c.is_whitespace() || "，。、,.;:!?".contains(c))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
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
    fn tokenize_handles_chinese_punctuation() {
        let tokens = tokenize("你好，世界!rust 编程");
        assert_eq!(tokens, vec!["你好", "世界", "rust", "编程"]);
    }

    #[test]
    fn score_counts_keyword_hits() {
        let msg = Message::new("m1", Role::User, "Rust 是一门系统编程语言，rust 好", 0);
        let keywords = vec!["rust".to_string(), "python".to_string()];
        let score = score_message(&msg, &keywords);
        assert_eq!(score, 1, "应只计 1 个独立关键词命中（contains 不计次数）");
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
