//! RAG 记忆增强：BM25 词法检索
//!
//! 实现 Okapi BM25 算法：
//! - 倒排索引构建（[`push_entry`]）：维护 token → [(entry_idx, tf)] 与文档频率表
//! - IDF / TF 计算（[`IndexState::idf`] / [`IndexState::avg_dl`]）
//! - 检索（[`bm25_search`]）：仅对含查询词的文档打分，避免全量遍历
//!
//! 利用倒排表 `HashMap<token, Vec<(entry_idx, tf)>>` 仅对含查询词的文档打分，
//! 查询路径全部用迭代器适配器，无显式 `for i in 0..len` 索引循环。

use std::collections::HashMap;

use super::types::{IndexState, MemoryEntry, MemoryHit, make_snippet};

impl IndexState {
    #[inline]
    pub(super) fn avg_dl(&self) -> f64 {
        if self.n_docs == 0 {
            0.0
        } else {
            self.total_dl as f64 / self.n_docs as f64
        }
    }

    /// BM25 的 IDF：采用 Okapi 经典公式 `ln((N - df + 0.5) / (df + 0.5) + 1)`，
    /// 保证非负（Lucene/ES 默认变体）。
    #[inline]
    pub(super) fn idf(&self, df: u32) -> f64 {
        let n = self.n_docs as f64;
        let df = df as f64;
        ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
    }
}

/// 把一条 entry 推入索引状态：更新 entries / inverted / df / n_docs / total_dl
pub(super) fn push_entry(state: &mut IndexState, entry: MemoryEntry) {
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
            .or_default()
            .push((idx, tf));
        *state.df.entry(tok.to_string()).or_insert(0) += 1;
    }
    state.n_docs += 1;
    state.total_dl += dl as u64;
    state.entries.push(entry);
}

/// BM25 检索：仅对含查询词的文档打分
pub(super) fn bm25_search(
    state: &IndexState,
    keywords: &[String],
    limit: usize,
    exclude_conv: Option<&str>,
) -> Vec<MemoryHit> {
    if state.entries.is_empty() || keywords.is_empty() {
        return Vec::new();
    }
    let avg_dl = state.avg_dl();
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
                let denom =
                    tf + k1 * (1.0 - b + b * (if avg_dl > 0.0 { dl / avg_dl } else { 0.0 }));
                let tf_component = if denom > 0.0 {
                    tf * (k1 + 1.0) / denom
                } else {
                    0.0
                };
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
