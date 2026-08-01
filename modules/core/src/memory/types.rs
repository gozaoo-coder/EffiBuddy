//! RAG 记忆增强：类型定义
//!
//! 集中存放 [`MemoryEntry`] / [`MemoryHit`] / [`MemoryStats`] / [`SearchMode`]
//! 等公开类型，以及索引内部状态 [`IndexState`] 与 BM25/RRF 默认参数常量。
//! 将数据结构与算法实现解耦，便于独立演进。

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::Role;

/// 默认 BM25 参数 k1（词频饱和参数）
pub(super) const DEFAULT_K1: f64 = 1.2;
/// 默认 BM25 参数 b（文档长度归一化参数）
pub(super) const DEFAULT_B: f64 = 0.75;
/// RRF 默认 k 值（rank fusion 平滑参数）
pub(super) const DEFAULT_RRF_K: u32 = 60;

/// 检索命中片段的最大字符数
///
/// 词法（BM25）与向量两路检索构造 [`MemoryHit::snippet`] 时统一使用，
/// 同时被 `search_history` 工具复用，避免各处重复硬编码。
///
/// 取值权衡：100 字符对中文不足两句，难以承载有效上下文；240 字符约 2~3 句，
/// 既能给模型足够语义线索，又不会在 5 条结果下过度占用上下文窗口（≈1.2K 字符）。
/// 对 ASR 结构化内容（标题\n摘要\n转写前缀）亦能覆盖标题 + 大部分摘要。
pub const SNIPPET_MAX_CHARS: usize = 240;

/// 嵌入向量维度（仅用于一致性校验，不强制；不同 provider 维度不同）
/// 这里不写死维度，由 provider 决定。
/// 异步嵌入向量提供者 trait
///
/// core 模块不依赖任何 HTTP/rig 实现，仅定义接口。agent 模块提供
/// `OpenAIEmbeddingProvider` 实现，使用已配置的 OpenAI 兼容 `/embeddings` 接口。
///
/// 实现方需保证：
/// - 输入多条文本时按顺序返回对应向量
/// - 向量维度对同一 provider/model 稳定
/// - 错误以 `CoreError::Agent` 上抛
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// 批量计算嵌入向量。`texts` 顺序与返回 `Vec<Vec<f32>>` 顺序一一对应。
    async fn embed(&self, texts: &[&str]) -> crate::Result<Vec<Vec<f32>>>;
}

/// 一条被索引的历史消息条目
///
/// 字段按大小降序排列以最小化 padding：
/// `conversation_id`/`message_id`/`content`（String, 24B）
/// → `tokens`（Vec, 24B）
/// → `embedding`（Option<Vec>, 24B）
/// → `timestamp`（u64, 8B）
/// → `role`（Role, 1B）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub conversation_id: String,
    pub message_id: String,
    pub content: String,
    /// 预分词结果（小写化），避免每次查询重复切词
    pub tokens: Vec<String>,
    /// 嵌入向量；None 表示尚未计算，向量检索时跳过
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
    pub timestamp: u64,
    pub role: Role,
}

impl MemoryEntry {
    /// 文档长度（token 数），BM25 用
    #[inline]
    pub(super) fn dl(&self) -> usize {
        self.tokens.len()
    }

    /// 稳定键：`<conv_id>:<msg_id>`，用于 embedding 持久化缓存匹配
    #[inline]
    pub fn cache_key(&self) -> String {
        format!("{}:{}", self.conversation_id, self.message_id)
    }
}

/// 检索命中结果
///
/// 字段按大小降序：String(24) → f32(4) → u64(8) 顺序调整后为
/// String(24) → u64(8) → f32(4) → bool(1)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryHit {
    pub conversation_id: String,
    pub message_id: String,
    pub snippet: String,
    pub timestamp: u64,
    /// 相关性得分，越大越相关；不同检索方式分数含义不同（BM25 / cosine / RRF）
    pub score: f32,
    pub role: Role,
}

/// 检索模式
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    /// 词法 BM25
    Lexical,
    /// 向量 embedding 余弦相似度
    Vector,
    /// 混合 RRF（默认推荐）
    #[default]
    Hybrid,
}

/// 索引统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    pub total_entries: usize,
    pub unique_tokens: usize,
    pub embedded_entries: usize,
    pub avg_doc_len: f64,
}

/// 索引内部可变状态（被 RwLock 包裹）
pub(super) struct IndexState {
    pub(super) entries: Vec<MemoryEntry>,
    /// 倒排表：token → [(entry_idx, tf)]
    pub(super) inverted: HashMap<String, Vec<(usize, u32)>>,
    /// 文档频率：token → 出现该 token 的文档数
    pub(super) df: HashMap<String, u32>,
    /// 文档总数
    pub(super) n_docs: u64,
    /// 文档总长度（token 数之和）
    pub(super) total_dl: u64,
    /// BM25 参数
    pub(super) k1: f64,
    pub(super) b: f64,
    /// 嵌入向量提供者（可热替换）
    pub(super) embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

/// 生成片段：取前 `max_chars` 字符，在 UTF-8 字符边界处安全截断并补省略号。
///
/// 词法/向量两路检索与 `search_history` 工具均通过本函数构造片段，
/// 集中在此避免重复实现。默认长度见 [`SNIPPET_MAX_CHARS`]。
#[inline]
pub fn make_snippet(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    let boundary = content.ceil_char_boundary(max_chars);
    let mut s = String::with_capacity(boundary + 3);
    s.push_str(&content[..boundary]);
    s.push('…');
    s
}
