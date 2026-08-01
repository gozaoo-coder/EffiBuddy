//! RAG 记忆增强：双路检索（词法 BM25 + 向量 embedding）
//!
//! 提供跨会话的历史记忆检索能力，是 EffiSuite "记忆增强 + history search 增强"
//! 的核心数据结构。被 `RigAgent` 用于自动注入相关历史上文，也被
//! `SearchMemoryTool` 暴露给 LLM 主动检索全部历史对话记忆。
//!
//! # 检索算法
//!
//! - **词法（BM25）**：基于 Okapi BM25 + IDF 加权，优于简单 `contains` 计数。
//!   利用倒排表 `HashMap<token, Vec<(entry_idx, tf)>>` 仅对含查询词的文档打分，
//!   避免全量遍历。
//! - **向量（embedding 余弦相似度）**：通过 [`EmbeddingProvider`] 异步获取查询向量,
//!   与每条已嵌入条目比对。Provider 由上层（agent 模块）注入，core 不依赖网络。
//! - **混合（RRF, Reciprocal Rank Fusion）**：默认模式。对两路结果按
//!   `score = Σ 1/(k + rank)` 合并，无需归一化原始分数，鲁棒性高。
//!
//! # 并发与性能（对齐 user_rules）
//!
//! - 索引状态包在 `RwLock` 中：查询并发读，写入短暂持写锁且锁内零 IO。
//! - `MemoryEntry` 字段按大小降序：String(24) → Vec(24) → Option<Vec<f32>>(24)
//!   → u64(8) → Role(1)，最小化 padding。
//! - 查询路径全部用迭代器适配器，无显式 `for i in 0..len` 索引循环。
//! - 结果 Vec 用 `with_capacity` 预分配。
//! - embedding 持久化缓存由外部 `import_embeddings`/`export_embeddings` 注入,
//!   避免在 core 引入文件 IO。
//!
//! # 模块组织
//!
//! 为避免"上帝文件"，本模块按职责拆分为多个子文件：
//! - [`types`]：公开类型与内部状态 [`types::IndexState`]、BM25/RRF 默认参数
//! - [`tokenize`]：CJK 单字+bigram 分词器（通过 `crate::tokenize` 复用）
//! - [`bm25`]：BM25 检索算法、IDF/TF 计算、倒排索引构建
//! - [`vector`]：向量余弦相似度检索
//! - [`rrf`]：RRF 融合算法

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::{CoreError, Message, Role};

mod bm25;
mod rrf;
mod tokenize;
mod types;
mod vector;

// 对外公开 API（签名与可见性保持不变）
pub use types::{EmbeddingProvider, MemoryEntry, MemoryHit, MemoryStats, SearchMode};
// `tokenize` 被 storage.rs / skill_index.rs 等通过 `crate::tokenize` 复用
pub use tokenize::tokenize;

// 内部辅助：将子模块中的算法函数引入本模块作用域，供 MemoryIndex 调用
use bm25::{bm25_search, push_entry};
use rrf::rrf_fuse;
use types::{IndexState, DEFAULT_B, DEFAULT_K1, DEFAULT_RRF_K};
use vector::cosine_search;
// `cosine_sim` / `vec_norm` 仅供单元测试直接调用，门控以避免非测试构建的 unused 警告
#[cfg(test)]
use vector::{cosine_sim, vec_norm};

/// 跨会话历史记忆索引，线程安全可廉价 clone（内部 RwLock + Arc）
///
/// 典型用法：
/// - 启动时 `rebuild_from_messages` 从 `ConversationStore` 全量重建
/// - 新消息到达时 `add` 增量更新
/// - `RigAgent` 在 `build_contextual_prompt` 中调用 `search_hybrid` 自动注入上文
/// - `SearchMemoryTool` 调用 `search` 供 LLM 主动检索
#[derive(Clone)]
pub struct MemoryIndex {
    state: Arc<RwLock<IndexState>>,
}

