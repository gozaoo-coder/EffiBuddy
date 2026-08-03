//! git 风格会话版本控制
//!
//! 为会话消息历史提供类 git 的版本管理：提交链、分支、临时版本书签、
//! 回溯与撤回，配合 [`crate::storage::ConversationStore`] 使用。
//!
//! - [`types`]：数据结构（Commit / VersionRepo / RefSummary / VersionList 等）
//! - [`store`]：版本仓库存储（VersionStore）
//! - [`tests`]：版本控制行为单元测试
//!
//! 用法要点：
//! - 用 `ConversationStore::with_versions(...)` 创建启用了版本控制的会话存储，
//!   每次 `append_message` 自动产生一个 `Append` 提交；
//! - 版本操作（开分支/回溯/撤回/检出/保存临时版本）经 `ConversationStore` 的
//!   `version_*` 委托方法执行（在会话锁内同步更新仓库与工作区）。

pub mod store;
#[cfg(test)]
pub mod tests;
pub mod types;

pub use store::VersionStore;
pub use types::{
    Commit, CommitKind, CommitSummary, RefKind, RefSummary, VersionList, VersionOpResult,
    VersionRepo,
};
