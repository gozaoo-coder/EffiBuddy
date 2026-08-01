//! 已安装技能的轻量检索索引（RAG 自动注入用）
//!
//! 与 [`MemoryIndex`] 不同，本索引面向**已安装技能**（数量少、内容稳定），
//! 仅提供 BM25 词法检索即可满足需求，不引入向量路复杂度。
//!
//! # 用途
//!
//! - [`RigAgent`](../../effisuite_agent/rig_agent/struct.RigAgent.html) 在
//!   `build_context_parts` 中调用 `search` 自动检索 Top-K 相关技能，
//!   把 name + description 注入到 `[可用技能]` 段，让 agent 知道"我能用什么"
//! - [`ListInstalledSkillsTool`] / [`GetSkillDetailTool`] 等工具直接读
//!   `SkillStore`，不依赖本索引；本索引仅服务于 RAG 自动注入
//!
//! # 并发与性能（对齐 user_rules）
//!
//! - 索引状态包在 `RwLock` 中：查询并发读，写入短暂持写锁且锁内零 IO
//! - `SkillEntry` 字段按大小降序：String(24) → Vec(24) → u64(8) → bool(1)
//! - 查询路径全部用迭代器适配器，无显式 `for i in 0..len` 索引循环
//! - 结果 Vec 用 `with_capacity` 预分配
//! - 复用 `memory::tokenize`，避免重复实现分词逻辑

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{tokenize, Skill, SkillStore};

/// 默认 BM25 参数 k1（词频饱和参数）
const DEFAULT_K1: f64 = 1.2;
/// 默认 BM25 参数 b（文档长度归一化参数）
const DEFAULT_B: f64 = 0.75;

/// 一条被索引的技能条目
///
/// 字段按大小降序排列以最小化 padding：
/// `id`/`name`/`description`/`tokens`（String/Vec, 24B）
/// → `created_at`（u64, 8B）→ `builtin`（bool, 1B）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 预分词结果（小写化），避免每次查询重复切词
    pub tokens: Vec<String>,
    pub created_at: u64,
    pub builtin: bool,
}

impl SkillEntry {
    /// 文档长度（token 数），BM25 用
    #[inline]
    fn dl(&self) -> usize {
        self.tokens.len()
    }
}

/// 检索命中结果
///
/// 字段按大小降序：String(24) → u64(8) → f32(4) → bool(1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillHit {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: u64,
    /// 相关性得分，越大越相关（BM25）
    pub score: f32,
    pub builtin: bool,
}