impl MemoryIndex {
    /// 创建空索引
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(IndexState {
                entries: Vec::new(),
                inverted: HashMap::new(),
                df: HashMap::new(),
                n_docs: 0,
                total_dl: 0,
                k1: DEFAULT_K1,
                b: DEFAULT_B,
                embedding_provider: None,
            })),
        }
    }

    /// 设置嵌入向量提供者（agent 模块在构造 OpenAIEmbeddingProvider 后注入）。
    /// 设置后 `search_vector` / `search_hybrid` 才会启用向量路。
    pub async fn set_embedding_provider(&self, provider: Arc<dyn EmbeddingProvider>) {
        let mut s = self.state.write().await;
        s.embedding_provider = Some(provider);
    }

    /// 清除嵌入向量提供者（如配置变为 mock 后端时）
    pub async fn clear_embedding_provider(&self) {
        let mut s = self.state.write().await;
        s.embedding_provider = None;
    }

    /// 从已加载的消息列表全量重建索引（清除旧数据）。
    /// 锁内仅做内存操作，零 IO。
    pub async fn rebuild_from_messages(
        &self,
        messages: impl IntoIterator<Item = (String, Message)>,
    ) {
        let mut s = self.state.write().await;
        s.entries.clear();
        s.inverted.clear();
        s.df.clear();
        s.n_docs = 0;
        s.total_dl = 0;

        // 预估容量：messages 数量未知，用默认 16 起步
        s.entries = Vec::with_capacity(16);
        for (conv_id, msg) in messages {
            // 跳过空内容与系统消息（系统消息多为技能 preamble，无检索价值）
            if msg.content.trim().is_empty() || matches!(msg.role, Role::System) {
                continue;
            }
            let tokens = tokenize(&msg.content);
            let entry = MemoryEntry {
                conversation_id: conv_id,
                message_id: msg.id,
                content: msg.content,
                tokens,
                embedding: None,
                timestamp: msg.timestamp,
                role: msg.role,
            };
            push_entry(&mut s, entry);
        }
    }

    /// 增量追加一条消息。若 `(conv_id, msg_id)` 已存在则跳过（幂等）。
    /// 锁内仅做内存操作。
    pub async fn add(&self, conv_id: &str, msg: Message) {
        // 跳过空内容与系统消息
        if msg.content.trim().is_empty() || matches!(msg.role, Role::System) {
            return;
        }
        let mut s = self.state.write().await;
        // 幂等检查：相同 (conv_id, msg_id) 不重复索引
        let exists = s
            .entries
            .iter()
            .any(|e| e.conversation_id == conv_id && e.message_id == msg.id);
        if exists {
            return;
        }
        let tokens = tokenize(&msg.content);
        let entry = MemoryEntry {
            conversation_id: conv_id.to_string(),
            message_id: msg.id,
            content: msg.content,
            tokens,
            embedding: None,
            timestamp: msg.timestamp,
            role: msg.role,
        };
        push_entry(&mut s, entry);
    }

    /// 为指定 `(conv_id, msg_id)` 设置 embedding（来自磁盘缓存或批量计算结果）。
    /// 若条目不存在则忽略。
    pub async fn set_embedding(&self, conv_id: &str, msg_id: &str, embedding: Vec<f32>) {
        let mut s = self.state.write().await;
        if let Some(entry) = s
            .entries
            .iter_mut()
            .find(|e| e.conversation_id == conv_id && e.message_id == msg_id)
        {
            entry.embedding = Some(embedding);
        }
    }

    /// 从外部缓存导入 embedding（key = `"<conv_id>:<msg_id>"`）。
    /// 仅匹配当前已索引条目；不存在的 key 忽略。
    pub async fn import_embeddings(&self, cache: &HashMap<String, Vec<f32>>) {
        let mut s = self.state.write().await;
        for entry in s.entries.iter_mut() {
            if entry.embedding.is_some() {
                continue;
            }
            if let Some(emb) = cache.get(&entry.cache_key()) {
                entry.embedding = Some(emb.clone());
            }
        }
    }

    /// 导出当前所有已计算的 embedding，用于持久化缓存。
    pub async fn export_embeddings(&self) -> HashMap<String, Vec<f32>> {
        let s = self.state.read().await;
        let mut out = HashMap::with_capacity(s.entries.len());
        for entry in s
            .entries
            .iter()
            .filter_map(|e| e.embedding.as_ref().map(|emb| (e.cache_key(), emb)))
        {
            out.insert(entry.0, entry.1.clone());
        }
        out
    }

    /// 索引统计信息
    pub async fn stats(&self) -> MemoryStats {
        let s = self.state.read().await;
        let embedded = s.entries.iter().filter(|e| e.embedding.is_some()).count();
        MemoryStats {
            total_entries: s.entries.len(),
            unique_tokens: s.inverted.len(),
            embedded_entries: embedded,
            avg_doc_len: s.avg_dl(),
        }
    }

    /// 批量为尚未计算 embedding 的条目计算向量（best-effort，progressive）。
    ///
    /// - 取最多 `limit` 条未嵌入的条目
    /// - 通过 provider 批量计算 embedding（HTTP IO 在锁外）
    /// - 写回索引（短暂写锁）
    ///
    /// 若未设置 provider 则立即返回 Ok(())。
    /// 调用方可在后台 task 中循环调用此方法，直到 `stats().embedded_entries == total_entries`。
    pub async fn ensure_embeddings(&self, limit: usize) -> crate::Result<usize> {
        if limit == 0 {
            return Ok(0);
        }
        // 1. 读出 provider 与待嵌入条目（短暂读锁）
        let (provider, to_embed) = {
            let s = self.state.read().await;
            let provider = match s.embedding_provider.clone() {
                Some(p) => p,
                None => return Ok(0),
            };
            let to_embed: Vec<(String, String, String)> = s
                .entries
                .iter()
                .filter(|e| e.embedding.is_none())
                .take(limit)
                .map(|e| {
                    (
                        e.conversation_id.clone(),
                        e.message_id.clone(),
                        e.content.clone(),
                    )
                })
                .collect();
            (provider, to_embed)
        };

        if to_embed.is_empty() {
            return Ok(0);
        }

        // 2. 锁外批量计算 embedding
        let texts: Vec<&str> = to_embed.iter().map(|(_, _, t)| t.as_str()).collect();
        let embeddings = provider.embed(&texts).await?;

        if embeddings.len() != to_embed.len() {
            return Err(CoreError::Agent(format!(
                "ensure_embeddings: 数量不匹配 期望 {} 实际 {}",
                to_embed.len(),
                embeddings.len()
            )));
        }

        // 3. 写回（短暂写锁）
        let mut s = self.state.write().await;
        for ((conv_id, msg_id, _), emb) in to_embed.iter().zip(embeddings) {
            if let Some(entry) = s
                .entries
                .iter_mut()
                .find(|e| e.conversation_id == *conv_id && e.message_id == *msg_id)
            {
                entry.embedding = Some(emb);
            }
        }
        Ok(to_embed.len())
    }

    /// 词法 BM25 检索
    ///
    /// - `query`：查询字符串，自动分词
    /// - `limit`：最多返回条数
    /// - `exclude_conv`：排除某 conversation 的条目（通常为当前会话）
    pub async fn search_lexical(
        &self,
        query: &str,
        limit: usize,
        exclude_conv: Option<&str>,
    ) -> Vec<MemoryHit> {
        let keywords = tokenize(query);
        if keywords.is_empty() {
            return Vec::new();
        }
        let s = self.state.read().await;
        bm25_search(&s, &keywords, limit, exclude_conv)
    }

    /// 向量 embedding 检索
    ///
    /// 若未设置 `EmbeddingProvider` 或无条目含 embedding，返回空 Vec。
    /// 查询 embedding 通过 provider 异步计算（IO 在锁外）。
    pub async fn search_vector(
        &self,
        query: &str,
        limit: usize,
        exclude_conv: Option<&str>,
    ) -> crate::Result<Vec<MemoryHit>> {
        // 1. 读出 provider（短暂读锁）
        let provider = {
            let s = self.state.read().await;
            match s.embedding_provider.clone() {
                Some(p) => p,
                None => return Ok(Vec::new()),
            }
        };

        // 2. 锁外计算 query embedding（IO/网络）
        let q_emb = provider.embed(&[query]).await?;
        if q_emb.is_empty() {
            return Ok(Vec::new());
        }
        let q_vec = &q_emb[0];

        // 3. 读出 entries 快照（短暂读锁），锁内仅内存比对
        let s = self.state.read().await;
        Ok(cosine_search(&s, q_vec, limit, exclude_conv))
    }

    /// 混合检索（RRF 融合 BM25 + Vector）
    ///
    /// 两路分别取 top-N（N = max(limit*4, 20)），按 RRF 合并后取 top-limit。
    /// 若向量路不可用则退化为纯词法。
    pub async fn search_hybrid(
        &self,
        query: &str,
        limit: usize,
        exclude_conv: Option<&str>,
    ) -> Vec<MemoryHit> {
        let n = (limit * 4).max(20);
        // 词法路：同步内存操作
        let lexical = self.search_lexical(query, n, exclude_conv).await;
        // 向量路：可能因无 provider 返回空
        let vector = self
            .search_vector(query, n, exclude_conv)
            .await
            .unwrap_or_default();

        if vector.is_empty() {
            // 退化为纯词法
            return lexical.into_iter().take(limit).collect();
        }
        if lexical.is_empty() {
            return vector.into_iter().take(limit).collect();
        }
        rrf_fuse(lexical, vector, limit, DEFAULT_RRF_K)
    }

    /// 按 mode 分派检索
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        mode: SearchMode,
        exclude_conv: Option<&str>,
    ) -> Vec<MemoryHit> {
        match mode {
            SearchMode::Lexical => self.search_lexical(query, limit, exclude_conv).await,
            SearchMode::Vector => self
                .search_vector(query, limit, exclude_conv)
                .await
                .unwrap_or_default(),
            SearchMode::Hybrid => self.search_hybrid(query, limit, exclude_conv).await,
        }
    }
}

