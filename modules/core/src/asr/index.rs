//! ASR 摘要 RAG 索引：接入 MemoryIndex 做摘要检索的适配层
//!
//! 将 ASR 记录的 `标题 + 摘要 + 转写前200字` 索引到 [`MemoryIndex`] 中，
//! 复用其 BM25 词法检索能力。不修改 MemoryIndex 原有实现，通过 namespace
//! （conversation_id = "asr"）隔离 ASR 条目与其他历史记忆条目。
//!
//! # 适配设计
//!
//! - MemoryIndex 的 `add` 方法以 `(conversation_id, message_id)` 作为幂等键。
//!   ASR 条目使用 `conversation_id = "asr"`、`message_id = record.id`。
//! - MemoryIndex 无 `remove` API，通过 tombstone 集合（`removed`）在 search 时
//!   过滤已删除的条目。
//! - `created_at` 转为毫秒时间戳存入 `MemoryEntry.timestamp`，search 时按时间
//!   范围过滤。
//! - search 时仅保留 `conversation_id == "asr"` 的命中，排除其他历史记忆条目。

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::memory::MemoryIndex;
use crate::models::{Message, Role};
use crate::Result;

use super::types::AsrRecord;

/// ASR 命名空间：用作 MemoryIndex 的 conversation_id，隔离 ASR 条目
const ASR_NAMESPACE: &str = "asr";

/// 转写前缀长度：索引到 MemoryIndex 的 transcript 截取字符数
const TRANSCRIPT_PREFIX_CHARS: usize = 200;

/// ASR 摘要检索命中结果
///
/// 字段按大小降序：String（24B）→ DateTime（8B）→ f32（4B）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrSummaryHit {
    pub record_id: String,
    pub title: String,
    pub summary_snippet: String,
    pub created_at: DateTime<Utc>,
    /// 相关性得分（BM25），越大越相关
    pub score: f32,
}

/// ASR 摘要 RAG 索引：在 MemoryIndex 之上构建的适配层
///
/// 由于 MemoryIndex 无 remove API，使用 tombstone 集合（`removed`）过滤
/// 已删除的条目。`upsert_summary` 对已存在的条目为幂等 no-op（MemoryIndex
/// 的 `add` 方法对相同 `(conv_id, msg_id)` 跳过）。
pub struct AsrSummaryIndex {
    /// 共享的 MemoryIndex 实例（与其他 RAG 索引共用）
    memory: Arc<RwLock<MemoryIndex>>,
    /// ASR 条目在 MemoryIndex 中的命名空间（conversation_id）
    namespace: String,
    /// 已删除 record id 集合（tombstone），search 时过滤
    removed: Arc<RwLock<HashSet<String>>>,
}