/// 索引内部可变状态（被 RwLock 包裹）
struct IndexState {
    entries: Vec<SkillEntry>,
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

/// 已安装技能检索索引，线程安全可廉价 clone（内部 RwLock + Arc）
///
/// 典型用法：
/// - 启动时 `rebuild` 从 `SkillStore` 全量重建
/// - 技能增删后 `rebuild` 全量刷新（技能数量少，全量重建开销可忽略）
/// - `RigAgent` 在 `build_context_parts` 中调用 `search` 自动注入 `[可用技能]` 段
#[derive(Clone)]
pub struct SkillIndex {
    state: Arc<RwLock<IndexState>>,
}

impl SkillIndex {
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
            })),
        }
    }

    /// 从 `SkillStore` 全量重建索引。
    ///
    /// 锁内仅做内存操作，零 IO。SkillStore 的 list_all 是异步 IO，
    /// 由调用方在锁外完成（传入已加载的 Vec<Skill>）。
    pub async fn rebuild(&self, skills: Vec<Skill>) {
        let mut s = self.state.write().await;
        s.entries.clear();
        s.inverted.clear();
        s.df.clear();
        s.n_docs = 0;
        s.total_dl = 0;

        // 预分配 entries 容量，避免 push 扩容
        s.entries.reserve(skills.len());
        for (idx, skill) in skills.into_iter().enumerate() {
            // 拼接 name + description 作为文档内容（preamble 太长不入索引，
            // agent 通过 get_skill_detail 工具按需获取完整 preamble）
            let doc = format!("{} {}", skill.name, skill.description);
            let tokens = tokenize(&doc);
            // 累加 total_dl
            s.total_dl += tokens.len() as u64;
            s.n_docs += 1;
            // 构建 term frequency 表：token → tf
            let mut tf_map: HashMap<String, u32> = HashMap::with_capacity(tokens.len());
            for t in &tokens {
                *tf_map.entry(t.clone()).or_insert(0) += 1;
            }
            // 更新倒排表与 df
            for (t, tf) in tf_map {
                s.inverted.entry(t.clone()).or_default().push((idx, tf));
                *s.df.entry(t).or_insert(0) += 1;
            }
            s.entries.push(SkillEntry {
                id: skill.id,
                name: skill.name,
                description: skill.description,
                tokens,
                created_at: skill.created_at,
                builtin: skill.builtin,
            });
        }
    }

    /// 便捷方法：从 SkillStore 全量加载并重建（IO 在锁外）。
    ///
    /// 供 Tauri 命令层在技能增删后调用，避免重复写 list_all + rebuild 模板。
    pub async fn rebuild_from_store(&self, store: &SkillStore) -> crate::Result<()> {
        let skills = store.list_all().await?;
        self.rebuild(skills).await;
        Ok(())
    }

    /// 返回当前索引的条目数（含内置技能）
    pub async fn len(&self) -> usize {
        self.state.read().await.entries.len()
    }

    /// 索引是否为空
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    /// 列出全部已索引技能（按 created_at 降序）。
    ///
    /// 供 ListInstalledSkillsTool 使用，避免直接走 SkillStore IO。
    pub async fn list_all(&self) -> Vec<SkillEntry> {
        let s = self.state.read().await;
        let mut out: Vec<SkillEntry> = s.entries.to_vec();
        out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        out
    }

    /// 按 id 查找已索引技能。
    ///
    /// 供 GetSkillDetailTool 等需要快速定位单个技能的工具使用。
    /// 注意：返回的是索引快照（不含 preamble/working_dir 等完整字段），
    /// 需要完整字段时仍应走 `SkillStore::get`。
    pub async fn find_by_id(&self, id: &str) -> Option<SkillEntry> {
        let s = self.state.read().await;
        s.entries.iter().find(|e| e.id == id).cloned()
    }

    /// BM25 词法检索：返回与 query 最相关的 Top-K 技能。
    ///
    /// 利用倒排表仅对含查询词的文档打分，避免全量遍历。
    /// 空查询或空索引返回空 Vec。
    pub async fn search(&self, query: &str, limit: usize) -> Vec<SkillHit> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() {
            return Vec::new();
        }
        let limit = if limit == 0 { 5 } else { limit };

        let s = self.state.read().await;
        if s.n_docs == 0 {
            return Vec::new();
        }
        let avg_dl = s.avg_dl();
        let n_docs = s.n_docs as f64;

        // 候选文档：仅对含查询词的文档打分
        // 用 HashMap 累加每个候选文档的 BM25 分数
        let mut scores: HashMap<usize, f64> = HashMap::with_capacity(query_tokens.len() * 4);
        for qt in &query_tokens {
            let Some(postings) = s.inverted.get(qt) else {
                continue;
            };
            let df = *s.df.get(qt).unwrap_or(&0);
            if df == 0 {
                continue;
            }
            let idf = s.idf(df);
            for &(entry_idx, tf) in postings {
                let entry = &s.entries[entry_idx];
                let dl = entry.dl() as f64;
                let tf = tf as f64;
                // Okapi BM25 打分
                let denom = tf + s.k1 * (1.0 - s.b + s.b * (dl / avg_dl.max(1e-9)));
                if denom == 0.0 {
                    continue;
                }
                let score = idf * (tf * (s.k1 + 1.0)) / denom;
                *scores.entry(entry_idx).or_insert(0.0) += score;
            }
        }

        // 按分数降序取 Top-K
        let mut ranked: Vec<(usize, f64)> = scores.into_iter().collect();
        // 用 sort_by 即可，n 通常很小（<100），无需 partial_sort
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(limit);

        // 构造命中结果
        let mut hits: Vec<SkillHit> = Vec::with_capacity(ranked.len());
        for (idx, score) in ranked {
            let e = &s.entries[idx];
            hits.push(SkillHit {
                id: e.id.clone(),
                name: e.name.clone(),
                description: e.description.clone(),
                created_at: e.created_at,
                score: score as f32,
                builtin: e.builtin,
            });
        }
        // 防止编译器警告：n_docs 仅用于上面 idf 计算，此处显式引用
        let _ = n_docs;
        hits
    }
}

