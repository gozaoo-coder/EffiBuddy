//! `EditReviseTool`：查看 / 修订历史编辑操作
//!
//! 提供 3 种 action：
//! - `view`：查看指定 op_id 的详情（原内容、新内容、行号变化、操作类型、时间戳、摘要）
//! - `list`：列出最近 N 条历史操作摘要
//! - `patch`：用新 text 重做指定 op_id（先撤回该操作恢复 old_content，再用新 text
//!   在原位置重新执行；要求该 op_id 是该文件的最新操作）
//!
//! ## patch 语义
//! patch 会**删除原 op_id 记录**并产生新 op_id（由重新执行时的 record_edit 分配）。
//! AI 需要用新 op_id 撤回后续操作。这避免了"原地修改"导致的快照不一致问题。
//!
//! ## 安全检查
//! - view / list 只读，无副作用
//! - patch 要求 op_id 是该文件的最新操作（否则拒绝，提示先 undo 后续操作）
//! - patch 在锁外做文件 I/O：先写回 old_content（撤回），再删除记录，再重新执行

use std::path::PathBuf;

use rig_core::tool::Tool;
use tokio::fs;

use super::history::{EditHistoryHandle, EditOpParams, EditRecord};
use super::regex_tool::EditFileRegexTool;
use super::tool::EditFileTool;
use super::types::{EditFileArgs, EditFileRegexArgs, EditReviseArgs};

/// list 默认条数
const DEFAULT_LIST_LIMIT: usize = 10;
/// list 最大条数
const MAX_LIST_LIMIT: usize = 50;
/// view 单侧内容最大展示字符数（超出截断）
const MAX_VIEW_CONTENT_CHARS: usize = 2000;

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("edit_revise error: {0}")]
pub struct EditReviseError(String);

/// 编辑历史查看 / 修订工具
///
/// `history` 为必填句柄（无历史则该工具无意义，不应注册）。
/// `cwd` 为可选工作区，仅在 patch 重新执行时用于解析相对路径（一般与原 edit 一致）。
pub struct EditReviseTool {
    cwd: Option<PathBuf>,
    history: EditHistoryHandle,
}

impl EditReviseTool {
    pub fn new(history: EditHistoryHandle) -> Self {
        Self { cwd: None, history }
    }

    pub fn with_cwd(cwd: PathBuf, history: EditHistoryHandle) -> Self {
        Self {
            cwd: Some(cwd),
            history,
        }
    }
}

impl Tool for EditReviseTool {
    const NAME: &'static str = "edit_revise";

    type Error = EditReviseError;
    type Args = EditReviseArgs;
    type Output = String;

    fn description(&self) -> String {
        "查看 / 修订历史编辑操作（op_id 由 edit_file / edit_file_regex 成功操作返回）。\
         action：view=查看指定 op_id 详情；list=列出最近 N 条摘要（默认 10，最大 50）；\
         patch=用 new_text 重做指定 op_id（要求它是该文件最新操作）。\
         patch 会删除原 op_id 并产生新 op_id。推荐 XML 传参免转义。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["view", "list", "patch"], "description": "操作类型：view / list / patch" },
                "op_id": { "type": "integer", "description": "目标 op_id（view / patch 必填）" },
                "limit": { "type": "integer", "description": "list 最大条数，默认 10，最大 50", "default": 10 },
                "new_text": { "type": "string", "description": "patch 的新文本，推荐 <content> 包裹" }
            },
            "required": ["action"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let action = args.action.as_str();
        match action {
            "view" => self.action_view(args.op_id).await,
            "list" => self.action_list(args.limit).await,
            "patch" => self.action_patch(args.op_id, args.new_text).await,
            other => Err(EditReviseError(format!(
                "未知 action `{other}`：支持 view / list / patch"
            ))),
        }
    }
}

impl EditReviseTool {
    async fn action_view(&self, op_id: Option<u64>) -> Result<String, EditReviseError> {
        let op_id = op_id.ok_or_else(|| EditReviseError("view 需要 op_id 参数".into()))?;
        let h = self.history.read().await;
        let rec = h.get(op_id).ok_or_else(|| {
            EditReviseError(format!(
                "op_id={op_id} 不存在（可能已撤回 / patch 删除 / 超出历史容量 {}）",
                super::history::MAX_HISTORY
            ))
        })?;
        Ok(format_record_detail(rec))
    }

