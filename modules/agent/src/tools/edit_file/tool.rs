//! `EditFileTool` 及其 `Tool` trait 实现：读取文件 → 解析/校验操作 →
//! 执行 splice → 写回 → 委托 [`super::report`] 生成报告。
//!
//! 启用 `history`（`Arc<RwLock<EditHistory>>`）后，每次成功的非 dry_run 编辑
//! 会记录一条快照（操作前后文件完整内容），返回的报告中含 `op_id`，
//! 供 `edit_revise` / `edit_undo` 工具使用。

use std::path::PathBuf;

use rig_core::tool::Tool;
use tokio::fs;

use super::super::resolve_path;
use super::super::text_utils::extract_content;
use super::history::{EditHistoryHandle, EditOpParams, EditRecordKind, LineEditParams, record_edit};
use super::types::{EditFileArgs, EditKind, OpResult, ParsedOp};
use super::MAX_OPS;

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("edit_file error: {0}")]
pub struct EditFileError(String);

/// 文件编辑工具
///
/// `cwd` 为可选工作区：设置后相对路径以此为基准，未设置则依赖进程 cwd。
/// `history` 为可选编辑历史句柄：注入后每次成功编辑会记录快照，返回 op_id。
pub struct EditFileTool {
    cwd: Option<PathBuf>,
    history: Option<EditHistoryHandle>,
}

impl EditFileTool {
    pub fn new() -> Self {
        Self {
            cwd: None,
            history: None,
        }
    }

    /// 指定工作区目录，相对路径将 join 到此目录
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self {
            cwd: Some(cwd),
            history: None,
        }
    }

    /// 注入编辑历史句柄，启用 op_id 编号与撤回能力（链式调用）
    pub fn with_history(mut self, history: EditHistoryHandle) -> Self {
        self.history = Some(history);
        self
    }
}

impl Default for EditFileTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for EditFileTool {
    const NAME: &'static str = "edit_file";

    type Error = EditFileError;
    type Args = EditFileArgs;
    type Output = String;

