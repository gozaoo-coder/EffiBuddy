//! edit_file 工具的编辑历史存储
//!
//! 记录每次 `edit_file` / `edit_file_regex` 工具调用的完整快照（操作前后文件内容），
//! 供 `edit_revise`（查看 / 修订）与 `edit_undo`（撤回）工具使用。
//!
//! ## 设计要点
//! - **快照而非反向操作**：直接保存 `old_content` / `new_content` 字符串，
//!   撤回时把 `old_content` 写回文件即可，无需关心操作类型（行替换 / 正则替换）
//! - **进程内内存存储**：不持久化到磁盘，重启后历史清空（编辑操作本身已落盘）
//! - **容量限制**：最多 `MAX_HISTORY` 条，超出丢弃最老的（FIFO）
//! - **op_id 单调递增**：全局唯一，进程内有效；用 `u64` 足够长生命周期
//! - **线程安全**：通过 `Arc<RwLock<EditHistory>>` 共享给多个工具
//!   - 锁临界区极短：仅做 Vec push / 查找 / 删除，**不做 I/O**
//!   - 文件读写由调用方在锁外完成
//! - **撤回语义**：撤回 `op_id` 时，要求它是该文件的最新操作（无更晚记录覆盖同文件）；
//!   撤回成功后从历史中移除该记录。这样保证历史中所有记录的 `old_content` 始终
//!   对应"当前文件状态应用该操作前的快照"

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// 历史记录上限（FIFO，超出丢弃最老）
pub(crate) const MAX_HISTORY: usize = 200;

/// 编辑记录类型（用于报告展示）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditRecordKind {
    /// 按行号编辑（edit_file）
    LineEdit,
    /// 正则表达式编辑（edit_file_regex）
    RegexEdit,
    /// 修订重做（edit_revise 触发的新记录）
    Revised,
}

impl EditRecordKind {
    /// 中文标签（报告 / 前端展示用）
    #[inline]
    pub fn label(&self) -> &'static str {
        match self {
            EditRecordKind::LineEdit => "行号编辑",
            EditRecordKind::RegexEdit => "正则编辑",
            EditRecordKind::Revised => "修订重做",
        }
    }
}

/// 编辑操作参数（用于 edit_revise patch 重新执行）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EditOpParams {
    /// 行号编辑：原始操作列表（每个元素对应一次 line edit）
    Line { ops: Vec<LineEditParams> },
    /// 正则编辑：原始 pattern / 选项（replacement 由 patch 时的 new_text 提供）
    Regex {
        pattern: String,
        replacement: String,
        global: bool,
        multiline: bool,
        case_sensitive: bool,
    },
}

/// 行号编辑的单个操作参数（与 [`super::types::EditOp`] 对齐，但独立存储）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineEditParams {
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub insert_before: Option<usize>,
    pub text: String,
}

/// 单条编辑历史记录
///
/// 字段按大小降序：String(24B) == PathBuf(24B) > i64(8B) == u64(8B) > u32(4B)
/// > EditRecordKind(1B)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditRecord {
    /// 操作唯一 id（全局单调递增，1 开始）
    pub op_id: u64,
    /// 操作发生时间（Unix 秒）
    pub timestamp: i64,
    /// 受影响的文件绝对路径
    pub file_path: PathBuf,
    /// 操作前文件完整内容
    pub old_content: String,
    /// 操作后文件完整内容
    pub new_content: String,
    /// 人类可读的简要描述（如"替换第 10-15 行 / 正则替换 3 处"）
    pub summary: String,
    /// 操作类型
    pub kind: EditRecordKind,
    /// 受影响的行数（替换/插入行数；正则替换为命中数）
    pub lines_changed: u32,
    /// 原始操作参数（用于 edit_revise patch 重新执行）
    pub params: EditOpParams,
}