impl Default for SkillIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill(id: &str, name: &str, desc: &str, builtin: bool) -> Skill {
        Skill {
            id: id.to_string(),
            name: name.to_string(),
            description: desc.to_string(),
            preamble: String::new(),
            tools: Vec::new(),
            working_dir: None,
            created_at: 1,
            builtin,
            source: None,
            source_slug: None,
            source_owner: None,
            source_version: None,
        }
    }

    #[tokio::test]
    async fn rebuild_and_search_basic() {
        let idx = SkillIndex::new();
        idx.rebuild(vec![
            make_skill("weather", "Weather", "Get current weather forecast", false),
            make_skill(
                "translator",
                "Translator",
                "Translate text between languages",
                false,
            ),
            make_skill(
                "agent-reach",
                "Agent Reach",
                "Internet access and search",
                true,
            ),
        ])
        .await;

        assert_eq!(idx.len().await, 3);

        let hits = idx.search("weather", 5).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "weather");
        assert!(!hits[0].builtin);
    }

    #[tokio::test]
    async fn search_ranks_repeated_terms_higher() {
        let idx = SkillIndex::new();
        idx.rebuild(vec![
            make_skill("a", "Rust programming", "Rust language guide", false),
            make_skill("b", "Python", "Snake language", false),
        ])
        .await;

        // "rust" 出现在 a 的 name 与 description，应排第一
        let hits = idx.search("rust", 5).await;
        assert!(!hits.is_empty());
        assert_eq!(hits[0].id, "a");
    }

    #[tokio::test]
    async fn search_empty_query_returns_empty() {
        let idx = SkillIndex::new();
        idx.rebuild(vec![make_skill("a", "Skill", "desc", false)])
            .await;
        let hits = idx.search("   ", 5).await;
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn search_empty_index_returns_empty() {
        let idx = SkillIndex::new();
        let hits = idx.search("anything", 5).await;
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn list_all_sorted_by_created_at_desc() {
        let idx = SkillIndex::new();
        idx.rebuild(vec![
            make_skill("old", "Old", "desc", false),
            Skill {
                id: "new".to_string(),
                name: "New".to_string(),
                description: "desc".to_string(),
                preamble: String::new(),
                tools: Vec::new(),
                working_dir: None,
                created_at: 100,
                builtin: false,
                source: None,
                source_slug: None,
                source_owner: None,
                source_version: None,
            },
        ])
        .await;
        let list = idx.list_all().await;
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "new"); // created_at=100 在前
        assert_eq!(list[1].id, "old");
    }

    #[tokio::test]
    async fn find_by_id_returns_entry() {
        let idx = SkillIndex::new();
        idx.rebuild(vec![make_skill("weather", "Weather", "forecast", false)])
            .await;
        let e = idx.find_by_id("weather").await;
        assert!(e.is_some());
        assert_eq!(e.unwrap().name, "Weather");
        assert!(idx.find_by_id("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn rebuild_clears_old_data() {
        let idx = SkillIndex::new();
        idx.rebuild(vec![make_skill("a", "A", "desc", false)]).await;
        assert_eq!(idx.len().await, 1);
        // 重建为不同数据
        idx.rebuild(vec![make_skill("b", "B", "desc", false)]).await;
        assert_eq!(idx.len().await, 1);
        assert!(idx.find_by_id("a").await.is_none());
        assert!(idx.find_by_id("b").await.is_some());
    }

    #[tokio::test]
    async fn search_cjk_query() {
        let idx = SkillIndex::new();
        idx.rebuild(vec![
            make_skill("zh", "翻译助手", "中英文翻译工具", false),
            make_skill("en", "Weather", "forecast", false),
        ])
        .await;
        let hits = idx.search("翻译", 5).await;
        assert!(!hits.is_empty());
        assert_eq!(hits[0].id, "zh");
    }
}