    fn description(&self) -> String {
        let cwd_hint = self
            .cwd
            .as_ref()
            .map(|p| format!("当前工作区：{}（相对路径以此为准）", p.display()))
            .unwrap_or_else(|| "未设置工作区，相对路径依赖进程工作目录".to_string());
        let history_hint = if self.history.is_some() {
            "\n**编辑历史**：每次成功编辑会分配一个 op_id（在返回报告开头），\
             可用 edit_revise 查看/修订、edit_undo 撤回该操作。"
        } else {
            ""
        };
        format!(
            "**代码修改优先用正则**：当改动具有规律性（批量替换某函数调用、统一命名/格式、\
             \"所有 `println!` 调用\"、`TODO` 注释等），请**优先使用 `edit_file_regex`**\
             （正则匹配替换，支持首处/全部 + 捕获组引用），比手数行号更稳更快；\
             `edit_file` 用于需要精确控制行号的单点编辑（如插入/替换特定某几行、追加）。\n\n\
             按行号精确编辑本地文本文件，**不覆盖整文件**。与 read_file / search_file 配合：\
             先读取或搜索拿到行号，再替换指定行。\n\n\
             **操作模式**（edits 为操作数组，每个元素一种模式）：\n\
             1. 替换：start_line（+ 可选 end_line）+ text，把该行/该区间整段替换为 text；\
             空 text 表示删除该行\n\
             2. 插入：insert_before + text，在指定行之前插入新行；\
             insert_before = 总行数+1 等价于追加\n\
             3. 追加：不给任何行号字段，直接把 text 作为新行写入文件末尾（无需知道行数）\n\n\
              **规则**：行号一律 1-based 且指编辑前原文件；多个操作自动排序执行、区间不能重叠；\
              一次最多 {MAX_OPS} 个操作。text 推荐用 <content>...</content> 包裹避免转义（同 write_file）。\n\
              **可选参数**：dry_run=true 仅预览不写盘（大改前先确认安全，返回完整 diff 与行号变化）；\
              diff_context=N 控制变更明细上下文行数（默认 1，0 = 只显示变更行）。\n\
              **行数校准**：替换模式下若 text 行数与 (end_line - start_line + 1) 不一致，\
                 报告会明确标注声明行数与实际写入行数，便于核对的下次操作。\
                 路径不做沙箱限制（信任本地 agent 环境）。\n{cwd_hint}{history_hint}"
        )
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "文件路径（绝对或相对工作区），文件必须已存在"
                },
                "edits": {
                    "type": "array",
                      "description": "编辑操作列表，按行号自动排序执行；行号指编辑前原文件（1-based）。规律性批量改动请改用 edit_file_regex",
                    "items": {
                        "type": "object",
                        "properties": {
                            "start_line": {
                                "type": "integer",
                                "description": "替换模式起始行号（1-based，含）；不给任何行号 = 追加到文件末尾"
                            },
                            "end_line": {
                                "type": "integer",
                                "description": "替换模式结束行号（1-based，含）；缺省 = 只替换 start_line 一行"
                            },
                            "insert_before": {
                                "type": "integer",
                                "description": "插入模式：在指定行号之前插入 text；总行数+1 等价于追加到末尾"
                            },
                            "text": {
                                "type": "string",
                                "description": "目标文本，推荐 <content>...</content> 包裹避免转义"
                            }
                        },
                        "required": ["text"]
                    }
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "true = 仅预览：计算并返回完整报告（含 diff 与行号变化），不写入磁盘",
                    "default": false
                },
                "diff_context": {
                    "type": "integer",
                    "description": "变更明细中上下文行数（每侧），默认 1；0 = 只显示变更行本身",
                    "default": 1
                }
            },
            "required": ["path", "edits"]
        })
    }


    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let resolved = resolve_path(&args.path, self.cwd.as_deref());

        let dry_run = args.dry_run.unwrap_or(false);
        let diff_context = args.diff_context.unwrap_or(1).min(10);

        if args.edits.is_empty() {
            return Err(EditFileError(
                "edits 不能为空：请至少提供一个编辑操作（替换/插入/追加）".to_string(),
            ));
        }
        if args.edits.len() > MAX_OPS {
            return Err(EditFileError(format!(
                "edits 数量（{}）超过单次上限 {MAX_OPS}，请分批编辑",
                args.edits.len()
            )));
        }

        // 1. 读取原文件（必须是 UTF-8 文本，编辑二进制文件无意义）
        let bytes = fs::read(&resolved)
            .await
            .map_err(|e| {
                EditFileError(format!(
                    "读取文件失败 [{}]: {e}（若文件不存在，请先用 write_file 创建）",
                    resolved.display()
                ))
            })?;
        let content = String::from_utf8(bytes).map_err(|_| {
            EditFileError(format!(
                "文件不是有效 UTF-8 文本 [{}]，无法按行编辑",
                resolved.display()
            ))
        })?;
        // 保留原始内容用于历史快照（仅 non-dry_run 时记录）
        let old_content_snapshot = if !dry_run && self.history.is_some() {
            Some(content.clone())
        } else {
            None
        };

        // 2. 切行：保留 EOL 风格（\r\n vs \n）与"是否以换行结尾"
        let eol = if content.contains("\r\n") { "\r\n" } else { "\n" };
        let had_trailing_newline = content.ends_with('\n');
        // split('\n') 后：去掉末尾空元素；每行去掉行尾 \r（重拼时用 eol 还原）。
        // 空文件（""）直接得到 0 行，而不是 1 个空行。
        let mut lines: Vec<String> = if content.is_empty() {
            Vec::new()
        } else {
            content
                .split('\n')
                .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
                .collect()
        };
        if had_trailing_newline && lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        let line_count = lines.len();
        let old_count = line_count;

        // 3. 解析 + 校验每个操作（行号 1-based → 0-based）
        let mut ops: Vec<ParsedOp> = Vec::with_capacity(args.edits.len());
        for (i, op) in args.edits.iter().enumerate() {
            let text = extract_content(&op.text);
            if let Some(n) = op.insert_before {
                if n == 0 || n > line_count + 1 {
                    return Err(EditFileError(format!(
                        "第 {} 个操作：insert_before（{n}）超出范围（1..={}）",
                        i + 1,
                        line_count + 1
                    )));
                }
                ops.push(ParsedOp {
                    kind: EditKind::Insert { at: n - 1 },
                    text,
                    orig_pos: n,
                    is_append: n == line_count + 1,
                });
            } else if let Some(s) = op.start_line {
                if s == 0 || s > line_count {
                    return Err(EditFileError(format!(
                        "第 {} 个操作：start_line（{s}）超出范围（1..={line_count}）",
                        i + 1
                    )));
                }
                let e = op.end_line.unwrap_or(s);
                if e < s {
                    return Err(EditFileError(format!(
                        "第 {} 个操作：end_line（{e}）不能小于 start_line（{s}）",
                        i + 1
                    )));
                }
                if e > line_count {
                    return Err(EditFileError(format!(
                        "第 {} 个操作：end_line（{e}）超出文件总行数（{line_count}）",
                        i + 1
                    )));
                }
                ops.push(ParsedOp {
                    kind: EditKind::Replace { start: s - 1, end: e - 1 },
                    text,
                    orig_pos: s,
                    is_append: false,
                });
            } else {
                // 追加模式：写入文件末尾新行
                ops.push(ParsedOp {
                    kind: EditKind::Insert { at: line_count },
                    text,
                    orig_pos: line_count + 1,
                    is_append: true,
                });
            }
        }

        // 4. 校验区间互不重叠（基于原文件行号）
        for (i, a) in ops.iter().enumerate() {
            for (j, b) in ops.iter().enumerate() {
                if j <= i {
                    continue;
                }
                if let (EditKind::Replace { start: s1, end: e1 }, EditKind::Replace { start: s2, end: e2 }) =
                    (&a.kind, &b.kind)
                {
                    if s1 <= e2 && s2 <= e1 {
                        return Err(EditFileError(format!(
                            "第 {} 个操作与第 {} 个操作的替换区间重叠（{}..={} 与 {}..={}），\
                             请合并为一次区间替换",
                            i + 1,
                            j + 1,
                            s1 + 1,
                            e1 + 1,
                            s2 + 1,
                            e2 + 1
                        )));
                    }
                }
                if let EditKind::Replace { start: s, end: e } = a.kind {
                    if let EditKind::Insert { at } = b.kind {
                        // 插入点落在替换区间内部（不含边界：at==s 是在块前插，at==e+1 是在块后插）
                        if s < at && at <= e {
                            return Err(EditFileError(format!(
                                "第 {} 个操作的插入点（第 {} 行前）落在第 {} 个操作的替换区间\
                                 （{}..={}）内部",
                                j + 1,
                                at + 1,
                                i + 1,
                                s + 1,
                                e + 1
                            )));
                        }
                    }
                }
                if let EditKind::Replace { start: s, end: e } = b.kind {
                    if let EditKind::Insert { at } = a.kind {
                        if s < at && at <= e {
                            return Err(EditFileError(format!(
                                "第 {} 个操作的插入点（第 {} 行前）落在第 {} 个操作的替换区间\
                                 （{}..={}）内部",
                                i + 1,
                                at + 1,
                                j + 1,
                                s + 1,
                                e + 1
                            )));
                        }
                    }
                }
            }
        }

        // 5. 按原文件位置升序执行，用 shift 补偿前面操作造成的行号偏移。
        //    同位置插入按数组顺序自然衔接（先插的在前）。
        //    排序：位置升序；替换与插入同位时替换先执行（两种顺序结果一致）。
        let mut ordered: Vec<usize> = (0..ops.len()).collect();
        ordered.sort_by(|&x, &y| {
            let px = op_pos(&ops[x]);
            let py = op_pos(&ops[y]);
            px.cmp(&py).then_with(|| {
                // 同位置：替换排在插入前
                let rx = matches!(ops[x].kind, EditKind::Replace { .. });
                let ry = matches!(ops[y].kind, EditKind::Replace { .. });
                ry.cmp(&rx)
            })
        });

        let mut shift: isize = 0;
        // 记录每个操作的结果信息，用于 diff 报告
        let mut op_results: Vec<OpResult> = Vec::with_capacity(ops.len());

        for &idx in &ordered {
            let op = &ops[idx];
            let new_lines: Vec<String> = if op.text.is_empty() {
                Vec::new()
            } else {
                op.text.split('\n').map(String::from).collect()
            };
            let (start, original_start, original_lines, ctx_before, ctx_before_start, ctx_after, ctx_after_start) =
                match op.kind {
                    EditKind::Replace { start, end } => {
                        let s = (start as isize + shift) as usize;
                        let e = (end as isize + shift) as usize;
                        let original = lines[s..=e].to_vec();
                        // 变更前上下文（执行前状态 = 旧行之前的紧邻行）
                        let ctx_before_start = s.saturating_sub(diff_context);
                        let ctx_before = lines[ctx_before_start..s].to_vec();
                        lines.splice(s..=e, new_lines.clone());
                        shift += new_lines.len() as isize - (end as isize - start as isize + 1);
                        // 变更后上下文（执行后状态 = 新行之后的紧邻行）
                        let new_end = s + new_lines.len();
                        let ctx_after_start = new_end;
                        let ctx_after = lines[
                            new_end..(new_end + diff_context).min(lines.len())
                        ]
                        .to_vec();
                        (s, start, original, ctx_before, ctx_before_start, ctx_after, ctx_after_start)
                    }
                    EditKind::Insert { at } => {
                        let idx_pos = (at as isize + shift) as usize;
                        let ctx_before_start = idx_pos.saturating_sub(diff_context);
                        let ctx_before = lines[ctx_before_start..idx_pos].to_vec();
                        lines.splice(idx_pos..idx_pos, new_lines.clone());
                        shift += new_lines.len() as isize;
                        let after_start = idx_pos + new_lines.len();
                        let ctx_after_start = after_start;
                        let ctx_after = lines[
                            after_start..(after_start + diff_context).min(lines.len())
                        ]
                        .to_vec();
                        (idx_pos, at, Vec::new(), ctx_before, ctx_before_start, ctx_after, ctx_after_start)
                    }
                };
            op_results.push(OpResult {
                op_index: idx,
                new_start: start,
                original_start,
                original_lines,
                new_lines,
                ctx_before,
                ctx_before_start,
                ctx_after,
                ctx_after_start,
            });
        } // end for &idx in &ordered

        // 6. 重拼文件：join + 还原结尾换行（追加模式保证以换行结尾）
        let mut final_content = lines.join(eol);
        let appended = ops.iter().any(|o| o.is_append);
        if had_trailing_newline || appended {
            final_content.push_str(eol);
        }
        let new_count = lines.len();

        // 7. 写回（dry_run 仅预览不落盘）
        if !dry_run {
            fs::write(&resolved, final_content.as_bytes())
                .await
                .map_err(|e| EditFileError(format!("写入文件失败 [{}]: {e}", resolved.display())))?;
        }

        // 8. 记录历史（non-dry_run 且注入了 history）：锁内仅 push，无 I/O
        let op_id = if !dry_run {
            if let Some(history) = &self.history {
                let summary = build_summary(&resolved, old_count, new_count, &ops);
                let lines_changed = op_results
                    .iter()
                    .map(|r| r.new_lines.len().max(r.original_lines.len()) as u32)
                    .sum();
                // 构造原始操作参数（用于 edit_revise patch 重新执行）
                let params = EditOpParams::Line {
                    ops: args
                        .edits
                        .iter()
                        .map(|op| LineEditParams {
                            start_line: op.start_line,
                            end_line: op.end_line,
                            insert_before: op.insert_before,
                            text: op.text.clone(),
                        })
                        .collect(),
                };
                Some(record_edit(
                    history,
                    resolved.clone(),
                    old_content_snapshot.unwrap_or_default(),
                    final_content.clone(),
                    summary,
                    EditRecordKind::LineEdit,
                    lines_changed,
                    params,
                )
                .await)
            } else {
                None
            }
        } else {
            None
        };

        // 9. 组装报告：每个操作的变化 + 迷你 diff（含上下文）+ no-op/重复/行数校准警告
        let report = super::report::build_report(
            dry_run,
            op_id,
            &resolved,
            old_count,
            new_count,
            &op_results,
            &ops,
            &lines,
            diff_context,
        );
        Ok(report)
    }
}

