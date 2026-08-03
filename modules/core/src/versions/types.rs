//! git 风格会话版本控制 —— 数据结构
//!
//! 设计对齐 git 语义：
//! - **Commit**：内容寻址（SHA-256）+ 父指针，形成不可变的历史链（DAG）。
//!   每个 commit 保存一份有序 `message_ids` 快照 + 指向消息池的引用，消息本体
//!   内容寻址存放一次（追加去重，类似 git 的 blob/tree）。
//! - **Refs**（引用）：名称 → 提交 hash。命名约定：
//!   - `main`：默认分支
//!   - `branch-<ts>`：用户开启的分支
//!   - `temp-<ts>`：用户保存的临时版本书签
//!   - `chkpt-<ts>`：破坏性操作前自动保存的撤销检查点（可随时回到操作前）
//! - **HEAD**：当前检出的分支名。工作区（会话 JSON 文件）始终等于 HEAD 提交快照。
//!
//! 所有结构体 `Serialize`/`Deserialize`，经 Tauri 命令边界直接透传给前端。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Message;

/// 提交类型（`serde(rename_all = "snake_case")`，与前端 `VersionKind` 对齐）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitKind {
    /// 追加消息（发送/流式落盘/定时任务/系统消息等每次 append 自动生成）
    Append,
    /// 开启分支
    Branch,
    /// 保存临时版本
    TempSave,
    /// 回溯版本（重置 HEAD 到某条消息的提交）
    Rollback,
    /// 撤回至此消息前（重置 HEAD 到该消息提交的父提交）
    Undo,
}

impl CommitKind {
    /// 中文字面量（供 note/前端展示）
    pub fn label(self) -> &'static str {
        match self {
            CommitKind::Append => "新消息",
            CommitKind::Branch => "开启分支",
            CommitKind::TempSave => "临时版本",
            CommitKind::Rollback => "回溯版本",
            CommitKind::Undo => "撤回",
        }
    }
}

/// 一次提交（git commit 语义）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    /// 内容寻址 hash（SHA-256，16 进制前缀）
    pub hash: String,
    /// 父提交 hash；`None` 表示根提交
    pub parent: Option<String>,
    pub kind: CommitKind,
    /// 人类可读说明（如"新消息"、"分支起点"、"撤回至 m5"）
    pub note: String,
    pub created_at: u64,
    /// 快照最后一条消息 id（空快照为 ""）
    pub head_message_id: String,
    /// 快照包含的消息 id 有序列表
    pub message_ids: Vec<String>,
    /// 快照消息总数
    pub message_count: usize,
}

impl Commit {
    /// 解析快照为完整消息列表（消息本体从池中按 id 取回）
    pub fn resolve(&self, pool: &BTreeMap<String, Message>) -> Vec<Message> {
        self.message_ids
            .iter()
            .filter_map(|id| pool.get(id).cloned())
            .collect()
    }
}

/// 单个会话的版本仓库（每会话一个 `repo.json`）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionRepo {
    /// hash → Commit：全部历史，永不删除（对象库）
    pub commits: BTreeMap<String, Commit>,
    /// 消息池：id → Message（内容寻址，追加去重，所有提交共享）
    pub messages: BTreeMap<String, Message>,
    /// 引用：名称 → 提交 hash（git refs，可移动/删除）
    pub refs: BTreeMap<String, String>,
    /// 临时版本的用户备注：temp 引用名 → 备注（旧仓库无此字段时为空）
    #[serde(default)]
    pub temp_notes: BTreeMap<String, String>,
    /// 当前检出的分支名（HEAD）
    pub head: String,
}

/// 引用摘要（版本列表展示用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefSummary {
    pub name: String,
    /// "main" | "branch" | "temp" | "checkpoint"
    pub kind: String,
    pub hash: String,
    pub created_at: u64,
    pub message_count: usize,
    pub head_message_id: String,
    pub note: String,
}

/// 提交摘要（当前分支历史链展示用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitSummary {
    pub hash: String,
    pub kind: CommitKind,
    pub note: String,
    pub created_at: u64,
    pub head_message_id: String,
    pub message_count: usize,
    /// 是否为当前 HEAD
    pub is_head: bool,
}

/// 会话版本列表（前端「版本管理」面板展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionList {
    /// 当前分支名
    pub head: String,
    pub refs: Vec<RefSummary>,
    /// 当前分支从 HEAD 回溯的提交链（新→旧）
    pub commits: Vec<CommitSummary>,
}

/// 版本操作结果（开启分支/回溯/撤回/检出后返回给前端）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionOpResult {
    /// 操作后 HEAD 提交 hash
    pub head_hash: String,
    pub kind: CommitKind,
    /// 当前分支名
    pub branch: String,
    /// 操作说明（note）
    pub note: String,
    /// 恢复后的完整消息列表（前端据此刷新消息区）
    pub messages: Vec<Message>,
}

/// 引用名解析结果：分支名 → 归属类别
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Main,
    Branch,
    Temp,
    Checkpoint,
}

impl RefKind {
    /// 由引用名判定类别
    pub fn of(name: &str) -> RefKind {
        if name == "main" {
            RefKind::Main
        } else if name.starts_with("branch-") {
            RefKind::Branch
        } else if name.starts_with("temp-") {
            RefKind::Temp
        } else {
            RefKind::Checkpoint
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RefKind::Main => "main",
            RefKind::Branch => "branch",
            RefKind::Temp => "temp",
            RefKind::Checkpoint => "checkpoint",
        }
    }
}
