//! `EditUndoTool`：撤回指定 op_id 的编辑操作
//!
//! 把文件内容恢复到指定 op_id 操作前的状态（写回 `old_content`）。
//!
//! ## 安全检查
//! - op_id 必须存在
//! - op_id 必须是该文件的最新编辑操作（否则拒绝，提示先 undo 后续操作）
//!
//! ## dry_run
//! `dry_run=true` 时仅返回撤回后将恢复的内容预览，不写入磁盘。
//!
//! ## 锁与 I/O 分离
//! - 读锁：查找记录 + is_latest 检查
//! - 锁外：文件 I/O（写回 old_content）
//! - 写锁：从历史中移除该记录
//!
//! 顺序保证：单 agent 顺序调用工具，不会并发；多 agent 并发场景下，文件 I/O 在锁外
//! 可能有竞态，但每次 undo 都验证 is_latest，最坏情况是撤回失败需重试。

use std::path::PathBuf;

use rig_core::tool::Tool;
use tokio::fs;

use super::history::{EditHistoryHandle, MAX_HISTORY};
use super::types::EditUndoArgs;

/// 撤回预览时展示的内容最大字符数
const MAX_UNDO_PREVIEW_CHARS: usize = 2000;

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("edit_undo error: {0}")]
pub struct EditUndoError(String);

/// 编辑撤回工具
///
/// `history` 为必填句柄（无历史则该工具无意义，不应注册）。
pub struct EditUndoTool {
    history: EditHistoryHandle,
    /// 保留 cwd 字段以便未来扩展（当前撤回直接用记录中的绝对路径，不需要 cwd）
    #[allow(dead_code)]
    cwd: Option<PathBuf>,
}

impl EditUndoTool {
    pub fn new(history: EditHistoryHandle) -> Self {
        Self {
            history,
            cwd: None,
        }
    }

    pub fn with_cwd(cwd: PathBuf, history: EditHistoryHandle) -> Self {
        Self {
            history,
            cwd: Some(cwd),
        }
    }
}

impl Tool for EditUndoTool {
    const NAME: &'static str = "edit_undo";

    type Error = EditUndoError;
    type Args = EditUndoArgs;
    type Output = String;

    fn description(&self) -> String {
        "撤回指定 op_id 的编辑操作：把文件内容恢复到该操作前状态。\
         要求 op_id 是该文件最新编辑（有更晚操作时先撤回后续）。\
         op_id：edit_file / edit_file_regex / edit_revise 返回；\
         dry_run=true 仅预览恢复内容不写盘。撤回后 op_id 从历史删除。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "op_id": { "type": "integer", "description": "要撤回的 op_id（必填）" },
                "dry_run": { "type": "boolean", "description": "true=仅预览不写盘", "default": false }
            },
            "required": ["op_id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let op_id = args.op_id;
        let dry_run = args.dry_run.unwrap_or(false);

        // 1. 读锁：查找记录 + is_latest 检查
        let rec = {
            let h = self.history.read().await;
            let r = h.get(op_id).ok_or_else(|| {
                EditUndoError(format!(
                    "op_id={op_id} 不存在（可能已撤回 / patch 删除 / 超出历史容量 {MAX_HISTORY}）"
                ))
            })?;
            if !h.is_latest_for_file(op_id, &r.file_path) {
                return Err(EditUndoError(format!(
                    "op_id={op_id} 不是文件 [{}] 的最新编辑操作，无法直接撤回。\
                     请先撤回对该文件的更晚操作（用 edit_revise action=list 查看历史）",
                    r.file_path.display()
                )));
            }
            r.clone()
        };

        // 2. dry_run：仅预览
        if dry_run {
            return Ok(format!(
                "[预览] 撤回 op_id={} 将恢复文件 [{}] 到操作前状态（{} 字符 → {} 字符）:\n{}",
                op_id,
                rec.file_path.display(),
                rec.new_content.len(),
                rec.old_content.len(),
                truncate_content(&rec.old_content, MAX_UNDO_PREVIEW_CHARS)
            ));
        }

        // 3. 锁外：写回 old_content（撤回）
        fs::write(&rec.file_path, rec.old_content.as_bytes())
            .await
            .map_err(|e| {
                EditUndoError(format!("撤回写文件失败 [{}]: {e}", rec.file_path.display()))
            })?;

        // 4. 写锁：从历史中移除该记录
        let removed = {
            let mut h = self.history.write().await;
            h.remove_for_undo(op_id)
        };
        // remove_for_undo 内部会再次检查 is_latest（防并发），正常情况下必能移除
        if removed.is_none() {
            // 极端情况：并发导致 is_latest 变化，文件已写回但记录未删
            // 此时文件状态正确（已撤回），仅历史记录残留，不影响正确性
            tracing::warn!(
                "edit_undo: op_id={} 文件已撤回但历史记录未删除（可能并发修改）",
                op_id
            );
        }

        Ok(format!(
            "已撤回 op_id={}：文件 [{}] 已恢复到操作前状态（{} 字节 → {} 字节）。\n\
             该 op_id 已从历史中删除，无法再次撤回。",
            op_id,
            rec.file_path.display(),
            rec.new_content.len(),
            rec.old_content.len()
        ))
    }
}

