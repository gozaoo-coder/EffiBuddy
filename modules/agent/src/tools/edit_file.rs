//! edit_file 工具：按行号精确编辑本地文件
//!
//! 与 read_file（带行号输出）/ search_file（带行号命中）配合使用：
//! LLM 先读取/搜索拿到行号，再用本工具**精确替换目标行**。
//!
//! 每个 `edits` 元素支持三种操作模式：
//!
//! 1. **替换**：`start_line`（+ 可选 `end_line`）+ `text`
//!    - 只给 start_line：把该行替换为 text（text 可为多行；空 text 表示删除该行）
//!    - 给 start_line + end_line：把 `[start_line, end_line]` 区间整段替换为 text
//! 2. **插入**：`insert_before` + `text`：在指定行**之前**插入新行
//!    - insert_before = 文件总行数 + 1 等价于追加到末尾
//! 3. **追加**：不给任何行号字段，直接把 text 作为**新行写入文件末尾**
//!    （即"写入文本的最后一行新行"，无需知道当前行数）
//!
//! 规则：
//! - 行号一律为 **1-based**，指**编辑前原文件**的行号
//! - 多个 edits 自动按行号排序执行，区间互不重叠（重叠会报错）
//! - 一次调用最多 50 个操作
//! - text 支持 `<content>...</content>` / CDATA 包裹避免转义（同 write_file）
//!
//! 返回**迷你 diff**（`-` 原行 / `+` 新行，均带行号）便于 LLM 自查；
//! 若插入文本的最后一行与插入点后的首行完全相同，会输出 ⚠️ 重复插入警告
//! （LLM 最常见的复制错误：把插入点后的行也抄进了 text）。
//!
//! 工作区支持：构造时传入 `cwd: Option<PathBuf>`，相对路径会 join 到 cwd。
//! 信任本地 agent 环境，路径不做沙箱限制。

use std::path::PathBuf;

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::fs;

use super::resolve_path;
use super::text_utils::{extract_content, line_number_width};

/// 单次调用最大操作数，防止误传超大数组
const MAX_OPS: usize = 50;
/// 每个操作的预览最大行数
const MAX_PREVIEW_LINES: usize = 40;

/// 工具参数
#[derive(Deserialize)]
pub struct EditFileArgs {
    /// 要编辑的文件路径（绝对或相对工作区）
    pub path: String,
    /// 编辑操作列表（按行号自动排序执行；行号指编辑前原文件）
    pub edits: Vec<EditOp>,
    /// true = 仅预览：计算并返回完整报告（含 diff 与行号变化），不写入磁盘
    #[serde(default)]
    pub dry_run: Option<bool>,
    /// 变更明细中上下文行数（每侧），默认 1；0 = 只显示变更行本身
    #[serde(default)]
    pub diff_context: Option<usize>,
}

/// 单个编辑操作
///
/// 模式判定优先级：`insert_before` 有值 → 插入；否则 `start_line` 有值 → 替换；
/// 两者都无 → 追加到文件末尾。
#[derive(Deserialize)]
pub struct EditOp {
    /// 替换模式的起始行号（1-based，含）。None = 追加模式
    #[serde(default)]
    pub start_line: Option<usize>,
    /// 替换模式的结束行号（1-based，含）。None = 仅替换 start_line 一行
    #[serde(default)]
    pub end_line: Option<usize>,
    /// 插入模式：在指定行号之前插入新文本（与 start_line/end_line 互斥，优先）
    #[serde(default)]
    pub insert_before: Option<usize>,
    /// 目标文本（替换/插入/追加的内容）。推荐 `<content>...</content>` 包裹避免转义
    pub text: String,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("edit_file error: {0}")]
pub struct EditFileError(String);

/// 文件编辑工具
///
/// `cwd` 为可选工作区：设置后相对路径以此为基准，未设置则依赖进程 cwd。
pub struct EditFileTool {
    cwd: Option<PathBuf>,
}

impl EditFileTool {
    pub fn new() -> Self {
        Self { cwd: None }
    }

