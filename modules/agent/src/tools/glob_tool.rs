//! glob 工具：按文件名模式递归匹配
//!
//! 递归遍历搜索根目录（默认工作区）下全部文件，按 glob 模式匹配文件名，
//! 返回匹配文件路径列表（相对工作区，`/` 分隔），按修改时间降序排列。
//!
//! **支持的 glob 语法**（自实现，不引入第三方 crate）：
//! - `*` 匹配除路径分隔符外的任意字符
//! - `**` 匹配任意层级目录（含 0 层，如 `**/*.rs` 同时命中根目录与任意子目录下的 .rs）
//! - `?` 匹配单个字符
//! - `[abc]` / `[a-z]` / `[^abc]` / `[!abc]` 字符集（含范围与取反）
//! - `{a,b,c}` 多选项（如 `*.{json,toml}`，支持多组与嵌套）
//!
//! 匹配分两阶段：① `compile_pattern` 把模式字符串编译为 `Vec<Alt>`（每种 brace
//! 展开一条备选，每条备选是一组 `PatSeg` 段）；② 遍历文件时对每个相对路径
//! 切段后调用 `match_path` 做段级回溯匹配。**段匹配基于 `char` 而非字节**，
//! 保证 `?` / `[...]` 对多字节（如中文）文件名正确。
//!
//! 工作区支持：构造时传入 `cwd: Option<PathBuf>`，相对路径 join 到 cwd。
//! 信任本地 agent 环境，路径不做沙箱限制。

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::SystemTime;

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::fs;

use super::resolve_path;

/// 默认最大返回文件数
const MAX_RESULTS: usize = 500;
/// 扫描条目硬上限（防止超大仓库长时间阻塞 / 内存膨胀）
const MAX_SCAN_ENTRIES: usize = 50_000;
/// 生成的/依赖目录，遍历时跳过
const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", "__pycache__", ".venv", "venv",
    ".pytest_cache", ".mypy_cache", ".next", ".turbo", ".nuxt", ".svelte-kit",
    ".output", "coverage",
];

/// 工具参数
///
/// 字段按大小降序：`String`（24B）= `Option<String>`（24B，NonNull niche 优化）
/// > `Option<usize>`（16B，usize 无 niche）。
#[derive(Deserialize)]
pub struct GlobArgs {
    /// glob 模式，如 `*.rs`、`src/**/*.ts`、`*.{json,toml}`
    pub pattern: String,
    /// 搜索根目录（绝对或相对工作区），默认工作区根目录
    #[serde(default)]
    pub path: Option<String>,
    /// 最大返回文件数，默认 500
    #[serde(default)]
    pub max_results: Option<usize>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("glob error: {0}")]
pub struct GlobError(String);

/// 文件名 glob 匹配工具
///
/// `cwd` 为可选工作区：设置后作为默认搜索根，相对路径以此为基准。
pub struct GlobTool {
    cwd: Option<PathBuf>,
}

impl GlobTool {
    pub fn new() -> Self {
        Self { cwd: None }
    }

    /// 指定工作区目录，默认在此目录下递归匹配
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self { cwd: Some(cwd) }
    }
}

impl Default for GlobTool {
    fn default() -> Self {
        Self::new()
    }
}

// ============ glob 模式编译 ============

/// 编译后的单段模式
enum PatSeg {
    /// `**`：匹配任意层级目录（含 0 层）
    StarStar,
    /// 普通段（已转 `char` 数组，含 `*` `?` `[...]`）
    Chars(Vec<char>),
}

/// 一种 brace 展开后的完整模式（一组段）
struct Alt(Vec<PatSeg>);

/// 把模式字符串编译为多个备选：归一化分隔符 → brace 展开 → 段切分 → 转换
fn compile_pattern(pattern: &str) -> Vec<Alt> {
    let norm = pattern.replace('\\', "/");
    expand_braces(&norm)
        .into_iter()
        .map(|alt| {
            Alt(alt
                .split('/')
                .filter(|s| !s.is_empty() && *s != ".")
                .map(|s| {
                    if s == "**" {
                        PatSeg::StarStar
                    } else {
                        PatSeg::Chars(s.chars().collect())
                    }
                })
                .collect())
        })
        .collect()
}