/// 内容截断（按字符数，避免切到 UTF-8 边界）
fn truncate_content(content: &str, max_chars: usize) -> String {
    let chars: Vec<char> = content.chars().collect();
    if chars.len() > max_chars {
        let head: String = chars.iter().take(max_chars).collect();
        format!("{head}\n...（共 {} 字符，已截断）", chars.len())
    } else {
        content.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::edit_file::history::new_shared_history;
    use crate::tools::edit_file::tests_common::{read, setup_file, tmp_dir};
    use crate::tools::{EditFileArgs, EditFileTool, EditOp};
    use rig_core::tool::Tool;

    #[tokio::test]
    async fn undo_restores_file_content() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "line1\nline2\nline3\n").await;
        let history = new_shared_history();
        let edit_tool = EditFileTool::with_cwd(dir.clone()).with_history(history.clone());

        // 执行 edit：替换第 2 行
        let r = edit_tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(2),
                    end_line: None,
                    insert_before: None,
                    text: "CHANGED".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert_eq!(read(&p).await, "line1\nCHANGED\nline3\n");
        let op_id = extract_op_id(&r).unwrap();

        // undo
        let undo_tool = EditUndoTool::new(history);
        let r = undo_tool
            .call(EditUndoArgs {
                op_id,
                dry_run: None,
            })
            .await
            .unwrap();
        assert!(r.contains("已撤回"));
        // 文件应恢复到操作前
        assert_eq!(read(&p).await, "line1\nline2\nline3\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn undo_dry_run_does_not_write() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "a\nb\nc\n").await;
        let history = new_shared_history();
        let edit_tool = EditFileTool::with_cwd(dir.clone()).with_history(history.clone());

        let r = edit_tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(2),
                    end_line: None,
                    insert_before: None,
                    text: "B".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        let op_id = extract_op_id(&r).unwrap();
        assert_eq!(read(&p).await, "a\nB\nc\n");

        let undo_tool = EditUndoTool::new(history);
        let r = undo_tool
            .call(EditUndoArgs {
                op_id,
                dry_run: Some(true),
            })
            .await
            .unwrap();
        assert!(r.contains("[预览]"));
        // 文件未改变
        assert_eq!(read(&p).await, "a\nB\nc\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn undo_rejects_non_latest_op() {
        let dir = tmp_dir();
        setup_file(&dir, "a.txt", "a\nb\nc\n").await;
        let history = new_shared_history();
        let edit_tool = EditFileTool::with_cwd(dir.clone()).with_history(history.clone());

        // 两次 edit 同一文件
        let r1 = edit_tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(1),
                    end_line: None,
                    insert_before: None,
                    text: "X".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        let op_id_1 = extract_op_id(&r1).unwrap();
        let _ = edit_tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(2),
                    end_line: None,
                    insert_before: None,
                    text: "Y".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();

        // undo op_id_1 应失败
        let undo_tool = EditUndoTool::new(history);
        let r = undo_tool
            .call(EditUndoArgs {
                op_id: op_id_1,
                dry_run: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("不是文件"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn undo_nonexistent_op_returns_error() {
        let history = new_shared_history();
        let tool = EditUndoTool::new(history);
        let r = tool
            .call(EditUndoArgs {
                op_id: 999,
                dry_run: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("不存在"));
    }

    #[tokio::test]
    async fn undo_across_different_files_independent() {
        // 对 a.txt 编辑后，对 b.txt 编辑，再 undo b.txt 的操作应成功
        // （b.txt 的最新操作就是它本身，不影响 a.txt）
        let dir = tmp_dir();
        let pa = setup_file(&dir, "a.txt", "a1\na2\n").await;
        let pb = setup_file(&dir, "b.txt", "b1\nb2\n").await;
        let history = new_shared_history();
        let edit_tool = EditFileTool::with_cwd(dir.clone()).with_history(history.clone());

        let _ = edit_tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(1),
                    end_line: None,
                    insert_before: None,
                    text: "A1".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        let r_b = edit_tool
            .call(EditFileArgs {
                path: "b.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(1),
                    end_line: None,
                    insert_before: None,
                    text: "B1".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        let op_id_b = extract_op_id(&r_b).unwrap();
        assert_eq!(read(&pa).await, "A1\na2\n");
        assert_eq!(read(&pb).await, "B1\nb2\n");

        // undo b.txt 的操作应成功
        let undo_tool = EditUndoTool::new(history);
        let r = undo_tool
            .call(EditUndoArgs {
                op_id: op_id_b,
                dry_run: None,
            })
            .await;
        assert!(r.is_ok());
        // a.txt 不受影响
        assert_eq!(read(&pa).await, "A1\na2\n");
        // b.txt 恢复
        assert_eq!(read(&pb).await, "b1\nb2\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 从报告字符串中提取 op_id（格式 "[op_id=N]"）
    fn extract_op_id(report: &str) -> Option<u64> {
        let marker = "[op_id=";
        let start = report.find(marker)? + marker.len();
        let end = report[start..].find(']')? + start;
        report[start..end].parse().ok()
    }
}