impl Default for MemoryIndex {
    fn default() -> Self {
        Self::new()
    }
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    fn msg(id: &str, role: Role, content: &str, ts: u64) -> Message {
        Message::new(id, role, content, ts)
    }

    #[test]
    fn tokenize_handles_mixed_punctuation() {
        let tokens = tokenize("Rust 是一门系统编程语言，rust good!");
        // ASCII 整词
        assert!(tokens.contains(&"rust".to_string()));
        assert!(tokens.contains(&"good".to_string()));
        // CJK 被拆为单字 + bigram，不再保留整串 token
        assert!(!tokens.contains(&"是一门系统编程语言".to_string()));
        // bigram: "是一" / "一门" / "门系" / ...
        assert!(tokens.contains(&"是一".to_string()));
        assert!(tokens.contains(&"一门".to_string()));
        // 单字
        assert!(tokens.contains(&"是".to_string()));
        assert!(tokens.contains(&"门".to_string()));
    }

    #[test]
    fn tokenize_cjk_single_char() {
        // 单字 CJK:仅产生单字 token,bigram 为空
        let tokens = tokenize("异");
        assert_eq!(tokens, vec!["异".to_string()]);
    }

    #[test]
    fn tokenize_cjk_bigram_and_unigram() {
        // "异步编程" → bigram [异步, 步编, 编程] + 单字 [异, 步, 编, 程]
        let tokens = tokenize("异步编程");
        assert!(tokens.contains(&"异步".to_string()));
        assert!(tokens.contains(&"步编".to_string()));
        assert!(tokens.contains(&"编程".to_string()));
        assert!(tokens.contains(&"异".to_string()));
        assert!(tokens.contains(&"步".to_string()));
        assert!(tokens.contains(&"编".to_string()));
        assert!(tokens.contains(&"程".to_string()));
    }

