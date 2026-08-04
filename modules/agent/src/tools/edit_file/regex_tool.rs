//! `EditFileRegexTool`：基于正则表达式的全文 / 首处替换工具。
//!
//! 与 `edit_file`（按行号精确编辑）互补：当 LLM 不确定具体行号但能描述匹配
//! 模式时（如"所有 `println!` 调用"、"TODO 注释"、"函数签名"），用正则匹配
//! 并替换。
//!
//! ## 两种模式
//! - `global=false`（默认）：仅替换第一处匹配
//! - `global=true`：替换所有匹配
//!
//! ## 替换文本语法
//! 支持 `$1` / `${name}` 捕获组引用（regex crate 语法）。
//! 推荐用 `<content>...</content>` 包裹 `replacement` 避免 JSON 转义。
//!
//! ## 行号映射
//! 字节偏移 → 行号通过 `compute_line_starts` + 二分查找完成（O(log n)）。
//! 命中可能跨多行，报告会标注起始 / 结束行号。
//!
//! ## 安全性
//! - 编译失败返回友好错误（不暴露 regex crate 内部错误细节）
//! - `dry_run=true` 仅预览：返回命中位置与上下文，不写入磁盘
//! - 启用 `history` 后，non-dry_run 替换会记录快照，返回 op_id

use std::path::PathBuf;

use regex::RegexBuilder;
use rig_core::tool::Tool;
use tokio::fs;

use super::super::resolve_path;
use super::super::text_utils::{extract_content, line_number_width};
use super::history::{EditHistoryHandle, EditOpParams, EditRecordKind, record_edit};
use super::types::EditFileRegexArgs;

/// 单次报告最多展示的命中明细数（超出截断，避免报告过长）
const MAX_HITS_DISPLAY: usize = 10;
/// 报告中替换文本预览的最大字符数
const MAX_REPLACEMENT_PREVIEW_CHARS: usize = 40;

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("edit_file_regex error: {0}")]
pub struct EditFileRegexError(String);

/// 正则编辑工具
///
/// `cwd` 为可选工作区；`history` 为可选编辑历史句柄（注入后启用 op_id 编号）。
pub struct EditFileRegexTool {
    cwd: Option<PathBuf>,
    history: Option<EditHistoryHandle>,
}

impl EditFileRegexTool {
    pub fn new() -> Self {
        Self {
            cwd: None,
            history: None,
        }
    }

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

impl Default for EditFileRegexTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for EditFileRegexTool {
    const NAME: &'static str = "edit_file_regex";

    type Error = EditFileRegexError;
    type Args = EditFileRegexArgs;
    type Output = String;

