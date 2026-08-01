//! grep 工具：工作区正则表达式搜索（grep / ripgrep 风格）
//!
//! 递归遍历搜索根目录（默认工作区）下**全部文本文件**，对内容做**正则表达式**
//! 匹配，返回命中行的"文件路径 + 行号 + 行内容"。行号格式与 read_file / search_file
//! 完全一致，方便 LLM 直接引用行号调用 edit_file 精确编辑：
//!
//! ```text
//! path: src/main.rs
//!   12  fn main() {
//!   45  let x = 1;   // 命中正则
//! ```
//!
//! 与 search_file 的区别：
//! - search_file 做**字面关键词**匹配（任意一个 / 全部）
//! - grep 做**正则表达式**匹配（如 `fn \w+`、`TODO\(.*\)`、`\d{4}-\d{2}-\d{2}`）
//!
//! 支持三种输出模式：
//! - `content`（默认）：显示每个匹配行（path + 行号 + 内容）
//! - `files_with_matches`：只列出有匹配的文件路径
//! - `count`：每文件一行 `path: N matches`
//!
//! 其他特性：大小写敏感/不敏感、多行模式（可跨行匹配 + `^`/`$` 锚定行边界）、
//! glob 文件名过滤（如 `*.rs`）、上下文行（与 search_file 一致的 `·` 前缀）。
//! 自动跳过生成目录（.git / node_modules / target / dist 等）、二进制文件
//! （NUL 字节探测）与超大文件（> 4 MiB）。
//!
//! 正则在 `call` 入口编译**一次**后复用于全部文件，避免重复编译开销。

use std::collections::BTreeMap;
use std::path::PathBuf;

use rig_core::tool::Tool;
use tokio::fs;

use super::resolve_path;
use super::text_utils::{format_numbered_line, line_number_width};

mod args;
mod engine;
mod error;

#[cfg(test)]
mod tests;

pub use args::GrepArgs;
pub use error::GrepError;

#[allow(unused_imports)]
use engine::*;

/// 默认最大返回命中数
const DEFAULT_MAX_MATCHES: usize = 300;
/// 单文件最大命中行数（防止单个大文件刷屏）
const MAX_MATCHES_PER_FILE: usize = 100;
/// 扫描文件数硬上限（防止超大仓库长时间阻塞）
const MAX_SCAN_FILES: usize = 20_000;
/// 跳过大于该字节数的文件（4 MiB）
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// 上下文行数硬上限（防止输出爆炸）
const MAX_CONTEXT: usize = 20;
/// 生成的/依赖目录，搜索时跳过
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "__pycache__",
    ".venv",
    "venv",
    ".pytest_cache",
    ".mypy_cache",
    ".next",
    ".turbo",
    ".nuxt",
    ".svelte-kit",
    ".output",
    "coverage",
];

/// 输出模式标识
const MODE_CONTENT: &str = "content";
const MODE_FILES: &str = "files_with_matches";
const MODE_COUNT: &str = "count";

/// 工作区正则搜索工具
///
/// `cwd` 为可选工作区：设置后作为默认搜索根，相对路径以此为基准。
pub struct GrepTool {
    cwd: Option<PathBuf>,
}

impl GrepTool {
    pub fn new() -> Self {
        Self { cwd: None }
    }

    /// 指定工作区目录，默认在此目录下正则搜索
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self { cwd: Some(cwd) }
    }
}

impl Default for GrepTool {
    fn default() -> Self {
        Self::new()
    }
}

/// 单个文件的命中块：(相对展示路径, 文件总行数, 全文行列表, Vec<(行号, 是否命中)>)
/// 第 4 项 context=0 时即命中行、>0 时含上下文行。
type HitBlock = (String, usize, Vec<String>, Vec<(usize, bool)>);

impl Tool for GrepTool {
    const NAME: &'static str = "grep";

    type Error = GrepError;
    type Args = GrepArgs;
    type Output = String;

