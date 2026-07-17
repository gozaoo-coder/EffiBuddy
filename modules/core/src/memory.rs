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
//! - **向量（embedding 余弦相似度）**：通过 [`EmbeddingProvider`] 异步获取查询向量，
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
//! - embedding 持久化缓存由外部 `import_embeddings`/`export_embeddings` 注入，
//!   避免在 core 引入文件 IO。

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{CoreError, Message, Role};

/// 默认 BM25 参数 k1（词频饱和参数）
const DEFAULT_K1: f64 = 1.2;
/// 默认 BM25 参数 b（文档长度归一化参数）
const DEFAULT_B: f64 = 0.75;
/// RRF 默认 k 值（rank fusion 平滑参数）
const DEFAULT_RRF_K: u32 = 60;

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
    fn dl(&self) -> usize {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    /// 词法 BM25
    Lexical,
    /// 向量 embedding 余弦相似度
    Vector,
    /// 混合 RRF（默认推荐）
    Hybrid,
}

impl Default for SearchMode {
    fn default() -> Self {
        Self::Hybrid
    }
}

/// 索引内部可变状态（被 RwLock 包裹）
struct IndexState {
    entries: Vec<MemoryEntry>,
    /// 倒排表：token → [(entry_idx, tf)]
    inverted: HashMap<String, Vec<(usize, u32)>>,
    /// 文档频率：token → 出现该 token 的文档数
    df: HashMap<String, u32>,
    /// 文档总数
    n_docs: u64,
    /// 文档总长度（token 数之和）
    total_dl: u64,
    /// BM25 参数
    k1: f64,
    b: f64,
    /// 嵌入向量提供者（可热替换）
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl IndexState {
    #[inline]
    fn avg_dl(&self) -> f64 {
        if self.n_docs == 0 {
            0.0
        } else {
            self.total_dl as f64 / self.n_docs as f64
        }
    }

    /// BM25 的 IDF：采用 Okapi 经典公式 `ln((N - df + 0.5) / (df + 0.5) + 1)`，
    /// 保证非负（Lucene/ES 默认变体）。
    #[inline]
    fn idf(&self, df: u32) -> f64 {
        let n = self.n_docs as f64;
        let df = df as f64;
        ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
    }
}

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
        for entry in s.entries.iter().filter_map(|e| e.embedding.as_ref().map(|emb| (e.cache_key(), emb))) {
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
                .map(|e| (e.conversation_id.clone(), e.message_id.clone(), e.content.clone()))
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
        for ((conv_id, msg_id, _), emb) in to_embed.iter().zip(embeddings.into_iter()) {
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

/// 索引统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    pub total_entries: usize,
    pub unique_tokens: usize,
    pub embedded_entries: usize,
    pub avg_doc_len: f64,
}

// =========================================================
// 内部辅助函数
// =========================================================

/// 把一条 entry 推入索引状态：更新 entries / inverted / df / n_docs / total_dl
fn push_entry(state: &mut IndexState, entry: MemoryEntry) {
    let idx = state.entries.len();
    let dl = entry.dl();
    // 统计每个 token 的 tf（用 HashMap 累积避免重复遍历）
    let mut tf_map: HashMap<&str, u32> = HashMap::with_capacity(entry.tokens.len());
    for tok in &entry.tokens {
        *tf_map.entry(tok.as_str()).or_insert(0) += 1;
    }
    for (tok, tf) in tf_map {
        state
            .inverted
            .entry(tok.to_string())
            .or_insert_with(Vec::new)
            .push((idx, tf));
        *state.df.entry(tok.to_string()).or_insert(0) += 1;
    }
    state.n_docs += 1;
    state.total_dl += dl as u64;
    state.entries.push(entry);
}

/// 分词：按空白与中英文标点切分，转小写
///
/// 与 `search_history`/`storage` 中的分词保持一致，
/// 确保跨模块检索行为统一。
fn tokenize(content: &str) -> Vec<String> {
    content
        .split(|c: char| c.is_whitespace() || "，。、；：！？,.:;!?\"'`()[]{}【】《》".contains(c))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

/// BM25 检索：仅对含查询词的文档打分
fn bm25_search(
    state: &IndexState,
    keywords: &[String],
    limit: usize,
    exclude_conv: Option<&str>,
) -> Vec<MemoryHit> {
    if state.entries.is_empty() || keywords.is_empty() {
        return Vec::new();
    }
    let avg_dl = state.avg_dl();
    let n_docs = state.n_docs as f64;
    let k1 = state.k1;
    let b = state.b;

    // 候选文档：通过倒排表汇总，避免全量遍历
    // 候选 entry_idx -> 累积 BM25 分数
    let mut scores: HashMap<usize, f64> = HashMap::with_capacity(32);
    for kw in keywords {
        let df = match state.df.get(kw) {
            Some(&d) => d,
            None => continue,
        };
        let idf = state.idf(df);
        if let Some(postings) = state.inverted.get(kw) {
            for &(idx, tf) in postings {
                let entry = &state.entries[idx];
                if let Some(ex) = exclude_conv {
                    if entry.conversation_id == ex {
                        continue;
                    }
                }
                let dl = entry.dl() as f64;
                let tf = tf as f64;
                // BM25 tf 组件：tf * (k1 + 1) / (tf + k1 * (1 - b + b * dl / avg_dl))
                let denom = tf + k1 * (1.0 - b + b * (if avg_dl > 0.0 { dl / avg_dl } else { 0.0 }));
                let tf_component = if denom > 0.0 { tf * (k1 + 1.0) / denom } else { 0.0 };
                *scores.entry(idx).or_insert(0.0) += idf * tf_component;
            }
        }
    }

    // 按分数降序取 top-limit
    let mut scored: Vec<(usize, f64)> = scores.into_iter().collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(limit)
        .map(|(idx, score)| {
            let entry = &state.entries[idx];
            MemoryHit {
                conversation_id: entry.conversation_id.clone(),
                message_id: entry.message_id.clone(),
                snippet: make_snippet(&entry.content, 100),
                timestamp: entry.timestamp,
                score: score as f32,
                role: entry.role,
            }
        })
        .collect()
}

/// 向量余弦相似度检索
fn cosine_search(
    state: &IndexState,
    q_vec: &[f32],
    limit: usize,
    exclude_conv: Option<&str>,
) -> Vec<MemoryHit> {
    if state.entries.is_empty() || q_vec.is_empty() {
        return Vec::new();
    }
    let q_norm = vec_norm(q_vec);
    if q_norm == 0.0 {
        return Vec::new();
    }

    // 迭代器链：过滤已嵌入 + 排除当前会话 + 计算相似度 + 取 top-limit
    let mut scored: Vec<(usize, f64)> = state
        .entries
        .iter()
        .enumerate()
        .filter_map(|(idx, e)| {
            if let Some(ex) = exclude_conv {
                if e.conversation_id == ex {
                    return None;
                }
            }
            let emb = e.embedding.as_ref()?;
            let sim = cosine_sim(emb, q_vec, q_norm);
            Some((idx, sim))
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(limit)
        .map(|(idx, sim)| {
            let entry = &state.entries[idx];
            MemoryHit {
                conversation_id: entry.conversation_id.clone(),
                message_id: entry.message_id.clone(),
                snippet: make_snippet(&entry.content, 100),
                timestamp: entry.timestamp,
                score: sim as f32,
                role: entry.role,
            }
        })
        .collect()
}

/// Reciprocal Rank Fusion：融合两路结果
///
/// `score = Σ 1 / (k + rank)`，rank 从 1 开始。无需归一化原始分数。
fn rrf_fuse(lexical: Vec<MemoryHit>, vector: Vec<MemoryHit>, limit: usize, k: u32) -> Vec<MemoryHit> {
    // 用 (conv_id, msg_id) 作为合并键
    let mut fused: HashMap<(String, String), (MemoryHit, f64)> = HashMap::with_capacity(lexical.len() + vector.len());

    for (rank, hit) in lexical.iter().enumerate() {
        let key = (hit.conversation_id.clone(), hit.message_id.clone());
        let rrf_score = 1.0 / (k as f64 + (rank + 1) as f64);
        fused
            .entry(key)
            .and_modify(|e| e.1 += rrf_score)
            .or_insert_with(|| (hit.clone(), rrf_score));
    }
    for (rank, hit) in vector.iter().enumerate() {
        let key = (hit.conversation_id.clone(), hit.message_id.clone());
        let rrf_score = 1.0 / (k as f64 + (rank + 1) as f64);
        fused
            .entry(key)
            .and_modify(|e| e.1 += rrf_score)
            .or_insert_with(|| (hit.clone(), rrf_score));
    }

    let mut out: Vec<(MemoryHit, f64)> = fused.into_values().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out.into_iter()
        .take(limit)
        .map(|(mut hit, score)| {
            hit.score = score as f32;
            hit
        })
        .collect()
}

/// 向量 L2 范数
#[inline]
fn vec_norm(v: &[f32]) -> f64 {
    v.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt()
}

/// 余弦相似度（precomputable q_norm 优化）
#[inline]
fn cosine_sim(a: &[f32], b: &[f32], b_norm: f64) -> f64 {
    let a_norm = vec_norm(a);
    if a_norm == 0.0 || b_norm == 0.0 {
        return 0.0;
    }
    let dot: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (*x as f64) * (*y as f64))
        .sum();
    dot / (a_norm * b_norm)
}

/// 生成片段：取前 max_chars 字符，在 UTF-8 字符边界处截断
fn make_snippet(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    let boundary = content.ceil_char_boundary(max_chars);
    let mut s = String::with_capacity(boundary + 3);
    s.push_str(&content[..boundary]);
    s.push('…');
    s
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, role: Role, content: &str, ts: u64) -> Message {
        Message::new(id, role, content, ts)
    }

    #[test]
    fn tokenize_handles_mixed_punctuation() {
        let tokens = tokenize("Rust 是一门系统编程语言，rust good!");
        assert!(tokens.contains(&"rust".to_string()));
        assert!(tokens.contains(&"是".to_string()));
        assert!(tokens.contains(&"good".to_string()));
    }

    #[tokio::test]
    async fn add_and_search_lexical_basic() {
        let idx = MemoryIndex::new();
        idx.add("c1", msg("m1", Role::User, "Rust 是一门系统编程语言", 1)).await;
        idx.add("c1", msg("m2", Role::Assistant, "Rust 强调内存安全与零成本抽象", 2)).await;
        idx.add("c2", msg("m3", Role::User, "今天天气真好", 3)).await;

        let hits = idx.search_lexical("rust", 5, None).await;
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.snippet.to_lowercase().contains("rust")));
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
        idx.add("c1", msg("m1", Role::User, "rust programming", 1)).await;
        idx.add("c2", msg("m2", Role::User, "rust language", 2)).await;

        let hits = idx.search_lexical("rust", 5, Some("c1")).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].conversation_id, "c2");
    }

    #[tokio::test]
    async fn bm25_ranks_repeated_terms_higher() {
        let idx = MemoryIndex::new();
        idx.add("c1", msg("m1", Role::User, "rust rust rust rust", 1)).await;
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
        idx.add("c1", msg("m1", Role::System, "system preamble", 1)).await;
        idx.add("c1", msg("m2", Role::User, "", 2)).await;
        idx.add("c1", msg("m3", Role::User, "real content", 3)).await;
        let stats = idx.stats().await;
        assert_eq!(stats.total_entries, 1);
    }

    #[tokio::test]
    async fn rebuild_clears_old_data() {
        let idx = MemoryIndex::new();
        idx.add("c1", msg("m1", Role::User, "old data", 1)).await;
        assert_eq!(idx.stats().await.total_entries, 1);

        idx.rebuild_from_messages(vec![("c2".to_string(), msg("m2", Role::User, "new data", 2))])
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
        idx.add("c1", msg("m1", Role::User, "rust programming", 1)).await;
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