/// 生成历史记录摘要（行号信息用 1-based）
fn build_summary(
    path: &std::path::Path,
    old_count: usize,
    new_count: usize,
    ops: &[ParsedOp],
) -> String {
    let mut s = String::with_capacity(64);
    s.push_str(&format!(
        "{}（{}→{} 行）",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
        old_count,
        new_count
    ));
    for (i, op) in ops.iter().enumerate() {
        if i > 0 {
            s.push_str("；");
        }
        match op.kind {
            EditKind::Replace { start, end } => {
                let text_lines = if op.text.is_empty() { 0 } else { op.text.matches('\n').count() + 1 };
                if op.text.is_empty() {
                    s.push_str(&format!("删除 {}-{}", start + 1, end + 1));
                } else if start == end {
                    s.push_str(&format!("替换第 {} 行→{} 行", start + 1, text_lines));
                } else {
                    s.push_str(&format!("替换 {}-{}→{} 行", start + 1, end + 1, text_lines));
                }
            }
            EditKind::Insert { .. } => {
                let text_lines = op.text.matches('\n').count() + 1;
                if op.is_append {
                    s.push_str(&format!("追加 {} 行", text_lines));
                } else {
                    s.push_str(&format!("插入第 {} 行前 {} 行", op.orig_pos, text_lines));
                }
            }
        }
    }
    s
}

/// 操作在原文件中的排序位置（0-based）：替换=区间起点，插入=插入点
fn op_pos(op: &ParsedOp) -> usize {
    match op.kind {
        EditKind::Replace { start, .. } => start,
        EditKind::Insert { at } => at,
    }
}
