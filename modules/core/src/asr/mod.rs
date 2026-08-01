//! ASR（语音转写）持久化存储与摘要 RAG 索引。
//!
//! 按职责拆分为子模块，避免"上帝文件"：
//! - [`types`]：数据结构（AsrRecord / AsrStatus / AsrSource）
//! - [`store`]：AsrStore 持久化（JSON 文件 + 每记录独立锁 + transcript 分离存储）
//! - [`index`]：AsrSummaryIndex（接入 MemoryIndex 做 RAG 索引/检索的适配层）
//!
//! # 并发与性能（对齐 user_rules）
//!
//! - AsrStore 使用每记录独立锁 + 全局文件锁，临界区极短（仅 records.json 读-改-写）
//! - search 在锁外执行（load_all 后释放锁，迭代器链过滤）
//! - 所有 IO 使用 `tokio::fs` 异步
//! - AsrSummaryIndex 通过 tombstone 集合实现 remove，不修改 MemoryIndex 原有实现

mod index;
mod store;
mod types;

pub use index::{AsrSummaryHit, AsrSummaryIndex};
pub use store::{AsrSearchQuery, AsrStore};
pub use types::{AsrRecord, AsrSource, AsrStatus};