    /// 指定工作区目录，相对路径将 join 到此目录
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self { cwd: Some(cwd) }
    }
}

impl Default for EditFileTool {
    fn default() -> Self {
        Self::new()
    }
}

/// 内部解析后的操作（行号统一转 0-based）
enum EditKind {
    /// 替换原文件 [start, end]（0-based，含两端）
    Replace { start: usize, end: usize },
    /// 在原文件第 at 行（0-based）之前插入
    Insert { at: usize },
}

struct ParsedOp {
    kind: EditKind,
    text: String,
    /// 用于报告的原文件行号（1-based）：替换=start_line，插入/追加=插入位置
    orig_pos: usize,
    /// 是否为追加到末尾（仅用于报告文案）
    is_append: bool,
}

struct OpResult {
    /// 对应 ops 中的下标
    op_index: usize,
    /// 新文本在新文件中的起始 0-based 行号
    new_start: usize,
    /// 操作在**原文件**中的 0-based 起始位置（替换=区间起点，插入=插入点）
    original_start: usize,
    /// 被替换掉的原始行（插入/追加为空）
    original_lines: Vec<String>,
    /// 写入的新行
    new_lines: Vec<String>,
    /// 变更前上下文（执行时状态，即旧行之前的紧邻行）
    ctx_before: Vec<String>,
    /// ctx_before 第一行在**执行时**文件中的 0-based 行号
    ctx_before_start: usize,
    /// 变更后上下文（执行时状态，即新行之后的紧邻行）
    ctx_after: Vec<String>,
    /// ctx_after 第一行在**执行时**文件中的 0-based 行号
    ctx_after_start: usize,
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
        format!(
            "按行号精确编辑本地文本文件，**不覆盖整文件**。与 read_file / search_file 配合：\
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
              diff_context=N 控制变更明细上下文行数（默认 1，0 = 只显示变更行）。\
              路径不做沙箱限制（信任本地 agent 环境）。\n{cwd_hint}"
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
                    "description": "编辑操作列表，按行号自动排序执行；行号指编辑前原文件（1-based）",
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

        // 8. 组装报告：每个操作的变化 + 迷你 diff（含上下文）+ no-op/重复警告
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
        for result in &op_results {
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
        Ok(report)
    }
}

/// 操作在原文件中的排序位置（0-based）：替换=区间起点，插入=插入点
fn op_pos(op: &ParsedOp) -> usize {
    match op.kind {
        EditKind::Replace { start, .. } => start,
        EditKind::Insert { at } => at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("effisuite-edit-test-{}", uuid::Uuid::new_v4()))
    }

    async fn setup_file(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(&p, content).unwrap();
        p
    }

    async fn read(p: &PathBuf) -> String {
        tokio::fs::read_to_string(p).await.unwrap()
    }