impl AsrSummaryIndex {
    /// 创建 ASR 摘要索引
    pub fn new(memory: Arc<RwLock<MemoryIndex>>) -> Self {
        Self {
            memory,
            namespace: ASR_NAMESPACE.to_string(),
            removed: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// 将 ASR 记录的摘要索引到 MemoryIndex
    ///
    /// 索引内容 = `标题\n摘要\n转写前200字`，以 `record.id` 作为 message_id，
    /// namespace 作为 conversation_id。`created_at` 转为毫秒时间戳存入
    /// `MemoryEntry.timestamp`，供 search 时按时间范围过滤。
    ///
    /// 注意：MemoryIndex.add 对相同 `(conv_id, msg_id)` 幂等跳过，
    /// 因此对已索引的 record 重复调用不会更新内容。
    pub async fn upsert_summary(&self, record: &AsrRecord) -> Result<()> {
        let content = build_index_content(record);
        let timestamp = record.created_at.timestamp_millis().max(0) as u64;
        let msg = Message::new(record.id.clone(), Role::User, content, timestamp);

        // Clone MemoryIndex（廉价 Arc clone）避免跨 await 持有外层 RwLock
        let memory = self.memory.read().await.clone();
        memory.add(&self.namespace, msg).await;

        // 从 tombstone 集合中移除（如果之前被删除过又重新索引）
        self.removed.write().await.remove(&record.id);
        Ok(())
    }

    /// 从索引中移除指定 record（tombstone 标记）
    ///
    /// MemoryIndex 无 remove API，此处通过 tombstone 集合在 search 时过滤。
    pub async fn remove(&self, id: &str) -> Result<()> {
        self.removed.write().await.insert(id.to_string());
        Ok(())
    }

    /// 搜索 ASR 摘要：先通过 MemoryIndex BM25 召回，再按 namespace、
    /// tombstone、时间范围过滤
    pub async fn search(
        &self,
        keyword: &str,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<AsrSummaryHit>> {
        let memory = self.memory.read().await.clone();
        // 多取候选以补偿 namespace 与 tombstone 过滤导致的数量缩减
        let fetch_limit = limit.saturating_mul(2).max(20);
        let hits = memory.search_lexical(keyword, fetch_limit, None).await;

        let start_ts = start.map(|d| d.timestamp_millis().max(0) as u64);
        let end_ts = end.map(|d| d.timestamp_millis().max(0) as u64);

        let removed = self.removed.read().await;

        let mut results: Vec<AsrSummaryHit> = hits
            .into_iter()
            .filter(|h| h.conversation_id == self.namespace)
            .filter(|h| !removed.contains(&h.message_id))
            .filter(|h| start_ts.map_or(true, |s| h.timestamp >= s))
            .filter(|h| end_ts.map_or(true, |e| h.timestamp <= e))
            .map(|h| {
                let (title, summary_snippet) = split_title_and_snippet(&h.snippet);
                AsrSummaryHit {
                    record_id: h.message_id,
                    title,
                    summary_snippet,
                    created_at: ts_to_datetime(h.timestamp),
                    score: h.score,
                }
            })
            .collect();
        results.truncate(limit);
        Ok(results)
    }
}

/// 构建索引内容：`标题\n摘要\n转写前200字`
///
/// 用 `\n` 分隔以便 search 时从 snippet 的首行提取标题。
fn build_index_content(record: &AsrRecord) -> String {
    let summary = record.summary.as_deref().unwrap_or("");
    let transcript_prefix: String = record
        .transcript
        .chars()
        .take(TRANSCRIPT_PREFIX_CHARS)
        .collect();
    let capacity = record.title.len() + summary.len() + transcript_prefix.len() + 2;
    let mut content = String::with_capacity(capacity);
    content.push_str(&record.title);
    content.push('\n');
    content.push_str(summary);
    content.push('\n');
    content.push_str(&transcript_prefix);
    content
}

/// 从 snippet 中分割标题与摘要：标题为第一行，其余为摘要片段
fn split_title_and_snippet(snippet: &str) -> (String, String) {
    match snippet.find('\n') {
        Some(pos) => {
            let title = snippet[..pos].to_string();
            let summary_snippet = snippet[pos + 1..].trim_start().to_string();
            (title, summary_snippet)
        }
        None => (snippet.to_string(), String::new()),
    }
}

/// 毫秒时间戳转 DateTime<Utc>，无效时回退到当前时间
#[inline]
fn ts_to_datetime(ts: u64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ts as i64)
        .single()
        .unwrap_or_else(Utc::now)
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::types::{AsrRecord, AsrSource, AsrStatus};
    use crate::config::AsrProvider;

    fn make_record(id: &str, title: &str, summary: &str, transcript: &str) -> AsrRecord {
        let now = Utc::now();
        AsrRecord {
            id: id.to_string(),
            audio_path: format!("{}.wav", id),
            transcript: transcript.to_string(),
            title: title.to_string(),
            language: "zh-CN".to_string(),
            summary: Some(summary.to_string()),
            error_message: None,
            tags: vec!["test".to_string()],
            created_at: now,
            updated_at: now,
            duration_ms: 1000,
            sample_rate: 16000,
            provider: AsrProvider::VolcEngine,
            status: AsrStatus::Completed,
            source: AsrSource::Upload,
        }
    }

    #[tokio::test]
    async fn upsert_and_search() {
        let memory = Arc::new(RwLock::new(MemoryIndex::new()));
        let index = AsrSummaryIndex::new(memory);

        let record = make_record(
            "r1",
            "语音会议",
            "讨论了 Rust 异步编程",
            "今天我们讨论了 Rust 异步编程的最佳实践",
        );
        index.upsert_summary(&record).await.unwrap();

        let hits = index.search("Rust 异步", None, None, 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record_id, "r1");
        assert_eq!(hits[0].title, "语音会议");
    }

    #[tokio::test]
    async fn search_returns_empty_for_no_match() {
        let memory = Arc::new(RwLock::new(MemoryIndex::new()));
        let index = AsrSummaryIndex::new(memory);

        let record = make_record("r1", "标题", "摘要", "转写内容");
        index.upsert_summary(&record).await.unwrap();

        let hits = index.search("完全不相关的关键词", None, None, 10).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn remove_filters_from_search() {
        let memory = Arc::new(RwLock::new(MemoryIndex::new()));
        let index = AsrSummaryIndex::new(memory);

        let record = make_record("r1", "测试", "内容", "测试内容");
        index.upsert_summary(&record).await.unwrap();

        // 删除前能搜到
        let hits = index.search("测试", None, None, 10).await.unwrap();
        assert_eq!(hits.len(), 1);

        // 删除后搜不到
        index.remove("r1").await.unwrap();
        let hits = index.search("测试", None, None, 10).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn search_does_not_return_non_asr_entries() {
        let memory = Arc::new(RwLock::new(MemoryIndex::new()));
        let index = AsrSummaryIndex::new(memory.clone());

        // 添加 ASR 条目
        let record = make_record("r1", "ASR记录", "ASR摘要", "ASR转写");
        index.upsert_summary(&record).await.unwrap();

        // 直接通过 MemoryIndex 添加非 ASR 条目（不同 namespace）
        let memory_clone = memory.read().await.clone();
        memory_clone
            .add(
                "chat_history",
                Message::new("m1", Role::User, "ASR 关键词", 1),
            )
            .await;

        // 搜索 "ASR" 应只返回 ASR namespace 的条目
        let hits = index.search("ASR", None, None, 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record_id, "r1");
    }

    #[tokio::test]
    async fn upsert_after_remove_re_indexes() {
        let memory = Arc::new(RwLock::new(MemoryIndex::new()));
        let index = AsrSummaryIndex::new(memory);

        let record = make_record("r1", "重新索引", "内容", "重新索引的转写");
        index.upsert_summary(&record).await.unwrap();
        index.remove("r1").await.unwrap();

        // 重新 upsert（注意：MemoryIndex.add 对相同 key 幂等跳过，
        // 但 tombstone 会被清除，所以 search 能再次返回）
        index.upsert_summary(&record).await.unwrap();
        let hits = index.search("重新索引", None, None, 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record_id, "r1");
    }

    #[test]
    fn split_title_and_snippet_works() {
        let (title, snippet) = split_title_and_snippet("标题\n摘要内容");
        assert_eq!(title, "标题");
        assert_eq!(snippet, "摘要内容");

        let (title, snippet) = split_title_and_snippet("只有标题");
        assert_eq!(title, "只有标题");
        assert!(snippet.is_empty());
    }

    #[test]
    fn build_index_content_format() {
        let record = make_record("r1", "标题", "摘要", "转写前缀");
        let content = build_index_content(&record);
        assert_eq!(content, "标题\n摘要\n转写前缀");
    }

    #[test]
    fn build_index_content_empty_summary() {
        let mut record = make_record("r1", "标题", "摘要", "转写");
        record.summary = None;
        let content = build_index_content(&record);
        // summary 为 None 时，as_deref().unwrap_or("") 返回空字符串
        assert_eq!(content, "标题\n\n转写");
    }
}
