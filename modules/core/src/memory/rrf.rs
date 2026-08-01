//! RAG 记忆增强：RRF 混合融合
//!
//! 实现 Reciprocal Rank Fusion：对两路检索结果按
//! `score = Σ 1/(k + rank)` 合并，无需归一化原始分数，鲁棒性高。
//!
//! 用 `(conversation_id, message_id)` 作为合并键，保证同一文档在两路结果中合并得分。

use std::collections::HashMap;

use super::types::MemoryHit;

/// Reciprocal Rank Fusion：融合两路结果
///
/// `score = Σ 1 / (k + rank)`，rank 从 1 开始。无需归一化原始分数。
pub(super) fn rrf_fuse(
    lexical: Vec<MemoryHit>,
    vector: Vec<MemoryHit>,
    limit: usize,
    k: u32,
) -> Vec<MemoryHit> {
    // 用 (conv_id, msg_id) 作为合并键
    let mut fused: HashMap<(String, String), (MemoryHit, f64)> =
        HashMap::with_capacity(lexical.len() + vector.len());

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