    #[tokio::test]
    async fn replace_single_line() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "line1\nline2\nline3\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(2),
                    end_line: None,
                    insert_before: None,
                    text: "<content>CHANGED</content>".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("替换第 2 行"));
        assert_eq!(read(&p).await, "line1\nCHANGED\nline3\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn replace_range_with_multiline() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "a\nb\nc\nd\ne\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(2),
                    end_line: Some(4),
                    insert_before: None,
                    text: "<content>X\nY</content>".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("替换第 2-4 行 → 2 行新文本"));
        assert_eq!(read(&p).await, "a\nX\nY\ne\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn delete_line_with_empty_text() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "a\nb\nc\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(2),
                    end_line: None,
                    insert_before: None,
                    text: String::new(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("删除原第 2-2 行"));
        assert_eq!(read(&p).await, "a\nc\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn insert_before_line() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "a\nb\nc\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: None,
                    end_line: None,
                    insert_before: Some(2),
                    text: "<content>INS</content>".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("在第 2 行前插入 1 行新文本"));
        assert_eq!(read(&p).await, "a\nINS\nb\nc\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn append_writes_new_last_line() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "a\nb\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: None,
                    end_line: None,
                    insert_before: None,
                    text: "<content>c\nd</content>".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("在文件末尾追加 2 行新文本"));
        assert_eq!(read(&p).await, "a\nb\nc\nd\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn append_to_file_without_trailing_newline() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "a\nb").await; // 无结尾换行
        let tool = EditFileTool::with_cwd(dir.clone());
        tool.call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: None,
                end_line: None,
                insert_before: None,
                text: "c".to_string(),
            }],
                dry_run: None,
                diff_context: None,
        })
        .await
        .unwrap();
        // 追加内容作为"新行"写入，需补换行
        assert_eq!(read(&p).await, "a\nb\nc\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn preserve_missing_trailing_newline_without_append() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "a\nb").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        tool.call(EditFileArgs {
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
        assert_eq!(read(&p).await, "a\nB"); // 保留无结尾换行
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn preserve_crlf_style() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "a\r\nb\r\nc\r\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        tool.call(EditFileArgs {
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
        assert_eq!(read(&p).await, "a\r\nB\r\nc\r\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn multiple_edits_in_one_call() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "1\n2\n3\n4\n5\n6\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        tool.call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![
                EditOp { start_line: Some(2), end_line: None, insert_before: None, text: "two".to_string() },
                EditOp { start_line: None, end_line: None, insert_before: Some(4), text: "inserted".to_string() },
                EditOp { start_line: Some(6), end_line: Some(6), insert_before: None, text: "six".to_string() },
                EditOp { start_line: None, end_line: None, insert_before: None, text: "seven".to_string() },
            ],
                dry_run: None,
                diff_context: None,
        })
        .await
        .unwrap();
        assert_eq!(read(&p).await, "1\ntwo\n3\ninserted\n4\n5\nsix\nseven\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn same_position_inserts_keep_array_order() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "a\nb\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        tool.call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![
                EditOp { start_line: None, end_line: None, insert_before: Some(1), text: "A".to_string() },
                EditOp { start_line: None, end_line: None, insert_before: Some(1), text: "B".to_string() },
            ],
                dry_run: None,
                diff_context: None,
        })
        .await
        .unwrap();
        assert_eq!(read(&p).await, "A\nB\na\nb\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn overlapping_ranges_rejected() {
        let dir = tmp_dir();
        setup_file(&dir, "a.txt", "1\n2\n3\n4\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![
                    EditOp { start_line: Some(1), end_line: Some(3), insert_before: None, text: "x".to_string() },
                    EditOp { start_line: Some(2), end_line: Some(4), insert_before: None, text: "y".to_string() },
                ],
                dry_run: None,
                diff_context: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("重叠"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn out_of_range_rejected() {
        let dir = tmp_dir();
        setup_file(&dir, "a.txt", "1\n2\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(9),
                    end_line: None,
                    insert_before: None,
                    text: "x".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("超出"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn missing_file_rejected() {
        let dir = tmp_dir();
        let tool = EditFileTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileArgs {
                path: "nope.txt".to_string(),
                edits: vec![EditOp {
                    start_line: None,
                    end_line: None,
                    insert_before: None,
                    text: "x".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("write_file"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn report_contains_mini_diff() {
        let dir = tmp_dir();
        setup_file(&dir, "a.txt", "a\nb\nc\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(2),
                    end_line: None,
                    insert_before: None,
                    text: "B\nB2".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        // 迷你 diff：- 原行（原行号 2） / + 新行（新行号 2-3）
        assert!(r.contains("- 2  b"), "out: {r}");
        assert!(r.contains("+ 2  B") && r.contains("+ 3  B2"), "out: {r}");
        assert!(r.contains("新行号 2-3"), "out: {r}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn duplicate_insert_warns() {
        let dir = tmp_dir();
        setup_file(&dir, "a.txt", "impl Foo {\n    /// 创建空索引\n    pub fn new() -> Self {\n}\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        // 模拟 LLM 复制错误：插入文本把插入点后的行也抄了进去
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: None,
                    end_line: None,
                    insert_before: Some(2),
                    text: "    /// 创建空索引\n    pub fn new() -> Self {".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("⚠️ 警告"), "out: {r}");
        assert!(r.contains("重复插入"), "out: {r}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn legit_duplicate_insert_does_not_warn() {
        let dir = tmp_dir();
        setup_file(&dir, "a.txt", "a\nb\nc\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        // 插入点后首行是 c，插入文本末行是 b → 不触发警告
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: None,
                    end_line: None,
                    insert_before: Some(3),
                    text: "x\nb".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(!r.contains("⚠️"), "out: {r}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn edit_empty_file_via_append() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        tool.call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: None,
                end_line: None,
                insert_before: None,
                text: "hello".to_string(),
            }],
                dry_run: None,
                diff_context: None,
        })
        .await
        .unwrap();
        assert_eq!(read(&p).await, "hello\n");
        std::fs::remove_dir_all(&dir).ok();
    }
    #[tokio::test]
    async fn dry_run_previews_without_writing() {
        let dir = tmp_dir();
        let p = setup_file(&dir, "a.txt", "a\nb\nc\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(2),
                    end_line: None,
                    insert_before: None,
                    text: "B".to_string(),
                }],
                dry_run: Some(true),
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("[预览]"), "out: {r}");
        assert!(r.contains("未写入磁盘"), "out: {r}");
        assert!(r.contains("+ 2  B"), "out: {r}");
        // 文件内容未被改变
        assert_eq!(read(&p).await, "a\nb\nc\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn noop_replace_warns() {
        let dir = tmp_dir();
        setup_file(&dir, "a.txt", "a\nb\nc\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        // 替换文本与原内容完全相同 → 应警告未产生实际变更
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(2),
                    end_line: None,
                    insert_before: None,
                    text: "b".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("未产生实际变更"), "out: {r}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn whitespace_only_replace_hints() {
        let dir = tmp_dir();
        setup_file(&dir, "a.txt", "fn main() {\n    let x = 1;\n}\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        // 只改缩进：4 空格 → 2 空格，内容相同 → 应提示仅有空白差异
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(2),
                    end_line: None,
                    insert_before: None,
                    text: "  let x = 1;".to_string(),
                }],
                dry_run: None,
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("仅有空白/缩进差异"), "out: {r}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn diff_context_shows_surrounding_lines() {
        let dir = tmp_dir();
        setup_file(&dir, "a.txt", "a\nb\nc\nd\ne\n").await;
        let tool = EditFileTool::with_cwd(dir.clone());
        // 默认 diff_context=1：diff 含上下文行（· 标记）
        // 用 dry_run 避免修改文件，保证第二次调用仍在原文件上操作
        let r = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(3),
                    end_line: None,
                    insert_before: None,
                    text: "C".to_string(),
                }],
                dry_run: Some(true),
                diff_context: None,
            })
            .await
            .unwrap();
        assert!(r.contains("· 2  b"), "out: {r}");
        assert!(r.contains("- 3  c"), "out: {r}");
        assert!(r.contains("+ 3  C"), "out: {r}");
        assert!(r.contains("· 4  d"), "out: {r}");

        // diff_context=0：不显示上下文行（验证上下文行号 2 和 4 不出现）
        // 注意：不能简单断言 !contains("·")，因为"变更明细（· 上下文...）"
        // 标题里也含 · 字符；改为检查上下文行号格式 "· 2" / "· 4" 不出现
        let r2 = tool
            .call(EditFileArgs {
                path: "a.txt".to_string(),
                edits: vec![EditOp {
                    start_line: Some(3),
                    end_line: None,
                    insert_before: None,
                    text: "C".to_string(),
                }],
                dry_run: Some(true),
                diff_context: Some(0),
            })
            .await
            .unwrap();
        assert!(!r2.contains("· 2"), "out: {r2}");
        assert!(!r2.contains("· 4"), "out: {r2}");
        std::fs::remove_dir_all(&dir).ok();
    }

}
