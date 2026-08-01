//! 报告生成：把执行结果组装成含迷你 diff 与警告的人类可读文本。
//!
//! 从 [`super::tool`] 的 `call` 末尾调用，纯函数（仅读取执行后的状态）。

use std::path::Path;

use super::super::text_utils::line_number_width;
use super::types::{EditKind, OpResult, ParsedOp};
use super::MAX_PREVIEW_LINES;

/// 组装编辑报告。
///
/// - `op_results` / `ops`：按执行顺序记录的每个操作结果与对应解析操作（下标一致）
/// - `lines`：**执行后**的文件行（用于重复插入检测，比较插入点之后的内容）
/// - `diff_context`：变更明细上下文行数（每侧），0 = 只显示变更行本身
#[allow(clippy::too_many_arguments)]
pub(super) fn build_report(
    dry_run: bool,
    resolved: &Path,
    old_count: usize,
    new_count: usize,
    op_results: &[OpResult],
    ops: &[ParsedOp],
    lines: &[String],
    diff_context: usize,
) -> String {
    let mut report = String::with_capacity(512);
    report.push_str(&format!(
        "{}文件 {}（{} 行 → {} 行）\n",
        if dry_run { "[预览] " } else { "" },
        resolved.display(),
        old_count,
        new_count
    ));
    let width = line_number_width(new_count);
    let show_ctx = diff_context > 0;
    for result in op_results {
        let op = &ops[result.op_index];
        let text_lines = &result.new_lines;
        let end = result.new_start + text_lines.len().saturating_sub(1);
        let change_desc = match op.kind {
            EditKind::Replace { start: s, end: e } => {
                if text_lines.is_empty() {
                    format!("删除原第 {}-{} 行", s + 1, e + 1)
                } else if s == e {
                    format!("替换第 {} 行 → {} 行新文本", s + 1, text_lines.len())
                } else {
                    format!(
                        "替换第 {}-{} 行 → {} 行新文本",
                        s + 1,
                        e + 1,
                        text_lines.len()
                    )
                }
            }
            EditKind::Insert { .. } => {
                if op.is_append {
                    format!("在文件末尾追加 {} 行新文本", text_lines.len())
                } else {
                    format!(
                        "在第 {} 行前插入 {} 行新文本",
                        op.orig_pos,
                        text_lines.len()
                    )
                }
            }
        };
        if text_lines.is_empty() {
            // 删除行：没有对应的新行号，不再标注
            report.push_str(&format!("- {change_desc}\n"));
        } else if result.new_start == end {
            report.push_str(&format!(
                "- {change_desc}（新行号 {}）\n",
                result.new_start + 1
            ));
        } else {
            report.push_str(&format!(
                "- {change_desc}（新行号 {}-{}）\n",
                result.new_start + 1,
                end + 1
            ));
        }

        // 迷你 diff（· 上下文 / - 旧行 / + 新行，均带行号），一眼可辨重复/误删/改错位置
        let mut diff_shown = false;
        if show_ctx && !result.ctx_before.is_empty() {
            for (k, line) in result.ctx_before.iter().enumerate() {
                if !diff_shown {
                    report.push_str("  变更明细（· 上下文 / - 旧行 / + 新行）：\n");
                    diff_shown = true;
                }
                report.push_str(&format!(
                    "  · {:>width$}  {}\n",
                    result.ctx_before_start + 1 + k,
                    line
                ));
            }
        }
        for (k, line) in result.original_lines.iter().take(MAX_PREVIEW_LINES).enumerate() {
            if !diff_shown {
                report.push_str("  变更明细（· 上下文 / - 旧行 / + 新行）：\n");
                diff_shown = true;
            }
            report.push_str(&format!(
                "  - {:>width$}  {}\n",
                result.original_start + k + 1,
                line
            ));
        }
        if result.original_lines.len() > MAX_PREVIEW_LINES {
            report.push_str(&format!(
                "  ...（原行共 {} 行，预览截断）\n",
                result.original_lines.len()
            ));
        }
        for (k, line) in text_lines.iter().take(MAX_PREVIEW_LINES).enumerate() {
            report.push_str(&format!(
                "  + {:>width$}  {}\n",
                result.new_start + 1 + k,
                line
            ));
        }
        if text_lines.len() > MAX_PREVIEW_LINES {
            report.push_str(&format!(
                "  ...（新行共 {} 行，预览截断）\n",
                text_lines.len()
            ));
        }
        if show_ctx && !result.ctx_after.is_empty() {
            for (k, line) in result.ctx_after.iter().enumerate() {
                report.push_str(&format!(
                    "  · {:>width$}  {}\n",
                    result.ctx_after_start + 1 + k,
                    line
                ));
            }
        }

        // no-op / 仅空白差异检测（仅替换操作）：
        // 常见错误——替换文本与原内容相同（行号选错），或只改了空白但内容没变
        if let EditKind::Replace { .. } = op.kind {
            if !text_lines.is_empty() && *text_lines == result.original_lines {
                report.push_str(&format!(
                    "⚠️ 该操作未产生实际变更：替换文本与原内容完全相同（原第 {}-{} 行），\
                     可能是行号选错或文本写错，请用 read_file 核对后重试\n",
                    result.original_start + 1,
                    result.original_start + result.original_lines.len()
                ));
            } else if !text_lines.is_empty()
                && text_lines
                    .iter()
                    .map(|l| l.trim())
                    .collect::<Vec<_>>()
                    == result
                        .original_lines
                        .iter()
                        .map(|l| l.trim())
                        .collect::<Vec<_>>()
            {
                report.push_str(
                    "⚠️ 该替换仅有空白/缩进差异（去掉空白后内容相同）；若目的是修缩进，\
                     请确认上面的 diff 中缩进已改对\n",
                );
            }
        }

        // 重复检测：插入/追加文本的**末尾若干行**与插入点后的**前若干行**完全相同，
        // 这是 LLM 复制插入点时最常见的错误（如本工具实测：把插入点后的函数签名
        // 整段抄进 text，导致签名重复、代码括号失配）
        if !text_lines.is_empty() {
            if let EditKind::Insert { .. } = op.kind {
                let after = result.new_start + text_lines.len();
                let max_k = text_lines.len().min(lines.len().saturating_sub(after));
                let mut dup_k = 0usize;
                for k in (1..=max_k).rev() {
                    if text_lines[text_lines.len() - k..] == lines[after..after + k] {
                        dup_k = k;
                        break;
                    }
                }
                if dup_k > 0 {
                    let sample = &text_lines[text_lines.len() - dup_k];
                    report.push_str(&format!(
                        "⚠️ 警告：插入文本末尾 {dup_k} 行与插入点后的前 {dup_k} 行完全相同\
                         （如 `{sample}`），疑似重复插入！请用 read_file 复核，必要时删除重复行\n"
                    ));
                }
            }
        }
    }
    report.push_str(
        "\n[提示：以上行号为本次编辑后的新行号，后续 read_file / search_file / edit_file 均以此为准]",
    );
    if dry_run {
        report.push_str(
            "\n[预览模式：以上为将要发生的变更，文件**未写入磁盘**。确认无误后去掉 dry_run=true 再执行]",
        );
    }
    report
}
