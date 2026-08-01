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

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use regex::Regex;
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::fs;

use super::resolve_path;
use super::text_utils::{format_numbered_line, line_number_width};

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

/// 工具参数
///
/// 字段按大小降序排列：String / Option<String>（24B）→ Option<usize>（16B）
/// → Option<bool>（2B），最小化结构体对齐 padding。
#[derive(Deserialize)]
pub struct GrepArgs {
    /// 正则表达式（regex crate 语法，如 `fn \w+`、`TODO\(.*\)`）
    pub pattern: String,
    /// 搜索根目录（绝对或相对工作区），默认工作区根目录
    #[serde(default)]
    pub path: Option<String>,
    /// 输出模式："content" | "files_with_matches" | "count"，默认 "content"
    #[serde(default)]
    pub output_mode: Option<String>,
    /// 文件名 glob 过滤模式（如 `*.rs`），只对文件名匹配，不对路径；默认不过滤
    #[serde(default)]
    pub glob: Option<String>,
    /// 上下文行数（命中行前后各显示 N 行），默认 0；上下文行以 `·` 前缀标记
    #[serde(default)]
    pub context: Option<usize>,
    /// 最多返回命中数（content 限制显示行数，其他模式限制文件数），默认 300
    #[serde(default)]
    pub max_matches: Option<usize>,
    /// 是否区分大小写，默认 false（不区分）
    #[serde(default)]
    pub case_sensitive: Option<bool>,
    /// 多行模式：true 时正则匹配整篇文本（可跨行），`^`/`$` 匹配行边界；
    /// 默认 false（逐行匹配，标准 grep 行为）
    #[serde(default)]
    pub multiline: Option<bool>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("grep error: {0}")]
pub struct GrepError(String);

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

/// 判断是否为二进制文件：探测前 8KB 是否含 NUL 字节
#[inline]
fn is_binary(bytes: &[u8]) -> bool {
    let probe = &bytes[..bytes.len().min(8192)];
    probe.contains(&0)
}

/// 简单 glob 匹配（支持 `*` 任意序列、`?` 单字符），只对文件名匹配，不对路径。
/// 采用经典双指针 + 回溯算法，O(n) 时间 O(1) 空间。
#[inline]
fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let mut pi = 0usize;
    let mut ni = 0usize;
    // star = (pattern 中 * 之后的位置, name 中回退重试的位置)
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi + 1, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp;
            star = Some((sp, sn + 1));
            ni = sn + 1;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// 二分查找：返回 `offset` 落在第几行（0-based）。
/// `line_starts` 为每行起始字节偏移（升序，首行始终为 0）。
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

/// 计算每行起始字节偏移（含首行 0），用于多行模式下把匹配字节区间映射回行号。
/// 单次遍历完成收集，按预估行数预分配容量。
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

/// 逐行匹配：返回命中行号（1-based，升序），最多 MAX_MATCHES_PER_FILE 个。
#[inline]
fn collect_hits_linebyline(re: &Regex, file_lines: &[&str]) -> Vec<usize> {
    let mut hits = Vec::new();
    for (i, line) in file_lines.iter().enumerate() {
        if re.is_match(line) {
            hits.push(i + 1);
            if hits.len() >= MAX_MATCHES_PER_FILE {
                break;
            }
        }
    }
    hits
}

/// 多行匹配：对整篇文本 `find_iter`，把每个匹配的字节区间映射到覆盖的行号集合
/// （1-based，升序去重），最多 MAX_MATCHES_PER_FILE 个。
fn collect_hits_multiline(re: &Regex, text: &str, total_lines: usize) -> Vec<usize> {
    if total_lines == 0 {
        return Vec::new();
    }
    let line_starts = compute_line_starts(text);
    let mut set: BTreeSet<usize> = BTreeSet::new();
    for m in re.find_iter(text) {
        let start_line = line_index_of(&line_starts, m.start()) + 1;
        // 匹配可能跨多行：把覆盖的所有行都标记为命中；空匹配按起始行计
        let end_line = if m.end() > m.start() {
            line_index_of(&line_starts, m.end() - 1) + 1
        } else {
            start_line
        };
        for ln in start_line..=end_line {
            set.insert(ln);
            if set.len() >= MAX_MATCHES_PER_FILE {
                return set.into_iter().collect();
            }
        }
    }
    set.into_iter().collect()
}