    fn description(&self) -> String {
        let cwd_hint = self
            .cwd
            .as_ref()
            .map(|p| format!("当前工作区：{}（相对路径以此为准）", p.display()))
            .unwrap_or_else(|| "未设置工作区，相对路径依赖进程工作目录".to_string());
        let history_hint = if self.history.is_some() {
            "\n**编辑历史**：每次成功替换会分配 op_id，可用 edit_revise 查看/修订、edit_undo 撤回。"
        } else {
            ""
        };
        format!(
            "**代码修改的首选工具**：当改动具有规律性（批量替换某函数调用、统一命名/格式、\
             所有 `println!` 调用、`TODO` 注释等）时**优先用本工具**，比手数行号的 edit_file 更稳更快。\
             用正则表达式匹配并替换文件内容。与 edit_file（按行号精确编辑）互补：\
              不确定行号但能描述匹配模式时使用（如所有 println! 调用、TODO 注释）。\n\n\
             **参数**：\n\
             - pattern：正则表达式（regex crate 语法）\n\
             - replacement：替换文本，支持 $1 / ${{name}} 捕获组引用；\
             推荐 <content>...</content> 包裹避免转义\n\
             - global：true=替换全部匹配，false=仅第一处（默认 false）\n\
             - multiline：true 时 ^ / $ 匹配行边界（默认 false）\n\
             - case_sensitive：默认 false（不区分大小写）\n\
             - dry_run=true 仅预览命中位置与上下文，不写入磁盘\n\
             - diff_context=N 命中明细上下文行数（每侧，默认 1，0=只显示命中行）\n\n\
               **安全**：编译失败返回友好错误；建议先用 dry_run 确认匹配范围再执行。\
               **参数也可整体用 XML 传入**（与 JSON 等价，系统自动识别）：每个参数一个标签，\
               形如 <_参数名_>值</_参数名_>，例如 <_PATTERN_>fn \\w+</_PATTERN_>、\
               <_REPLACEMENT_>替换文本</_REPLACEMENT_>，正则里的反斜杠/引号无需 JSON 转义。\
               {history_hint}\n{cwd_hint}"
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
                "pattern": {
                    "type": "string",
                    "description": "正则表达式（regex crate 语法，如 `fn \\w+`、`TODO\\(.*\\)`）"
                },
                "replacement": {
                    "type": "string",
                    "description": "替换文本。支持 $1 / ${name} 捕获组引用；推荐 <content>...</content> 包裹避免转义"
                },
                "global": {
                    "type": "boolean",
                    "description": "true = 替换所有匹配；false = 仅替换第一处。默认 false",
                    "default": false
                },
                "multiline": {
                    "type": "boolean",
                    "description": "多行模式：^ / $ 匹配行边界。默认 false",
                    "default": false
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "是否区分大小写。默认 false（不区分）",
                    "default": false
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "true = 仅预览：返回命中位置与上下文，不写入磁盘",
                    "default": false
                },
                "diff_context": {
                    "type": "integer",
                    "description": "命中明细中上下文行数（每侧），默认 1；0 = 只显示命中行本身",
                    "default": 1
                }
            },
            "required": ["path", "pattern", "replacement"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let resolved = resolve_path(&args.path, self.cwd.as_deref());
        let dry_run = args.dry_run.unwrap_or(false);
        let diff_context = args.diff_context.unwrap_or(1).min(10);
        let global = args.global.unwrap_or(false);
        let multiline = args.multiline.unwrap_or(false);
        let case_sensitive = args.case_sensitive.unwrap_or(false);

        let replacement = extract_content(&args.replacement);

        // 编译正则（用 RegexBuilder 设置 multi_line / case_insensitive）
        let re = RegexBuilder::new(&args.pattern)
            .case_insensitive(!case_sensitive)
            .multi_line(multiline)
            .build()
            .map_err(|e| EditFileRegexError(format!("正则编译失败 `{}`: {e}", args.pattern)))?;

        // 读取文件
        let bytes = fs::read(&resolved)
            .await
            .map_err(|e| {
                EditFileRegexError(format!(
                    "读取文件失败 [{}]: {e}（若文件不存在，请先用 write_file 创建）",
                    resolved.display()
                ))
            })?;
        let content = String::from_utf8(bytes).map_err(|_| {
            EditFileRegexError(format!(
                "文件不是有效 UTF-8 文本 [{}]，无法正则替换",
                resolved.display()
            ))
        })?;

        let old_content_snapshot = if !dry_run && self.history.is_some() {
            Some(content.clone())
        } else {
            None
        };

        // 收集命中位置（字节偏移）
        let matches: Vec<(usize, usize)> = if global {
            re.find_iter(&content).map(|m| (m.start(), m.end())).collect()
        } else {
            re.find(&content).map(|m| vec![(m.start(), m.end())]).unwrap_or_default()
        };

        if matches.is_empty() {
            return Ok(format!(
                "未在 [{}] 中找到匹配 `{}` 的内容",
                resolved.display(),
                args.pattern
            ));
        }

        // 计算行号映射（字节偏移 → 0-based 行号）
        let line_starts = compute_line_starts(&content);
        let old_lines: Vec<&str> = content.lines().collect();
        let old_total = old_lines.len();

        // 执行替换（Cow 自动避免无命中时的克隆，但这里已知有命中，直接 into_owned）
        let new_content: String = if global {
            re.replace_all(&content, replacement.as_str()).into_owned()
        } else {
            re.replace(&content, replacement.as_str()).into_owned()
        };
        let new_lines: Vec<&str> = new_content.lines().collect();
        let new_total = new_lines.len();

        // 写回
        if !dry_run {
            fs::write(&resolved, new_content.as_bytes())
                .await
                .map_err(|e| EditFileRegexError(format!("写入文件失败 [{}]: {e}", resolved.display())))?;
        }

        // 记录历史
        let op_id = if !dry_run {
            if let Some(history) = &self.history {
                let summary = format!(
                    "正则替换 {} 处 `{}`",
                    matches.len(),
                    args.pattern,
                );
                let params = EditOpParams::Regex {
                    pattern: args.pattern.clone(),
                    replacement: replacement.clone(),
                    global,
                    multiline,
                    case_sensitive,
                };
                Some(
                    record_edit(
                        history,
                        resolved.clone(),
                        old_content_snapshot.unwrap_or_default(),
                        new_content.clone(),
                        summary,
                        EditRecordKind::RegexEdit,
                        matches.len() as u32,
                        params,
                    )
                    .await,
                )
            } else {
                None
            }
        } else {
            None
        };

        // 生成报告
        let report = build_regex_report(
            dry_run,
            op_id,
            &resolved,
            &args.pattern,
            &replacement,
            global,
            &matches,
            &line_starts,
            &old_lines,
            &new_lines,
            old_total,
            new_total,
            diff_context,
        );

        Ok(report)
    }
}

/// 计算每行起始字节偏移（含首行 0），用于字节偏移 → 行号映射
#[inline]
fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::with_capacity(text.len() / 32 + 1);
    starts.push(0);
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// 二分查找：字节偏移落在第几行（0-based）
#[inline]
fn line_index_of(line_starts: &[usize], offset: usize) -> usize {
    debug_assert!(!line_starts.is_empty());
    let mut lo = 0usize;
    let mut hi = line_starts.len();
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if line_starts[mid] <= offset {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// 替换文本预览（最多 N 个字符，超出加 ...）
fn preview_replacement(replacement: &str) -> String {
    let chars: Vec<char> = replacement.chars().collect();
    if chars.len() > MAX_REPLACEMENT_PREVIEW_CHARS {
        let head: String = chars.iter().take(MAX_REPLACEMENT_PREVIEW_CHARS).collect();
        format!("{head}...")
    } else {
        replacement.to_string()
    }
}

/// 组装正则替换报告
#[allow(clippy::too_many_arguments)]
fn build_regex_report(
    dry_run: bool,
    op_id: Option<u64>,
    resolved: &std::path::Path,
    pattern: &str,
    replacement: &str,
    global: bool,
    matches: &[(usize, usize)],
    line_starts: &[usize],
    old_lines: &[&str],
    new_lines: &[&str],
    old_total: usize,
    new_total: usize,
    diff_context: usize,
) -> String {
    let mut report = String::with_capacity(512);
    let op_id_tag = match op_id {
        Some(id) => format!("[op_id={id}] "),
        None => String::new(),
    };
    let replacement_preview = preview_replacement(replacement);
    report.push_str(&format!(
        "{}{}正则替换 [{}] `{}` → `{}`：命中 {} 处（{}），文件 {} 行 → {} 行\n",
        op_id_tag,
        if dry_run { "[预览] " } else { "" },
        resolved.display(),
        pattern,
        replacement_preview,
        matches.len(),
        if global { "全部替换" } else { "仅第一处" },
        old_total,
        new_total
    ));

    let width = line_number_width(new_total.max(old_total));
    let show_ctx = diff_context > 0;
    let display_count = matches.len().min(MAX_HITS_DISPLAY);

    for (i, &(start_off, end_off)) in matches.iter().take(display_count).enumerate() {
        let start_line = line_index_of(line_starts, start_off); // 0-based
        let end_line = if end_off > start_off {
            line_index_of(line_starts, end_off - 1)
        } else {
            start_line
        };
        let span = if start_line == end_line {
            format!("第 {} 行", start_line + 1)
        } else {
            format!("第 {}-{} 行（跨 {} 行）", start_line + 1, end_line + 1, end_line - start_line + 1)
        };
        report.push_str(&format!("- 命中 #{}：{}\n", i + 1, span));

        if show_ctx {
            let ctx_start = start_line.saturating_sub(diff_context);
            let ctx_end = (end_line + diff_context).min(old_total.saturating_sub(1));
            report.push_str("  上下文（· 旧行 / 命中行无前缀标记）：\n");
            for ln in ctx_start..=ctx_end {
                let mark = if ln >= start_line && ln <= end_line { ' ' } else { '·' };
                report.push_str(&format!(
                    "  {mark} {:>width$}  {}\n",
                    ln + 1,
                    old_lines.get(ln).copied().unwrap_or(""),
                ));
            }
        } else {
            // 只显示命中首行
            report.push_str(&format!(
                "    {:>width$}  {}\n",
                start_line + 1,
                old_lines.get(start_line).copied().unwrap_or(""),
            ));
        }
    }
    if matches.len() > MAX_HITS_DISPLAY {
        report.push_str(&format!(
            "...（共 {} 处命中，仅展示前 {MAX_HITS_DISPLAY} 处）\n",
            matches.len()
        ));
    }

    // 展示替换后的前若干行（让 LLM 验证替换效果，特别是捕获组引用是否正确）
    if !new_lines.is_empty() {
        let show_new = new_lines.len().min(5);
        report.push_str(&format!(
            "  替换后前 {show_new} 行：\n"
        ));
        for (i, line) in new_lines.iter().take(show_new).enumerate() {
            report.push_str(&format!("  + {:>width$}  {line}\n", i + 1));
        }
    }

    report.push_str(
        "\n[提示：正则替换后行号会变化，后续 read_file / search_file / edit_file 请重新读取文件]",
    );
    if let Some(id) = op_id {
        report.push_str(&format!(
            "\n[本次操作 op_id={id}：可调用 edit_revise(action=view, op_id={id}) 查看详情，\
             或 edit_undo(op_id={id}) 撤回本次操作]"
        ));
    }
    if dry_run {
        report.push_str(
            "\n[预览模式：以上为将要发生的变更，文件**未写入磁盘**。确认无误后去掉 dry_run=true 再执行]",
        );
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::edit_file::tests_common::{read, setup_file, tmp_dir};
    use rig_core::tool::Tool;

    #[tokio::test]
    async fn regex_replace_first() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "foo\nbar\nfoo\nbaz\n").await;
        let tool = EditFileRegexTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileRegexArgs {
                path: "a.txt".to_string(),
                pattern: "foo".to_string(),
                replacement: "FOO".to_string(),
                global: Some(false),
                multiline: None,
                case_sensitive: None,
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("命中 1 处"));
        assert!(r.contains("仅第一处"));
        assert_eq!(read(&p).await, "FOO\nbar\nfoo\nbaz\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn regex_replace_all() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "foo\nbar\nfoo\nbaz\n").await;
        let tool = EditFileRegexTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileRegexArgs {
                path: "a.txt".to_string(),
                pattern: "foo".to_string(),
                replacement: "FOO".to_string(),
                global: Some(true),
                multiline: None,
                case_sensitive: None,
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("命中 2 处"));
        assert!(r.contains("全部替换"));
        assert_eq!(read(&p).await, "FOO\nbar\nFOO\nbaz\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn regex_replace_with_capture_group() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "fn old_name() {}\n").await;
        let tool = EditFileRegexTool::with_cwd(dir.clone());
        tool.call(EditFileRegexArgs {
            path: "a.txt".to_string(),
            pattern: r"fn (\w+)\(\)".to_string(),
            replacement: "fn new_$1()".to_string(),
            global: Some(true),
            multiline: None,
            case_sensitive: None,
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
        assert_eq!(read(&p).await, "fn new_old_name() {}\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn regex_no_match_returns_info() {
        let dir = tmp_dir();
        setup_file(&dir, "a.txt", "hello\n").await;
        let tool = EditFileRegexTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileRegexArgs {
                path: "a.txt".to_string(),
                pattern: "xyz".to_string(),
                replacement: "X".to_string(),
                global: None,
                multiline: None,
                case_sensitive: None,
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("未在"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn regex_invalid_pattern_returns_error() {
        let dir = tmp_dir();
        setup_file(&dir, "a.txt", "hello\n").await;
        let tool = EditFileRegexTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileRegexArgs {
                path: "a.txt".to_string(),
                pattern: "[invalid".to_string(),
                replacement: "X".to_string(),
                global: None,
                multiline: None,
                case_sensitive: None,
                dry_run: None,
                diff_context: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("正则编译失败"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn regex_dry_run_does_not_write() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "foo\nfoo\n").await;
        let tool = EditFileRegexTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileRegexArgs {
                path: "a.txt".to_string(),
                pattern: "foo".to_string(),
                replacement: "BAR".to_string(),
                global: Some(true),
                multiline: None,
                case_sensitive: None,
                dry_run: Some(true),
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("[预览]"));
        assert!(r.contains("未写入磁盘"));
        // 文件未被修改
        assert_eq!(read(&p).await, "foo\nfoo\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn regex_case_sensitive_default_insensitive() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "Foo\nFOO\nfoo\n").await;
        let tool = EditFileRegexTool::with_cwd(dir.clone());
        tool.call(EditFileRegexArgs {
            path: "a.txt".to_string(),
            pattern: "foo".to_string(),
            replacement: "X".to_string(),
            global: Some(true),
            multiline: None,
            case_sensitive: None, // 默认不区分大小写
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
        assert_eq!(read(&p).await, "X\nX\nX\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn regex_case_sensitive_enabled() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "Foo\nfoo\n").await;
        let tool = EditFileRegexTool::with_cwd(dir.clone());
        tool.call(EditFileRegexArgs {
            path: "a.txt".to_string(),
            pattern: "foo".to_string(),
            replacement: "X".to_string(),
            global: Some(true),
            multiline: None,
            case_sensitive: Some(true),
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
        assert_eq!(read(&p).await, "Foo\nX\n");
        std::fs::remove_dir_all(&dir).ok();
    }
}