    #[tokio::test]
    async fn cjk_short_query_hits_cross_conversation() {
        // 回归测试:中文短查询应能命中跨会话的长文本(此前 BM25 路召回为 0)
        let idx = MemoryIndex::new();
        idx.add(
            "conv_a",
            msg("m1", Role::User, "我们讨论过异步编程的优缺点", 1),
        )
        .await;
        idx.add("conv_b", msg("m2", Role::User, "今天天气真不错适合出门", 2))
            .await;

        // 在 conv_b 中查询 "异步",应命中 conv_a 的消息
        let hits = idx.search_lexical("异步", 5, Some("conv_b")).await;
        assert_eq!(hits.len(), 1, "短查询应能跨会话命中");
        assert_eq!(hits[0].conversation_id, "conv_a");
        assert!(hits[0].snippet.contains("异步"));
    }

    #[tokio::test]
    async fn add_and_search_lexical_basic() {
        let idx = MemoryIndex::new();
        idx.add("c1", msg("m1", Role::User, "Rust 是一门系统编程语言", 1))
            .await;
        idx.add(
            "c1",
            msg("m2", Role::Assistant, "Rust 强调内存安全与零成本抽象", 2),
        )
        .await;
        idx.add("c2", msg("m3", Role::User, "今天天气真好", 3))
            .await;

        let hits = idx.search_lexical("rust", 5, None).await;
        assert_eq!(hits.len(), 2);
        assert!(hits
            .iter()
            .all(|h| h.snippet.to_lowercase().contains("rust")));
    }