/// 把绝对/相对路径转为相对搜索根的展示路径，Windows 下统一为 `/` 分隔，
/// 方便 LLM 直接回传给 read_file / edit_file 使用。
#[inline]
fn display_path(path: &std::path::Path, root: &std::path::Path) -> String {
    let display = path
        .strip_prefix(root)
        .map(|r| r.display().to_string())
        .unwrap_or_else(|_| path.display().to_string());
    if cfg!(windows) {
        display.replace('\\', "/")
    } else {
        display
    }
}

/// 按 mode + 命中规模预估输出缓冲区容量，减少 String 扩容拷贝。
#[inline]
fn estimate_capacity(mode: &str, total_hits: usize, max_matches: usize) -> usize {
    let n = total_hits.min(max_matches);
    match mode {
        MODE_CONTENT => n * 96 + 256,
        MODE_FILES => n * 64 + 256,
        MODE_COUNT => n * 80 + 256,
        _ => 256,
    }
}

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
                let disp = display_path(&path, &root);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("effisuite-grep-test-{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn grep_basic_regex_match() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("src/nested")).unwrap();
        std::fs::write(
            dir.join("src/main.rs"),
            "fn main() {\n    let x = 1;\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src/nested/lib.rs"),
            "pub fn helper() {\n    return 42;\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("README.md"), "no match here\n").unwrap();
        // 生成目录应被跳过
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::write(dir.join("target/debug/out.rs"), "fn skipped() {}\n").unwrap();

        let tool = GrepTool::with_cwd(dir.clone());
        let out = tool
            .call(GrepArgs {
                pattern: r"fn \w+".to_string(),
                path: None,
                output_mode: None,
                glob: None,
                context: None,
                max_matches: None,
                case_sensitive: None,
                multiline: None,
            })
            .await
            .unwrap();

        assert!(out.contains("2 个文件命中"), "out: {out}");
        assert!(out.contains("path: src/main.rs"), "out: {out}");
        assert!(out.contains("fn main() {"), "out: {out}");
        assert!(out.contains("path: src/nested/lib.rs"), "out: {out}");
        assert!(out.contains("pub fn helper() {"), "out: {out}");
        // README 不含 fn 定义
        assert!(!out.contains("README.md"), "out: {out}");
        // target 目录被跳过
        assert!(!out.contains("target"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn grep_case_insensitive_default() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "Hello World\nhello rust\nHELLO there\n").unwrap();

        let tool = GrepTool::with_cwd(dir.clone());

        // 默认不区分大小写：三行都命中
        let out = tool
            .call(GrepArgs {
                pattern: "hello".to_string(),
                path: None,
                output_mode: None,
                glob: None,
                context: None,
                max_matches: None,
                case_sensitive: None,
                multiline: None,
            })
            .await
            .unwrap();
        assert!(out.contains("命中 3 处"), "out: {out}");
        assert!(
            out.contains("Hello World") && out.contains("hello rust") && out.contains("HELLO there"),
            "out: {out}"
        );

        // 区分大小写：只有 hello rust 命中
        let out = tool
            .call(GrepArgs {
                pattern: "hello".to_string(),
                path: None,
                output_mode: None,
                glob: None,
                context: None,
                max_matches: None,
                case_sensitive: Some(true),
                multiline: None,
            })
            .await
            .unwrap();
        assert!(out.contains("命中 1 处"), "out: {out}");
        assert!(out.contains("hello rust"), "out: {out}");
        assert!(!out.contains("Hello World"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn grep_files_with_matches_mode() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.rs"), "fn alpha() {}\n").unwrap();
        std::fs::write(dir.join("sub/b.rs"), "fn beta() {}\nno match\n").unwrap();
        std::fs::write(dir.join("c.txt"), "nothing\n").unwrap();

        let tool = GrepTool::with_cwd(dir.clone());
        let out = tool
            .call(GrepArgs {
                pattern: r"fn \w+".to_string(),
                path: None,
                output_mode: Some("files_with_matches".to_string()),
                glob: None,
                context: None,
                max_matches: None,
                case_sensitive: None,
                multiline: None,
            })
            .await
            .unwrap();

        assert!(out.contains("2 个文件命中"), "out: {out}");
        assert!(out.contains("a.rs"), "out: {out}");
        assert!(out.contains("sub/b.rs"), "out: {out}");
        assert!(!out.contains("c.txt"), "out: {out}");
        // files_with_matches 不应输出行内容
        assert!(!out.contains("fn alpha"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn grep_count_mode() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "foo\nfoo bar\nbaz\nfoo\n").unwrap();
        std::fs::write(dir.join("b.txt"), "baz\nqux\n").unwrap();

        let tool = GrepTool::with_cwd(dir.clone());
        let out = tool
            .call(GrepArgs {
                pattern: "foo".to_string(),
                path: None,
                output_mode: Some("count".to_string()),
                glob: None,
                context: None,
                max_matches: None,
                case_sensitive: None,
                multiline: None,
            })
            .await
            .unwrap();

        assert!(out.contains("a.txt: 3 matches"), "out: {out}");
        // b.txt 无命中，不应出现
        assert!(!out.contains("b.txt"), "out: {out}");
        assert!(out.contains("1 个文件命中"), "out: {out}");
        assert!(out.contains("3 处"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn grep_glob_filter_rs_only() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn rust_fn() {}\n").unwrap();
        std::fs::write(dir.join("b.go"), "fn go_fn() {}\n").unwrap();
        std::fs::write(dir.join("c.rs"), "fn another_fn() {}\n").unwrap();
        std::fs::write(dir.join("d.txt"), "fn txt_fn() {}\n").unwrap();

        let tool = GrepTool::with_cwd(dir.clone());
        let out = tool
            .call(GrepArgs {
                pattern: r"fn \w+".to_string(),
                path: None,
                output_mode: Some("files_with_matches".to_string()),
                glob: Some("*.rs".to_string()),
                context: None,
                max_matches: None,
                case_sensitive: None,
                multiline: None,
            })
            .await
            .unwrap();

        assert!(out.contains("2 个文件命中"), "out: {out}");
        assert!(out.contains("a.rs"), "out: {out}");
        assert!(out.contains("c.rs"), "out: {out}");
        // .go 和 .txt 被过滤掉
        assert!(!out.contains("b.go"), "out: {out}");
        assert!(!out.contains("d.txt"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn grep_context_lines_display() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "l1\nl2\ntokio hit\nl4\nl5\n").unwrap();
        let tool = GrepTool::with_cwd(dir.clone());
        let out = tool
            .call(GrepArgs {
                pattern: "tokio".to_string(),
                path: None,
                output_mode: None,
                glob: None,
                context: Some(1),
                max_matches: None,
                case_sensitive: None,
                multiline: None,
            })
            .await
            .unwrap();
        // 上下文行以 · 前缀标记，命中行保持原格式；path 行附上文件总行数
        assert!(out.contains("· 2  l2"), "out: {out}");
        assert!(out.contains("3  tokio hit"), "out: {out}");
        assert!(out.contains("· 4  l4"), "out: {out}");
        assert!(out.contains("a.txt（共 5 行）"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn grep_no_match_returns_message() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "nothing here\njust text\n").unwrap();
        let tool = GrepTool::with_cwd(dir.clone());

        let out = tool
            .call(GrepArgs {
                pattern: r"\d{4}-\d{2}-\d{2}".to_string(),
                path: None,
                output_mode: None,
                glob: None,
                context: None,
                max_matches: None,
                case_sensitive: None,
                multiline: None,
            })
            .await
            .unwrap();
        assert!(out.contains("未找到"), "out: {out}");
        assert!(out.contains("共扫描"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn grep_invalid_regex_errors() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let tool = GrepTool::with_cwd(dir.clone());

        // 未闭合的分组
        let r = tool
            .call(GrepArgs {
                pattern: r"fn (\w+".to_string(),
                path: None,
                output_mode: None,
                glob: None,
                context: None,
                max_matches: None,
                case_sensitive: None,
                multiline: None,
            })
            .await;
        assert!(r.is_err());
        let msg = r.unwrap_err().to_string();
        assert!(msg.contains("正则表达式编译失败"), "msg: {msg}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn grep_invalid_output_mode_errors() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let tool = GrepTool::with_cwd(dir.clone());

        let r = tool
            .call(GrepArgs {
                pattern: "foo".to_string(),
                path: None,
                output_mode: Some("bogus_mode".to_string()),
                glob: None,
                context: None,
                max_matches: None,
                case_sensitive: None,
                multiline: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("output_mode"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn grep_skips_binary_files() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // 含 NUL 字节的"二进制"文件
        std::fs::write(dir.join("bin.dat"), b"\x00\x01\x02tokio\x00\x03").unwrap();
        std::fs::write(dir.join("a.txt"), "tokio text\n").unwrap();

        let tool = GrepTool::with_cwd(dir.clone());
        let out = tool
            .call(GrepArgs {
                pattern: "tokio".to_string(),
                path: None,
                output_mode: None,
                glob: None,
                context: None,
                max_matches: None,
                case_sensitive: None,
                multiline: None,
            })
            .await
            .unwrap();
        assert!(out.contains("path: a.txt"), "out: {out}");
        assert!(!out.contains("bin.dat"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn grep_respects_subdir_path() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("root.txt"), "tokio root\n").unwrap();
        std::fs::write(dir.join("sub/inner.txt"), "tokio inner\n").unwrap();

        let tool = GrepTool::with_cwd(dir.clone());
        let out = tool
            .call(GrepArgs {
                pattern: "tokio".to_string(),
                path: Some("sub".to_string()),
                output_mode: None,
                glob: None,
                context: None,
                max_matches: None,
                case_sensitive: None,
                multiline: None,
            })
            .await
            .unwrap();
        assert!(out.contains("path: inner.txt"), "out: {out}");
        assert!(!out.contains("root.txt"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn grep_multiline_cross_line_match() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // 跨行匹配：foo 后跟换行再跟 bar
        std::fs::write(dir.join("a.txt"), "foo\nbar\nbaz\n").unwrap();
        std::fs::write(dir.join("b.txt"), "foo bar\nsingle\n").unwrap();

        let tool = GrepTool::with_cwd(dir.clone());
        // multiline=true：foo\nbar 可跨行匹配
        let out = tool
            .call(GrepArgs {
                pattern: "foo\\nbar".to_string(),
                path: None,
                output_mode: Some("files_with_matches".to_string()),
                glob: None,
                context: None,
                max_matches: None,
                case_sensitive: None,
                multiline: Some(true),
            })
            .await
            .unwrap();
        // a.txt 含跨行的 foo\nbar，b.txt 不含（其 foo bar 在同一行无换行）
        assert!(out.contains("a.txt"), "out: {out}");
        assert!(!out.contains("b.txt"), "out: {out}");

        // 同样的正则在逐行模式下匹配不到（每行单独匹配，无行内含 foo\nbar）
        let out2 = tool
            .call(GrepArgs {
                pattern: "foo\\nbar".to_string(),
                path: None,
                output_mode: Some("files_with_matches".to_string()),
                glob: None,
                context: None,
                max_matches: None,
                case_sensitive: None,
                multiline: None,
            })
            .await
            .unwrap();
        assert!(out2.contains("未找到"), "out: {out2}");

        std::fs::remove_dir_all(&dir).ok();
    }

    // ============ 纯函数单元测试 ============

    #[test]
    fn glob_match_basic() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(glob_match("*.rs", "a.rs"));
        assert!(!glob_match("*.rs", "main.go"));
        assert!(!glob_match("*.rs", "rust"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(glob_match("*.test.ts", "foo.test.ts"));
        assert!(!glob_match("*.test.ts", "foo.test.js"));
        // 多段 *
        assert!(glob_match("*main*", "src/main.rs"));
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(!glob_match("src/*.rs", "main.rs"));
    }

    #[test]
    fn line_index_of_basic() {
        // 单行文本，line_starts = [0]
        let starts = vec![0usize];
        assert_eq!(line_index_of(&starts, 0), 0);

        // "ab\ncd\nef" → line_starts = [0, 3, 6]
        let starts = vec![0, 3, 6];
        assert_eq!(line_index_of(&starts, 0), 0);
        assert_eq!(line_index_of(&starts, 2), 0);
        assert_eq!(line_index_of(&starts, 3), 1);
        assert_eq!(line_index_of(&starts, 5), 1);
        assert_eq!(line_index_of(&starts, 6), 2);
        assert_eq!(line_index_of(&starts, 8), 2);
    }

    #[test]
    fn compute_line_starts_correct() {
        let text = "ab\ncd\nef";
        let starts = compute_line_starts(text);
        assert_eq!(starts, vec![0, 3, 6]);

        let empty = "";
        assert_eq!(compute_line_starts(empty), vec![0]);

        let no_newline = "abc";
        assert_eq!(compute_line_starts(no_newline), vec![0]);

        let trailing = "a\nb\n";
        assert_eq!(compute_line_starts(trailing), vec![0, 2, 4]);
    }

    #[test]
    fn is_binary_detection() {
        assert!(!is_binary(b"plain text file"));
        assert!(!is_binary(b""));
        assert!(is_binary(b"\x00binary"));
        assert!(is_binary(b"text\x00more"));
        // NUL 在 8KB 之外不算二进制（探测前 8KB）
        let mut big = vec![b'a'; 9000];
        big.push(0);
        assert!(!is_binary(&big));
    }
}