    fn description(&self) -> String {
        let cwd_hint = self
            .cwd
            .as_ref()
            .map(|p| format!("当前工作区：{}（默认搜索根，相对路径以此为准）", p.display()))
            .unwrap_or_else(|| "未设置工作区，默认在进程工作目录下搜索".to_string());
        format!(
            "在工作区目录下**递归正则搜索**：对全部文本文件的内容做正则表达式匹配，\
             返回命中行的文件路径 + 行号 + 行内容（类似 grep / ripgrep）。\n\n\
             **与其他查找工具的边界**：\n\
             - `list_files`：列目录树，不按模式过滤、不读内容\n\
             - `glob`：按文件名模式匹配，不读内容\n\
             - `search_file`：字面关键词搜文件内容（非正则）\n\
             - `grep`（本工具）：正则表达式搜文件内容\n\
             - `search_codebase`：基于关键词加权排序的代码搜索（自然语言查询）\n\n\
             **输入**：pattern 为正则表达式（如 `fn \\w+`、`TODO\\(.*\\)`、`\\d{{4}}-\\d{{2}}-\\d{{2}}`），\
             默认不区分大小写，case_sensitive=true 时区分。multiline=true 时正则匹配整篇文本\
             （可跨行，`^`/`$` 锚定行边界），默认逐行匹配。\n\n\
             **输出模式**（output_mode）：\n\
             - `content`（默认）：显示每个匹配行（path + 行号 + 内容），格式与 search_file 一致\n\
             - `files_with_matches`：只列出有匹配的文件路径\n\
             - `count`：每文件一行 `path: N matches`\n\n\
             **返回格式**（行号 1-based，与 read_file 一致）：\n\
             path: src/main.rs\n\
             &nbsp;&nbsp;12  fn main() {{\n\
             &nbsp;&nbsp;45  let x = 1;   // 命中\n\n\
             **行号用于精确编辑**：拿到命中行号后，可直接调用 edit_file 的 start_line/end_line \
             替换目标行。\n\
             **context 参数**：命中行前后各显示 N 行上下文（默认 0 = 只显示命中行），\
             上下文行以 `·` 前缀标记。\n\
             **glob 参数**：文件名过滤（如 `*.rs` 只搜 Rust 文件），只匹配文件名不匹配路径。\n\
             自动跳过生成目录（.git/node_modules/target/dist 等）、二进制文件与 \
             大于 4 MiB 的文件。默认最多返回 {DEFAULT_MAX_MATCHES} 条结果，可用 max_matches 调整。\n\
             正则在调用入口编译一次后复用于全部文件。{cwd_hint}"
        )
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "正则表达式（regex crate 语法，如 fn \\w+、TODO\\(.*\\)、\\d{4}-\\d{2}-\\d{2}）"
                },
                "path": {
                    "type": "string",
                    "description": "搜索根目录（绝对或相对工作区），默认工作区根目录"
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "输出模式：content=显示匹配行（默认），files_with_matches=只列文件名，count=每文件计数",
                    "default": "content"
                },
                "glob": {
                    "type": "string",
                    "description": "文件名 glob 过滤模式（如 *.rs），只匹配文件名不匹配路径，默认不过滤"
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "是否区分大小写，默认 false（不区分）",
                    "default": false
                },
                "multiline": {
                    "type": "boolean",
                    "description": "多行模式：true 时正则匹配整篇文本（可跨行，^/$ 锚定行边界），默认 false（逐行匹配）",
                    "default": false
                },
                "context": {
                    "type": "integer",
                    "description": "命中行前后各显示多少行上下文，默认 0（只显示命中行）；上下文行以 · 前缀标记",
                    "default": 0
                },
                "max_matches": {
                    "type": "integer",
                    "description": "最多返回命中数（content 限制显示行数，其他模式限制文件数），默认 300",
                    "default": DEFAULT_MAX_MATCHES
                }
            },
            "required": ["pattern"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // ===== 1. 参数解析与校验 =====
        let case_sensitive = args.case_sensitive.unwrap_or(false);
        let multiline = args.multiline.unwrap_or(false);
        let max_matches = args.max_matches.unwrap_or(DEFAULT_MAX_MATCHES).max(1);
        let context = args.context.unwrap_or(0).min(MAX_CONTEXT);
        let mode = args.output_mode.as_deref().unwrap_or(MODE_CONTENT);
        match mode {
            MODE_CONTENT | MODE_FILES | MODE_COUNT => {}
            other => {
                return Err(GrepError(format!(
                    "无效的 output_mode [{}]：可选值为 content / files_with_matches / count",
                    other
                )))
            }
        }
        let glob = args.glob.as_deref().filter(|g| !g.is_empty());

        // ===== 2. 正则编译（只编译一次，复用于全部文件）=====
        let re = regex::RegexBuilder::new(&args.pattern)
            .case_insensitive(!case_sensitive)
            .multi_line(multiline)
            .build()
            .map_err(|e| {
                GrepError(format!("正则表达式编译失败 [{}]: {e}", args.pattern))
            })?;

        // ===== 3. 解析搜索根 =====
        let root = match &args.path {
            Some(p) => resolve_path(p, self.cwd.as_deref()),
            None => self.cwd.clone().unwrap_or_else(|| PathBuf::from(".")),
        };
        let meta = fs::metadata(&root)
            .await
            .map_err(|e| GrepError(format!("访问搜索根目录失败 [{}]: {e}", root.display())))?;
        if !meta.is_dir() {
            return Err(GrepError(format!(
                "搜索根不是目录 [{}]",
                root.display()
            )));
        }

        // ===== 4. 栈式目录遍历（与 search_file 一致）=====
        let mut stack: Vec<PathBuf> = vec![root.clone()];
        let mut scanned_files: usize = 0;
        let mut skipped_scan_cap = false;
        // content 模式：命中块
        let mut blocks: Vec<HitBlock> = Vec::new();
        // count 模式：(路径, 命中数)
        let mut count_per_file: Vec<(String, usize)> = Vec::new();
        // files_with_matches 模式：命中文件路径
        let mut matched_files: Vec<String> = Vec::new();

        while let Some(dir) = stack.pop() {
            let mut rd = fs::read_dir(&dir)
                .await
                .map_err(|e| GrepError(format!("读取目录失败 [{}]: {e}", dir.display())))?;
            while let Some(entry) = rd
                .next_entry()
                .await
                .map_err(|e| GrepError(format!("读取目录条目失败 [{}]: {e}", dir.display())))?
            {
                let name = entry.file_name().to_string_lossy().into_owned();
                let file_meta = entry
                    .metadata()
                    .await
                    .map_err(|e| GrepError(format!("读取元数据失败 [{}]: {e}", entry.path().display())))?;
                if file_meta.is_dir() {
                    if !SKIP_DIRS.contains(&name.as_str()) {
                        stack.push(entry.path());
                    }
                    continue;
                }
                if !file_meta.is_file() {
                    continue;
                }
                if file_meta.len() > MAX_FILE_BYTES {
                    continue;
                }
                // glob 过滤（只对文件名匹配，不对路径）
                if let Some(g) = glob {
                    if !glob_match(g, &name) {
                        continue;
                    }
                }
                if scanned_files >= MAX_SCAN_FILES {
                    skipped_scan_cap = true;
                    continue;
                }
                scanned_files += 1;

                let path = entry.path();
                let bytes = fs::read(&path)
                    .await
                    .map_err(|e| GrepError(format!("读取文件失败 [{}]: {e}", path.display())))?;
                if is_binary(&bytes) {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes).into_owned();

                // 计算命中行号
                let file_lines: Vec<&str> = text.lines().collect();
                let total_lines = file_lines.len();
                let hit_lines = if multiline {
                    collect_hits_multiline(&re, &text, total_lines)
                } else {
                    collect_hits_linebyline(&re, &file_lines)
                };
                if hit_lines.is_empty() {
                    continue;
                }

                let n_hits = hit_lines.len();
                let disp = display_path(&path, self.cwd.as_deref(), &root);
                match mode {
                    MODE_FILES => matched_files.push(disp),
                    MODE_COUNT => count_per_file.push((disp, n_hits)),
                    MODE_CONTENT => {
                        // 生成显示列表：(行号, 是否命中)，按行号升序
                        let show_lines: Vec<(usize, bool)> = if context > 0 {
                            let mut map: BTreeMap<usize, bool> = BTreeMap::new();
                            for &n in &hit_lines {
                                let lo = n.saturating_sub(context).max(1);
                                let hi = (n + context).min(total_lines);
                                for m in lo..=hi {
                                    map.entry(m).or_insert(false);
                                }
                            }
                            for &n in &hit_lines {
                                map.insert(n, true);
                            }
                            map.into_iter().collect()
                        } else {
                            hit_lines.iter().map(|&n| (n, true)).collect()
                        };
                        let all_lines: Vec<String> =
                            file_lines.iter().map(|s| s.to_string()).collect();
                        blocks.push((disp, total_lines, all_lines, show_lines));
                    }
                    _ => unreachable!(),
                }
            }
        }

        // ===== 5. 组装输出 =====
        let pattern_label = if case_sensitive {
            format!("/{}/", args.pattern)
        } else {
            format!("/{}/（不区分大小写）", args.pattern)
        };

        let files_with_hits = match mode {
            MODE_CONTENT => blocks.len(),
            MODE_FILES => matched_files.len(),
            MODE_COUNT => count_per_file.len(),
            _ => unreachable!(),
        };
        // 实际命中数（不含上下文行）：content 模式只统计 is_hit=true 的行
        let total_hits: usize = match mode {
            MODE_CONTENT => blocks
                .iter()
                .map(|(_, _, _, s)| s.iter().filter(|(_, h)| *h).count())
                .sum(),
            MODE_FILES => matched_files.len(),
            MODE_COUNT => count_per_file.iter().map(|(_, n)| *n).sum(),
            _ => unreachable!(),
        };

        if files_with_hits == 0 {
            return Ok(format!(
                "未找到匹配正则 {} 的内容（共扫描 {scanned_files} 个文件）",
                pattern_label
            ));
        }

        let mut out = String::with_capacity(estimate_capacity(mode, total_hits, max_matches));
        let mut shown = 0usize;
        let mut truncated = false;

        match mode {
            MODE_CONTENT => {
                for (path_display, total_lines, all_lines, show_lines) in &blocks {
                    if shown >= max_matches {
                        truncated = true;
                        break;
                    }
                    out.push_str(&format!("path: {path_display}（共 {total_lines} 行）\n"));
                    let width = line_number_width(*total_lines);
                    for (line_no, is_hit) in show_lines {
                        if shown >= max_matches {
                            truncated = true;
                            break;
                        }
                        let content = all_lines
                            .get(line_no.saturating_sub(1))
                            .map(String::as_str)
                            .unwrap_or("");
                        if *is_hit {
                            out.push_str(&format_numbered_line(*line_no, width, content));
                        } else {
                            // 上下文行：· 前缀标记，与命中行区分
                            out.push_str(&format!("· {line_no:>width$}  {content}"));
                        }
                        out.push('\n');
                        shown += 1;
                    }
                }
            }
            MODE_FILES => {
                for path_display in &matched_files {
                    if shown >= max_matches {
                        truncated = true;
                        break;
                    }
                    out.push_str(path_display);
                    out.push('\n');
                    shown += 1;
                }
            }
            MODE_COUNT => {
                for (path_display, n) in &count_per_file {
                    if shown >= max_matches {
                        truncated = true;
                        break;
                    }
                    out.push_str(&format!("{path_display}: {n} matches\n"));
                    shown += 1;
                }
            }
            _ => unreachable!(),
        }

        // 摘要头（行尾换行由各 mode 输出保证）
        let mut summary = match mode {
            MODE_CONTENT => format!(
                "搜索完成：正则 {}，共扫描 {scanned_files} 个文件，{files_with_hits} 个文件命中 {total_hits} 处，输出 {shown} 行（context={context}）：\n",
                pattern_label
            ),
            MODE_FILES => format!(
                "搜索完成：正则 {}，共扫描 {scanned_files} 个文件，{files_with_hits} 个文件命中：\n",
                pattern_label
            ),
            MODE_COUNT => format!(
                "搜索完成：正则 {}，共扫描 {scanned_files} 个文件，{files_with_hits} 个文件命中 {total_hits} 处：\n",
                pattern_label
            ),
            _ => unreachable!(),
        };
        if truncated {
            summary.push_str(&format!(
                "[已达最大显示数 {max_matches}，剩余结果已省略；可缩小范围或增大 max_matches]\n"
            ));
        }
        if skipped_scan_cap {
            summary.push_str(&format!(
                "[已达到扫描文件数上限 {MAX_SCAN_FILES}，部分文件未扫描]\n"
            ));
        }
        out.insert_str(0, &summary);
        Ok(out)
    }
}