    async fn action_list(&self, limit: Option<usize>) -> Result<String, EditReviseError> {
        let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT).min(MAX_LIST_LIMIT);
        let h = self.history.read().await;
        let recent = h.list_recent(limit);
        if recent.is_empty() {
            return Ok("编辑历史为空（无已记录的 edit 操作）".to_string());
        }
        let mut out = String::with_capacity(512);
        out.push_str(&format!("最近 {} 条编辑历史：\n", recent.len()));
        for r in recent {
            out.push_str(&format!(
                "- op_id={} [{}] {}（{} 行变化）{}\n",
                r.op_id,
                r.kind.label(),
                r.summary,
                r.lines_changed,
                format_timestamp(r.timestamp),
            ));
        }
        out.push_str(
            "\n[提示：edit_revise(action=view, op_id=N) 查看详情；edit_undo(op_id=N) 撤回]",
        );
        Ok(out)
    }

    async fn action_patch(
        &self,
        op_id: Option<u64>,
        new_text: Option<String>,
    ) -> Result<String, EditReviseError> {
        let op_id = op_id.ok_or_else(|| EditReviseError("patch 需要 op_id 参数".into()))?;
        let new_text = new_text.ok_or_else(|| EditReviseError("patch 需要 new_text 参数".into()))?;

        // 1. 读取原记录 + 检查 is_latest_for_file（读锁内完成）
        let rec = {
            let h = self.history.read().await;
            let r = h.get(op_id).ok_or_else(|| {
                EditReviseError(format!("op_id={op_id} 不存在"))
            })?;
            if !h.is_latest_for_file(op_id, &r.file_path) {
                return Err(EditReviseError(format!(
                    "op_id={op_id} 不是文件 [{}] 的最新编辑操作，\
                     无法直接 patch。请先 edit_undo 后续操作，或先 view 查看详情",
                    r.file_path.display()
                )));
            }
            r.clone()
        };

        // 2. 撤回：把 old_content 写回文件（锁外 I/O）
        fs::write(&rec.file_path, rec.old_content.as_bytes())
            .await
            .map_err(|e| {
                EditReviseError(format!("撤回写文件失败 [{}]: {e}", rec.file_path.display()))
            })?;

        // 3. 从历史中删除原 op_id（已确认是最新，必能删除；锁内仅 Vec::remove）
        {
            let mut h = self.history.write().await;
            // remove_for_undo 内部会再次检查 is_latest（防止并发），返回 None 时记录已不存在
            let _ = h.remove_for_undo(op_id);
        }

        // 4. 用 new_text 重新执行（调用 EditFileTool / EditFileRegexTool，自动 record_edit 产生新 op_id）
        let path_str = rec.file_path.to_string_lossy().into_owned();
        let new_report = match &rec.params {
            EditOpParams::Line { ops } => {
                if ops.is_empty() {
                    return Err(EditReviseError(format!(
                        "op_id={op_id} 的 LineEdit 参数为空，无法 patch"
                    )));
                }
                let tool = match &self.cwd {
                    Some(p) => {
                        EditFileTool::with_cwd(p.clone()).with_history(self.history.clone())
                    }
                    None => EditFileTool::new().with_history(self.history.clone()),
                };
                // 用 new_text 替换所有 ops 的 text（适用于单操作场景；多操作时
                // AI 应理解所有 text 都被同一 new_text 替换）
                let new_edits: Vec<crate::tools::EditOp> = ops
                    .iter()
                    .map(|p| crate::tools::EditOp {
                        start_line: p.start_line,
                        end_line: p.end_line,
                        insert_before: p.insert_before,
                        text: new_text.clone(),
                    })
                    .collect();
                tool.call(EditFileArgs {
                    path: path_str,
                    edits: new_edits,
                    dry_run: None,
                    diff_context: None,
                })
                .await
                .map_err(|e| EditReviseError(format!("patch 重新执行失败: {e}")))?
            }
            EditOpParams::Regex {
                pattern,
                global,
                multiline,
                case_sensitive,
                ..
            } => {
                let tool = match &self.cwd {
                    Some(p) => {
                        EditFileRegexTool::with_cwd(p.clone()).with_history(self.history.clone())
                    }
                    None => EditFileRegexTool::new().with_history(self.history.clone()),
                };
                tool.call(EditFileRegexArgs {
                    path: path_str,
                    pattern: pattern.clone(),
                    replacement: new_text,
                    global: Some(*global),
                    multiline: Some(*multiline),
                    case_sensitive: Some(*case_sensitive),
                    dry_run: None,
                    diff_context: None,
                })
                .await
                .map_err(|e| EditReviseError(format!("patch 重新执行失败: {e}")))?
            }
        };

        Ok(format!(
            "已用新 text 重做 op_id={op_id}（原 op_id 已删除，新 op_id 见下）：\n{new_report}"
        ))
    }
}

