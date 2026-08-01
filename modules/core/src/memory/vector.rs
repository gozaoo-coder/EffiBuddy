//! RAG 记忆增强：向量 embedding 检索
//!
//! 实现基于余弦相似度的向量检索（[`cosine_search`]）。
//! 通过 [`EmbeddingProvider`](super::EmbeddingProvider) 异步获取查询向量后，
//! 与每条已嵌入条目比对。Provider 由上层（agent 模块）注入，core 不依赖网络。
//!
//! 查询路径全部用迭代器适配器，结果 Vec 用 `with_capacity` 预分配。

use super::types::{IndexState, MemoryHit, make_snippet, SNIPPET_MAX_CHARS};

/// 向量余弦相似度检索
pub(super) fn cosine_search(
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
                snippet: make_snippet(&entry.content, SNIPPET_MAX_CHARS),
                timestamp: entry.timestamp,
                score: sim as f32,
                role: entry.role,
            }
        })
        .collect()
}

/// 向量 L2 范数
#[inline]
pub(super) fn vec_norm(v: &[f32]) -> f64 {
    v.iter()
        .map(|x| (*x as f64) * (*x as f64))
        .sum::<f64>()
        .sqrt()
}

/// 余弦相似度（precomputable q_norm 优化）
#[inline]
pub(super) fn cosine_sim(a: &[f32], b: &[f32], b_norm: f64) -> f64 {
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