/// brace 展开：`*.{rs,toml}` → `["*.rs", "*.toml"]`；支持多组与嵌套（`{a,{b,c}}`）
fn expand_braces(pattern: &str) -> Vec<String> {
    let bytes = pattern.as_bytes();
    let Some(open) = bytes.iter().position(|&b| b == b'{') else {
        return vec![pattern.to_string()];
    };
    // 寻找匹配的 `}`（处理嵌套）
    let mut depth = 0i32;
    let mut close = None;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                close = Some(i);
                break;
            }
        }
    }
    let Some(close) = close else {
        // 无匹配 `}`：按字面量处理
        return vec![pattern.to_string()];
    };
    let prefix = &pattern[..open];
    let inner = &pattern[open + 1..close];
    let suffix = &pattern[close + 1..];
    let mut out: Vec<String> = Vec::with_capacity(4);
    for alt in split_top_level_commas(inner) {
        let combined = format!("{prefix}{alt}{suffix}");
        // 递归展开后续 brace 组（如 `{a,b}.{c,d}`）
        out.extend(expand_braces(&combined));
    }
    out
}

/// 顶层逗号分割（忽略嵌套 brace 内的逗号）
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = Vec::with_capacity(4);
    let mut depth = 0i32;
    let mut start = 0;
    for (i, b) in s.as_bytes().iter().enumerate() {
        match *b {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

// ============ 匹配 ============

/// 路径是否匹配编译后的模式（任一备选命中即 true）
fn match_path(alts: &[Alt], path_segs: &[Vec<char>]) -> bool {
    alts.iter().any(|alt| match_segments(&alt.0, path_segs))
}

/// 段级匹配：`**` 匹配 0+ 段（回溯），普通段用 `match_segment`
fn match_segments(pat: &[PatSeg], path: &[Vec<char>]) -> bool {
    let mut pi = 0usize;
    let mut si = 0usize;
    let mut star_pi: Option<usize> = None;
    let mut star_si = 0usize;
    while si < path.len() {
        match pat.get(pi) {
            // ** 可吃掉 0 个或多个段，记录回溯点
            Some(PatSeg::StarStar) => {
                star_pi = Some(pi);
                star_si = si;
                pi += 1;
            }
            Some(PatSeg::Chars(c)) if match_segment(&path[si], c) => {
                pi += 1;
                si += 1;
            }
            _ => {
                if let Some(spi) = star_pi {
                    // 回溯：让上一个 ** 多吃一个段
                    pi = spi + 1;
                    star_si += 1;
                    si = star_si;
                } else {
                    return false;
                }
            }
        }
    }
    // 剩余模式必须全是 **（可匹配 0 段）
    pat[pi..].iter().all(|s| matches!(s, PatSeg::StarStar))
}

/// 单段匹配：`*`（不跨 `/`）、`?`、`[...]`
///
/// 基于 `char` 而非字节，保证多字节字符被 `?` 正确匹配为一个码点。
fn match_segment(seg: &[char], pat: &[char]) -> bool {
    let mut si = 0usize;
    let mut pi = 0usize;
    let mut star_pi: Option<usize> = None;
    let mut star_si = 0usize;
    while si < seg.len() {
        let advanced = match pat.get(pi).copied() {
            Some('*') => {
                star_pi = Some(pi);
                star_si = si;
                pi += 1;
                continue; // * 可匹配 0 个字符，不前进 si
            }
            Some('?') => {
                pi += 1;
                si += 1;
                true
            }
            Some('[') => match match_class(pat, pi, seg[si]) {
                Some((true, next)) => {
                    pi = next;
                    si += 1;
                    true
                }
                Some((false, _)) => false,
                None => {
                    // 未闭合的 [ 当字面量
                    if seg[si] == '[' {
                        pi += 1;
                        si += 1;
                        true
                    } else {
                        false
                    }
                }
            },
            Some(c) if c == seg[si] => {
                pi += 1;
                si += 1;
                true
            }
            _ => false,
        };
        if !advanced {
            if let Some(spi) = star_pi {
                // 回溯：让上一个 * 多吃一个字符
                pi = spi + 1;
                star_si += 1;
                si = star_si;
            } else {
                return false;
            }
        }
    }
    // 消费尾部多余的 *
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}

/// 字符类匹配 `[abc]` / `[a-z]` / `[^abc]` / `[!abc]`
///
/// 返回 `Some((是否命中, 类结束后索引))`；未闭合返回 `None`（由调用方当字面量处理）。
#[inline]
fn match_class(pat: &[char], start: usize, c: char) -> Option<(bool, usize)> {
    let mut i = start + 1;
    let mut negate = false;
    if i < pat.len() && (pat[i] == '!' || pat[i] == '^') {
        negate = true;
        i += 1;
    }
    let mut matched = false;
    let mut first = true; // ] 紧跟在 [ 或 [^ 后视为字面量成员
    while i < pat.len() {
        if pat[i] == ']' && !first {
            return Some((matched ^ negate, i + 1));
        }
        first = false;
        // 范围 a-z
        if i + 2 < pat.len() && pat[i + 1] == '-' && pat[i + 2] != ']' {
            let lo = pat[i];
            let hi = pat[i + 2];
            if c >= lo && c <= hi {
                matched = true;
            }
            i += 3;
        } else {
            if pat[i] == c {
                matched = true;
            }
            i += 1;
        }
    }
    None
}

impl Tool for GlobTool {
    const NAME: &'static str = "glob";

    type Error = GlobError;
    type Args = GlobArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "按文件名 glob 模式递归搜索文件（不读内容），返回匹配文件路径列表（相对工作区，按修改时间降序）。\
             语法：`*`=任意字符（不含分隔符）；`**`=任意层级；`?`=单字符；`[abc]`/`[a-z]`/`[^abc]` 字符集；\
             `{{a,b,c}}` 多选项（如 `*.{{json,toml}}`）。自动跳过 .git/node_modules/target 等生成目录；\
             默认最多 {MAX_RESULTS} 个。分工：目录→list_files；内容→search_file/grep/search_codebase。"
        )
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "glob 模式，如 *.rs、src/**/*.ts、*.{json,toml}" },
                "path": { "type": "string", "description": "搜索根目录（绝对或相对工作区），默认工作区根" },
                "max_results": { "type": "integer", "description": "最大返回文件数，默认 500", "default": MAX_RESULTS }
            },
            "required": ["pattern"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let max_results = args.max_results.unwrap_or(MAX_RESULTS).max(1);
        let alts = compile_pattern(&args.pattern);
        // 空模式 / 仅分隔符：无可匹配段，直接报错
        if alts.iter().all(|a| a.0.is_empty()) {
            return Err(GlobError(format!(
                "pattern 不能为空或仅含分隔符：[{}]",
                args.pattern
            )));
        }

        // 搜索根：path 参数优先，否则工作区根（无 cwd 时回退进程 cwd）
        let root = match &args.path {
            Some(p) => resolve_path(p, self.cwd.as_deref()),
            None => self.cwd.clone().unwrap_or_else(|| PathBuf::from(".")),
        };
        let root_meta = fs::metadata(&root)
            .await
            .map_err(|e| GlobError(format!("访问搜索根目录失败 [{}]: {e}", root.display())))?;
        if !root_meta.is_dir() {
            return Err(GlobError(format!(
                "搜索根不是目录 [{}]",
                root.display()
            )));
        }

        // 栈式遍历（非递归），跳过 SKIP_DIRS 与符号链接（避免环）
        let mut stack: Vec<PathBuf> = vec![root.clone()];
        let mut matches: Vec<(SystemTime, String)> = Vec::with_capacity(max_results.min(256));
        let mut scanned: usize = 0;
        let mut capped = false;

        'walk: while let Some(dir) = stack.pop() {
            let mut rd = fs::read_dir(&dir)
                .await
                .map_err(|e| GlobError(format!("读取目录失败 [{}]: {e}", dir.display())))?;
            while let Some(entry) = rd
                .next_entry()
                .await
                .map_err(|e| GlobError(format!("读取目录条目失败 [{}]: {e}", dir.display())))?
            {
                let ftype = entry.file_type().await.map_err(|e| {
                    GlobError(format!("读取文件类型失败 [{}]: {e}", entry.path().display()))
                })?;
                if ftype.is_symlink() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if ftype.is_dir() {
                    if !SKIP_DIRS.contains(&name.as_str()) {
                        stack.push(entry.path());
                    }
                    continue;
                }
                if !ftype.is_file() {
                    continue;
                }
                if scanned >= MAX_SCAN_ENTRIES {
                    capped = true;
                    break 'walk;
                }
                scanned += 1;

                let path = entry.path();

                // 匹配用路径：相对搜索根（glob 模式基于搜索根内的相对结构匹配，
                // 如 path=sub 时模式 *.rs 应命中 sub/inner.rs 的 "inner.rs" 段）
                let match_rel = path
                    .strip_prefix(&root)
                    .map(|r| r.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| path.to_string_lossy().into_owned());
                let match_rel = if cfg!(windows) {
                    match_rel.replace('\\', "/")
                } else {
                    match_rel
                };

                // 段切分并匹配
                let segs: Vec<Vec<char>> = match_rel
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.chars().collect())
                    .collect();
                if !match_path(&alts, &segs) {
                    continue;
                }

                // 展示路径：优先相对工作区 cwd（让 LLM 回传 read_file/edit_file 时路径一致，
                // 避免 path=sub 时返回 "inner.rs" 导致 read_file 解析为 cwd/inner.rs 找不到），
                // 其次相对搜索根，最后用绝对路径。Windows 下统一为 / 分隔。
                let display_rel = self
                    .cwd
                    .as_deref()
                    .and_then(|cwd| path.strip_prefix(cwd).ok())
                    .or_else(|| path.strip_prefix(&root).ok())
                    .map(|r| r.display().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let display_rel = if cfg!(windows) {
                    display_rel.replace('\\', "/")
                } else {
                    display_rel
                };

                let mtime = entry
                    .metadata()
                    .await
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                matches.push((mtime, display_rel));
            }
        }

        // 按修改时间降序；同时间按路径升序，保证输出稳定可复现
        // 用 then_with 惰性求值次级比较，首键已决出顺序时跳过 String 比较
        matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

        let total_matched = matches.len();
        let shown = total_matched.min(max_results);
        let truncated = total_matched > max_results;
        let width = shown.to_string().len();

        let root_display = {
            let s = root.to_string_lossy().into_owned();
            if cfg!(windows) {
                s.replace('\\', "/")
            } else {
                s
            }
        };

        if total_matched == 0 {
            return Ok(format!(
                "未找到匹配 {} 的文件（扫描 {} 个文件，根目录 {}）",
                args.pattern, scanned, root_display
            ));
        }

        // 输出：序号 + 路径（每行一条），尾部附统计信息
        let mut out = String::with_capacity(shown * 48 + 160);
        for (i, (_, path)) in matches.iter().take(max_results).enumerate() {
            // 行号右对齐到 shown 位数，两空格分隔
            let _ = write!(out, "{:>width$}  {}\n", i + 1, path);
        }
        out.push('\n');
        if truncated {
            let _ = write!(
                out,
                "输出 {} 个文件（已达 max_results 上限 {}，共匹配 {}，扫描 {} 个，模式 {}，根目录 {}）",
                shown, max_results, total_matched, scanned, args.pattern, root_display
            );
        } else {
            let _ = write!(
                out,
                "匹配 {} 个文件（扫描 {} 个，模式 {}，根目录 {}）",
                total_matched, scanned, args.pattern, root_display
            );
        }
        if capped {
            let _ = write!(out, "\n[已达到扫描条目上限 {}，部分目录未扫描]", MAX_SCAN_ENTRIES);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("effisuite-glob-test-{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn glob_basic_pattern() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        std::fs::write(dir.join("b.txt"), "x").unwrap();
        std::fs::write(dir.join("c.md"), "x").unwrap();

        let tool = GlobTool::with_cwd(dir.clone());
        let out = tool
            .call(GlobArgs {
                pattern: "*.txt".to_string(),
                path: None,
                max_results: None,
            })
            .await
            .unwrap();
        // glob 按修改时间降序排序（最近修改的在前），不保证文件名字典序。
        // 仅断言两个 .txt 文件都出现、c.md 不出现、计数正确，不假设顺序。
        assert!(out.contains("a.txt"), "out: {out}");
        assert!(out.contains("b.txt"), "out: {out}");
        assert!(!out.contains("c.md"), "out: {out}");
        assert!(out.contains("匹配 2 个文件"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn glob_recursive_double_star() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("src/nested")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "x").unwrap();
        std::fs::write(dir.join("src/nested/lib.rs"), "x").unwrap();
        std::fs::write(dir.join("README.md"), "x").unwrap();
        // 根目录下的 .rs 也应被 **/*.rs 命中（** 匹配 0 层）
        std::fs::write(dir.join("root.rs"), "x").unwrap();

        let tool = GlobTool::with_cwd(dir.clone());
        let out = tool
            .call(GlobArgs {
                pattern: "**/*.rs".to_string(),
                path: None,
                max_results: None,
            })
            .await
            .unwrap();
        assert!(out.contains("src/main.rs"), "out: {out}");
        assert!(out.contains("src/nested/lib.rs"), "out: {out}");
        assert!(out.contains("root.rs"), "out: {out}");
        assert!(!out.contains("README.md"), "out: {out}");
        assert!(out.contains("匹配 3 个文件"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn glob_brace_options() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "x").unwrap();
        std::fs::write(dir.join("b.toml"), "x").unwrap();
        std::fs::write(dir.join("c.json"), "x").unwrap();

        let tool = GlobTool::with_cwd(dir.clone());
        let out = tool
            .call(GlobArgs {
                pattern: "*.{rs,toml}".to_string(),
                path: None,
                max_results: None,
            })
            .await
            .unwrap();
        assert!(out.contains("a.rs"), "out: {out}");
        assert!(out.contains("b.toml"), "out: {out}");
        assert!(!out.contains("c.json"), "out: {out}");
        assert!(out.contains("匹配 2 个文件"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn glob_no_match() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();

        let tool = GlobTool::with_cwd(dir.clone());
        let out = tool
            .call(GlobArgs {
                pattern: "*.xyz".to_string(),
                path: None,
                max_results: None,
            })
            .await
            .unwrap();
        assert!(out.contains("未找到"), "out: {out}");
        assert!(out.contains("扫描 1 个文件"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn glob_skips_generated_dirs() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::write(dir.join(".git/secret.rs"), "x").unwrap();
        std::fs::write(dir.join("target/debug/out.rs"), "x").unwrap();
        std::fs::write(dir.join("visible.rs"), "x").unwrap();

        let tool = GlobTool::with_cwd(dir.clone());
        let out = tool
            .call(GlobArgs {
                pattern: "**/*.rs".to_string(),
                path: None,
                max_results: None,
            })
            .await
            .unwrap();
        assert!(out.contains("visible.rs"), "out: {out}");
        assert!(!out.contains("secret.rs"), "out: {out}");
        assert!(!out.contains("target"), "out: {out}");
        assert!(out.contains("匹配 1 个文件"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn glob_max_results_truncation() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..6 {
            std::fs::write(dir.join(format!("f{i}.txt")), "x").unwrap();
        }

        let tool = GlobTool::with_cwd(dir.clone());
        let out = tool
            .call(GlobArgs {
                pattern: "*.txt".to_string(),
                path: None,
                max_results: Some(3),
            })
            .await
            .unwrap();
        // 恰好 3 个编号行
        let numbered = out
            .lines()
            .filter(|l| !l.is_empty() && l.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
            .count();
        assert_eq!(numbered, 3, "out: {out}");
        assert!(out.contains("已达 max_results 上限 3"), "out: {out}");
        assert!(out.contains("共匹配 6"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn glob_question_and_char_class() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("file1.txt"), "x").unwrap();
        std::fs::write(dir.join("file2.txt"), "x").unwrap();
        std::fs::write(dir.join("file3.log"), "x").unwrap();
        std::fs::write(dir.join("fileX.txt"), "x").unwrap();

        let tool = GlobTool::with_cwd(dir.clone());

        // ? 匹配单个字符
        let out = tool
            .call(GlobArgs {
                pattern: "file?.txt".to_string(),
                path: None,
                max_results: None,
            })
            .await
            .unwrap();
        assert!(out.contains("file1.txt"), "out: {out}");
        assert!(out.contains("fileX.txt"), "out: {out}");
        assert!(out.contains("匹配 3 个文件"), "out: {out}");

        // [12] 字符集
        let out = tool
            .call(GlobArgs {
                pattern: "file[12].txt".to_string(),
                path: None,
                max_results: None,
            })
            .await
            .unwrap();
        assert!(out.contains("file1.txt"), "out: {out}");
        assert!(out.contains("file2.txt"), "out: {out}");
        assert!(!out.contains("fileX.txt"), "out: {out}");
        assert!(out.contains("匹配 2 个文件"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn glob_subdir_pattern() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("src/a")).unwrap();
        std::fs::write(dir.join("src/a/m.rs"), "x").unwrap();
        std::fs::write(dir.join("src/b.rs"), "x").unwrap();

        let tool = GlobTool::with_cwd(dir.clone());
        // src/**/*.rs：src 直接子文件 + 任意深层
        let out = tool
            .call(GlobArgs {
                pattern: "src/**/*.rs".to_string(),
                path: None,
                max_results: None,
            })
            .await
            .unwrap();
        assert!(out.contains("src/b.rs"), "out: {out}");
        assert!(out.contains("src/a/m.rs"), "out: {out}");
        assert!(out.contains("匹配 2 个文件"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn glob_empty_pattern_errors() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let tool = GlobTool::with_cwd(dir.clone());
        let r = tool
            .call(GlobArgs {
                pattern: "".to_string(),
                path: None,
                max_results: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("pattern"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // ============ 纯匹配函数单测（不依赖文件系统） ============

    #[test]
    fn match_segment_basic() {
        let s = |x: &str| x.chars().collect::<Vec<char>>();
        assert!(match_segment(&s("main.rs"), &s("*.rs")));
        assert!(!match_segment(&s("main.rs"), &s("*.ts")));
        assert!(match_segment(&s("a"), &s("?")));
        assert!(!match_segment(&s("ab"), &s("?")));
        assert!(match_segment(&s("file1"), &s("file[12]")));
        assert!(!match_segment(&s("file3"), &s("file[12]")));
        assert!(match_segment(&s("filex"), &s("file[a-z]")));
        assert!(match_segment(&s("fileX"), &s("file[!a-z]")));
        assert!(match_segment(&s("fileX"), &s("file[^a-z]")));
        // 未闭合 [ 当字面量
        assert!(match_segment(&s("a[b"), &s("a[b")));
    }

    #[test]
    fn match_path_double_star() {
        let alt = compile_pattern("**/*.rs");
        let p = |x: &str| {
            x.split('/')
                .filter(|s| !s.is_empty())
                .map(|s| s.chars().collect::<Vec<char>>())
                .collect::<Vec<_>>()
        };
        assert!(match_path(&alt, &p("main.rs"))); // ** 匹配 0 层
        assert!(match_path(&alt, &p("src/main.rs")));
        assert!(match_path(&alt, &p("src/nested/main.rs")));
        assert!(!match_path(&alt, &p("main.ts")));
    }

    #[test]
    fn match_path_root_only_star() {
        let alt = compile_pattern("*.rs");
        let p = |x: &str| {
            x.split('/')
                .filter(|s| !s.is_empty())
                .map(|s| s.chars().collect::<Vec<char>>())
                .collect::<Vec<_>>()
        };
        assert!(match_path(&alt, &p("main.rs")));
        assert!(!match_path(&alt, &p("src/main.rs"))); // * 不跨 /
    }

    #[test]
    fn expand_braces_multi() {
        assert_eq!(expand_braces("*.{rs,toml}"), vec!["*.rs", "*.toml"]);
        assert_eq!(expand_braces("{a,b}.txt"), vec!["a.txt", "b.txt"]);
        // 多组
        let mut r = expand_braces("{x,y}_{1,2}");
        r.sort();
        assert_eq!(r, vec!["x_1", "x_2", "y_1", "y_2"]);
        // 嵌套
        let mut r = expand_braces("{a,{b,c}}.rs");
        r.sort();
        assert_eq!(r, vec!["a.rs", "b.rs", "c.rs"]);
        // 无 brace
        assert_eq!(expand_braces("plain.rs"), vec!["plain.rs"]);
        // 未闭合
        assert_eq!(expand_braces("{a,b"), vec!["{a,b"]);
    }
}