/// 编辑历史存储
///
/// 通过 `Arc<RwLock<EditHistory>>` 在多个 edit 工具间共享。
/// 锁内只做记录操作（push / 查找 / 删除），**不做文件 I/O**。
#[derive(Debug, Default)]
pub struct EditHistory {
    /// 历史记录（按 op_id 升序，新记录 push 到末尾）
    records: Vec<EditRecord>,
    /// 下一个 op_id（单调递增，从 1 开始）
    next_id: u64,
}

impl EditHistory {
    /// 创建空的历史存储
    pub fn new() -> Self {
        Self {
            records: Vec::with_capacity(MAX_HISTORY),
            next_id: 1,
        }
    }

    /// 追加一条编辑记录，返回分配的 op_id
    ///
    /// 超出 `MAX_HISTORY` 时丢弃最老的记录（FIFO）。
    pub fn record(&mut self, mut rec: EditRecord) -> u64 {
        let op_id = self.next_id;
        rec.op_id = op_id;
        self.next_id += 1;
        self.records.push(rec);
        if self.records.len() > MAX_HISTORY {
            // FIFO 淘汰：remove(0) 在 200 条规模下开销可忽略
            self.records.remove(0);
        }
        op_id
    }

    /// 按 op_id 查找记录（不可变借用）
    pub fn get(&self, op_id: u64) -> Option<&EditRecord> {
        // records 按 op_id 升序，可用二分查找
        self.records
            .binary_search_by_key(&op_id, |r| r.op_id)
            .ok()
            .map(|i| &self.records[i])
    }

    /// 列出最近 N 条记录（按 op_id 降序，最新的在前）
    pub fn list_recent(&self, limit: usize) -> Vec<&EditRecord> {
        let n = limit.min(self.records.len());
        self.records.iter().rev().take(n).collect()
    }

    /// 检查 op_id 是否为指定文件的最新编辑操作
    ///
    /// 用于撤回前的安全检查：若该文件有更晚的编辑，撤回会破坏后续操作的 old_content。
    pub fn is_latest_for_file(&self, op_id: u64, file_path: &std::path::Path) -> bool {
        // 反向遍历找该文件的最新操作
        for r in self.records.iter().rev() {
            if r.file_path == file_path {
                return r.op_id == op_id;
            }
        }
        false
    }

    /// 移除并返回指定 op_id 的记录（仅当它是该文件的最新操作）
    ///
    /// 返回被移除的记录，调用方据此把 `old_content` 写回文件完成撤回。
    /// 若 op_id 不存在或不是该文件最新操作，返回 None。
    pub fn remove_for_undo(&mut self, op_id: u64) -> Option<EditRecord> {
        let idx = self
            .records
            .binary_search_by_key(&op_id, |r| r.op_id)
            .ok()?;
        let file_path = self.records[idx].file_path.clone();
        if !self.is_latest_for_file(op_id, &file_path) {
            return None;
        }
        Some(self.records.remove(idx))
    }

    /// 替换指定 op_id 的记录内容（用于 edit_revise 重做）
    ///
    /// 保留原 op_id 与 timestamp，更新 old / new_content / summary / lines_changed，
    /// kind 标记为 `Revised`。要求该 op_id 仍是该文件的最新操作（同 undo 安全检查）。
    pub fn replace_record(
        &mut self,
        op_id: u64,
        old_content: String,
        new_content: String,
        summary: String,
        lines_changed: u32,
    ) -> Option<&EditRecord> {
        let idx = self
            .records
            .binary_search_by_key(&op_id, |r| r.op_id)
            .ok()?;
        if !self.is_latest_for_file(op_id, &self.records[idx].file_path) {
            return None;
        }
        let rec = &mut self.records[idx];
        rec.old_content = old_content;
        rec.new_content = new_content;
        rec.summary = summary;
        rec.lines_changed = lines_changed;
        rec.kind = EditRecordKind::Revised;
        Some(&self.records[idx])
    }