    #[tokio::test]
    async fn add_is_idempotent() {
        let idx = MemoryIndex::new();
        let m = msg("m1", Role::User, "hello world", 1);
        idx.add("c1", m.clone()).await;
        idx.add("c1", m.clone()).await;
        let stats = idx.stats().await;
        assert_eq!(stats.total_entries, 1);
    }

    #[tokio::test]
    async fn search_excludes_current_conversation() {
        let idx = MemoryIndex::new();
        idx.add("c1", msg("m1", Role::User, "rust programming", 1))
            .await;
        idx.add("c2", msg("m2", Role::User, "rust language", 2))
            .await;

        let hits = idx.search_lexical("rust", 5, Some("c1")).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].conversation_id, "c2");
    }

    #[tokio::test]
    async fn bm25_ranks_repeated_terms_higher() {
        let idx = MemoryIndex::new();
        idx.add("c1", msg("m1", Role::User, "rust rust rust rust", 1))
            .await;
        idx.add("c2", msg("m2", Role::User, "rust once", 2)).await;

        let hits = idx.search_lexical("rust", 5, None).await;
        assert_eq!(hits.len(), 2);
        // 重复词频更高的文档应排名在前
        assert_eq!(hits[0].conversation_id, "c1");
        assert!(hits[0].score > hits[1].score);
    }

    #[tokio::test]
    async fn empty_content_and_system_messages_are_skipped() {
        let idx = MemoryIndex::new();
        idx.add("c1", msg("m1", Role::System, "system preamble", 1))
            .await;
        idx.add("c1", msg("m2", Role::User, "", 2)).await;
        idx.add("c1", msg("m3", Role::User, "real content", 3))
            .await;
        let stats = idx.stats().await;
        assert_eq!(stats.total_entries, 1);
    }

    #[tokio::test]
    async fn rebuild_clears_old_data() {
        let idx = MemoryIndex::new();
        idx.add("c1", msg("m1", Role::User, "old data", 1)).await;
        assert_eq!(idx.stats().await.total_entries, 1);

        idx.rebuild_from_messages(vec![(
            "c2".to_string(),
            msg("m2", Role::User, "new data", 2),
        )])
        .await;
        let stats = idx.stats().await;
        assert_eq!(stats.total_entries, 1);
        let hits = idx.search_lexical("old", 5, None).await;
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn vector_search_without_provider_returns_empty() {
        let idx = MemoryIndex::new();
        idx.add("c1", msg("m1", Role::User, "hello", 1)).await;
        let hits = idx.search_vector("hello", 5, None).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn vector_search_with_provider_and_embeddings() {
        // 简单 stub provider：返回固定向量
        struct StubProvider;
        #[async_trait]
        impl EmbeddingProvider for StubProvider {
            async fn embed(&self, texts: &[&str]) -> crate::Result<Vec<Vec<f32>>> {
                Ok(texts
                    .iter()
                    .map(|t| {
                        // 简单"嵌入"：把字符串字节当向量，长度归一化到 4 维
                        let len = t.len().max(1) as f32;
                        vec![len, len * 0.5, len * 0.25, len * 0.125]
                    })
                    .map(|v| {
                        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                        v.into_iter().map(|x| x / norm.max(1e-9)).collect()
                    })
                    .collect())
            }
        }

        let idx = MemoryIndex::new();
        idx.set_embedding_provider(Arc::new(StubProvider)).await;
        idx.add("c1", msg("m1", Role::User, "aaaa", 1)).await;
        idx.add("c2", msg("m2", Role::User, "aaaaaaaa", 2)).await;

        // 为已索引条目补 embedding（模拟批量计算）
        let s = idx.state.read().await;
        let keys: Vec<(String, String)> = s
            .entries
            .iter()
            .map(|e| (e.conversation_id.clone(), e.message_id.clone()))
            .collect();
        drop(s);
        for (conv_id, msg_id) in keys {
            idx.set_embedding(&conv_id, &msg_id, vec![0.5, 0.25, 0.125, 0.0625])
                .await;
        }

        let hits = idx.search_vector("aaaa", 5, None).await.unwrap();
        assert!(!hits.is_empty());
    }

    #[tokio::test]
    async fn hybrid_falls_back_to_lexical_without_provider() {
        let idx = MemoryIndex::new();
        idx.add("c1", msg("m1", Role::User, "rust programming", 1))
            .await;
        let hits = idx.search_hybrid("rust", 5, None).await;
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn export_import_embeddings_roundtrip() {
        let idx = MemoryIndex::new();
        idx.add("c1", msg("m1", Role::User, "hello", 1)).await;
        idx.set_embedding("c1", "m1", vec![0.1, 0.2, 0.3]).await;

        let exported = idx.export_embeddings().await;
        assert_eq!(exported.len(), 1);
        assert!(exported.contains_key("c1:m1"));

        // 新索引导入后应有 embedding
        let idx2 = MemoryIndex::new();
        idx2.add("c1", msg("m1", Role::User, "hello", 1)).await;
        idx2.import_embeddings(&exported).await;
        let stats = idx2.stats().await;
        assert_eq!(stats.embedded_entries, 1);
    }

    #[tokio::test]
    async fn rrf_fuses_two_lists() {
        let lexical = vec![
            MemoryHit {
                conversation_id: "c1".into(),
                message_id: "m1".into(),
                snippet: "a".into(),
                timestamp: 1,
                score: 1.0,
                role: Role::User,
            },
            MemoryHit {
                conversation_id: "c2".into(),
                message_id: "m2".into(),
                snippet: "b".into(),
                timestamp: 2,
                score: 0.5,
                role: Role::User,
            },
        ];
        let vector = vec![
            MemoryHit {
                conversation_id: "c2".into(),
                message_id: "m2".into(),
                snippet: "b".into(),
                timestamp: 2,
                score: 0.9,
                role: Role::User,
            },
            MemoryHit {
                conversation_id: "c3".into(),
                message_id: "m3".into(),
                snippet: "c".into(),
                timestamp: 3,
                score: 0.8,
                role: Role::User,
            },
        ];
        let fused = rrf_fuse(lexical, vector, 3, 60);
        assert_eq!(fused.len(), 3);
        // c2:m2 同时出现在两路，RRF 分数应最高
        assert_eq!(fused[0].conversation_id, "c2");
        assert_eq!(fused[0].message_id, "m2");
    }

    #[test]
    fn cosine_sim_orthogonal_vectors_are_zero() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let b_norm = vec_norm(&b);
        assert!((cosine_sim(&a, &b, b_norm) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn cosine_sim_identical_vectors_are_one() {
        let a = vec![1.0, 2.0, 3.0];
        let b_norm = vec_norm(&a);
        assert!((cosine_sim(&a, &a, b_norm) - 1.0).abs() < 1e-9);
    }
}