/// 格式化单条记录详情（用于 view action）
fn format_record_detail(rec: &EditRecord) -> String {
    let mut out = String::with_capacity(512);
    out.push_str(&format!("op_id={} 详情：\n", rec.op_id));
    out.push_str(&format!("- 类型: {}\n", rec.kind.label()));
    out.push_str(&format!("- 文件: {}\n", rec.file_path.display()));
    out.push_str(&format!("- 时间: {}\n", format_timestamp(rec.timestamp)));
    out.push_str(&format!("- 摘要: {}\n", rec.summary));
    out.push_str(&format!("- 行变化: {} 行\n", rec.lines_changed));
    out.push_str(&format!(
        "- 操作前内容（{} 字符）:\n{}\n",
        rec.old_content.len(),
        truncate_content(&rec.old_content, MAX_VIEW_CONTENT_CHARS)
    ));
    out.push_str(&format!(
        "- 操作后内容（{} 字符）:\n{}\n",
        rec.new_content.len(),
        truncate_content(&rec.new_content, MAX_VIEW_CONTENT_CHARS)
    ));
    out.push_str(
        "\n[可调用 edit_undo(op_id=...) 撤回此操作；edit_revise(action=patch, op_id=..., new_text=...) 用新 text 重做]",
    );
    out
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

/// 格式化 Unix 时间戳为可读字符串
fn format_timestamp(ts: i64) -> String {
    use chrono::DateTime;
    DateTime::<chrono::Utc>::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| ts.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::edit_file::history::new_shared_history;
    use crate::tools::edit_file::tests_common::{read, setup_file, tmp_dir};
    use crate::tools::{EditFileTool, EditOp};
    use rig_core::tool::Tool;

    #[tokio::test]
    async fn list_empty_history() {
        let history = new_shared_history();
        let tool = EditReviseTool::new(history);
        let r = tool
            .call(EditReviseArgs {
                action: "list".to_string(),
                op_id: None,
                limit: None,
                new_text: None,
            })
            .await
            .unwrap();
        assert!(r.contains("编辑历史为空"));
    }

    #[tokio::test]
    async fn view_and_list_after_edit() {
        let dir = tmp_dir();
        setup_file(&dir, "a.txt", "line1\nline2\nline3\n").await;
        let history = new_shared_history();
        let edit_tool = EditFileTool::with_cwd(dir.clone()).with_history(history.clone());
        // 执行一次 edit
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
        // 提取 op_id
        let op_id = extract_op_id(&r).expect("报告应含 op_id");

        let revise_tool = EditReviseTool::with_cwd(dir.clone(), history);
        // view
        let v = revise_tool
            .call(EditReviseArgs {
                action: "view".to_string(),
                op_id: Some(op_id),
                limit: None,
                new_text: None,
            })
            .await
            .unwrap();
        assert!(v.contains("op_id=1"));
        assert!(v.contains("CHANGED"));
        assert!(v.contains("line2"));

        // list
        let l = revise_tool
            .call(EditReviseArgs {
                action: "list".to_string(),
                op_id: None,
                limit: None,
                new_text: None,
            })
            .await
            .unwrap();
        assert!(l.contains("op_id=1"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn patch_line_edit_replaces_text() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "line1\nline2\nline3\n").await;
        let history = new_shared_history();
        let edit_tool = EditFileTool::with_cwd(dir.clone()).with_history(history.clone());
        // 第一次 edit：替换第 2 行为 "OLD"
        let r = edit_tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(2),
                    end_line: None,
                    insert_before: None,
                    text: "OLD".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert_eq!(read(&p).await, "line1\nOLD\nline3\n");
        let op_id = extract_op_id(&r).unwrap();

        // patch：用 "NEW" 重做
        let revise_tool = EditReviseTool::with_cwd(dir.clone(), history);
        let r = revise_tool
            .call(EditReviseArgs {
                action: "patch".to_string(),
                op_id: Some(op_id),
                limit: None,
                new_text: Some("NEW".to_string()),
            })
            .await
            .unwrap();
        assert!(r.contains("已用新 text 重做"));
        // 文件应变为 line1\nNEW\nline3\n（先撤回到 line1\nline2\nline3\n，再用 NEW 替换第 2 行）
        assert_eq!(read(&p).await, "line1\nNEW\nline3\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn patch_rejects_non_latest_op() {
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

        // patch op_id_1 应失败（不是最新操作）
        let revise_tool = EditReviseTool::with_cwd(dir.clone(), history);
        let r = revise_tool
            .call(EditReviseArgs {
                action: "patch".to_string(),
                op_id: Some(op_id_1),
                limit: None,
                new_text: Some("Z".to_string()),
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("不是文件"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn view_nonexistent_op_returns_error() {
        let history = new_shared_history();
        let tool = EditReviseTool::new(history);
        let r = tool
            .call(EditReviseArgs {
                action: "view".to_string(),
                op_id: Some(999),
                limit: None,
                new_text: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("不存在"));
    }

    /// 从报告字符串中提取 op_id（格式 "[op_id=N]"）
    fn extract_op_id(report: &str) -> Option<u64> {
        let marker = "[op_id=";
        let start = report.find(marker)? + marker.len();
        let end = report[start..].find(']')? + start;
        report[start..end].parse().ok()
    }
}