    /// 当前历史记录数
    #[inline]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// 共享编辑历史句柄类型
pub type EditHistoryHandle = Arc<RwLock<EditHistory>>;

/// 创建新的共享编辑历史
#[inline]
pub fn new_shared_history() -> EditHistoryHandle {
    Arc::new(RwLock::new(EditHistory::new()))
}

/// 工具内部用：记录一次编辑并返回 op_id（锁内不持有任何 I/O）
///
/// 调用方在完成文件写入后调用此函数，锁内仅做 Vec push 与 op_id 分配。
pub(crate) async fn record_edit(
    history: &EditHistoryHandle,
    file_path: PathBuf,
    old_content: String,
    new_content: String,
    summary: String,
    kind: EditRecordKind,
    lines_changed: u32,
    params: EditOpParams,
) -> u64 {
    let mut h = history.write().await;
    h.record(EditRecord {
        op_id: 0, // 由 record() 分配
        timestamp: Utc::now().timestamp(),
        file_path,
        old_content,
        new_content,
        summary,
        kind,
        lines_changed,
        params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn make_rec(file: &str, old: &str, new: &str) -> EditRecord {
        EditRecord {
            op_id: 0,
            timestamp: 0,
            file_path: PathBuf::from(file),
            old_content: old.to_string(),
            new_content: new.to_string(),
            summary: "test".to_string(),
            kind: EditRecordKind::LineEdit,
            lines_changed: 1,
            params: EditOpParams::Line { ops: vec![] },
        }
    }

    #[test]
    fn record_assigns_increasing_ids() {
        let mut h = EditHistory::new();
        let id1 = h.record(make_rec("a.txt", "x", "y"));
        let id2 = h.record(make_rec("a.txt", "y", "z"));
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn get_by_id() {
        let mut h = EditHistory::new();
        let id = h.record(make_rec("a.txt", "x", "y"));
        let r = h.get(id).unwrap();
        assert_eq!(r.file_path, Path::new("a.txt"));
        assert_eq!(r.old_content, "x");
    }

    #[test]
    fn list_recent_returns_latest_first() {
        let mut h = EditHistory::new();
        h.record(make_rec("a.txt", "1", "2"));
        h.record(make_rec("b.txt", "3", "4"));
        h.record(make_rec("c.txt", "5", "6"));
        let recent = h.list_recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].file_path, Path::new("c.txt"));
        assert_eq!(recent[1].file_path, Path::new("b.txt"));
    }

    #[test]
    fn is_latest_for_file_detects_older() {
        let mut h = EditHistory::new();
        let id1 = h.record(make_rec("a.txt", "1", "2"));
        let _id2 = h.record(make_rec("a.txt", "2", "3"));
        let _id3 = h.record(make_rec("b.txt", "x", "y"));
        // a.txt 的最新操作是 id2，不是 id1
        assert!(!h.is_latest_for_file(id1, Path::new("a.txt")));
        let id2 = id1 + 1;
        assert!(h.is_latest_for_file(id2, Path::new("a.txt")));
    }

    #[test]
    fn remove_for_undo_only_latest() {
        let mut h = EditHistory::new();
        let id1 = h.record(make_rec("a.txt", "1", "2"));
        let _id2 = h.record(make_rec("a.txt", "2", "3"));
        // id1 不是 a.txt 的最新操作，应拒绝
        assert!(h.remove_for_undo(id1).is_none());
        // id2 是最新操作，应允许
        let id2 = id1 + 1;
        let removed = h.remove_for_undo(id2);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().op_id, id2);
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn replace_record_only_latest() {
        let mut h = EditHistory::new();
        let id1 = h.record(make_rec("a.txt", "1", "2"));
        let _id2 = h.record(make_rec("a.txt", "2", "3"));
        // id1 不是最新，应失败
        assert!(h
            .replace_record(id1, "1".into(), "X".into(), "rev".into(), 1)
            .is_none());
        // id2 是最新，应成功
        let id2 = id1 + 1;
        let r = h.replace_record(id2, "2".into(), "Y".into(), "rev".into(), 1);
        assert!(r.is_some());
        let r = r.unwrap();
        assert_eq!(r.new_content, "Y");
        assert_eq!(r.kind, EditRecordKind::Revised);
    }
}
