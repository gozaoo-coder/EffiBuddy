//! search_file 工具：工作区全文搜索
//!
//! 递归遍历搜索根目录（默认工作区）下**全部文本文件**，对每一行做关键词匹配，
//! 返回命中行的"文件路径 + 行号 + 行内容"，行号格式与 read_file 完全一致，
//! 方便 LLM 直接引用行号调用 edit_file 精确编辑：
//!
//! ```text
//! path: src/main.rs
//!   12  fn main() {
//!   45  let x = 1;   // 命中关键词
//! ```
//!
//! - 关键词为**数组**：一行包含**任意一个**关键词即命中；
//!   `match_all=true` 时需包含全部关键词
//! - 默认不区分大小写（`case_sensitive=true` 时区分）
//! - 自动跳过生成目录（.git / node_modules / target / dist 等）、
//!   二进制文件（NUL 字节探测）与超大文件（> 4 MiB）
//!
//! 工作区支持：构造时传入 `cwd: Option<PathBuf>`，默认在工作区根目录下搜索；
//! 也可用 `path` 指定子目录。信任本地 agent 环境，路径不做沙箱限制。

use std::collections::BTreeMap;
use std::path::PathBuf;
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::fs;

use super::resolve_path;
use super::text_utils::{format_numbered_line, line_number_width};

/// 默认最大返回命中行数
const DEFAULT_MAX_MATCHES: usize = 300;
/// 单文件最大命中行数（防止单个大文件刷屏）
const MAX_MATCHES_PER_FILE: usize = 100;
/// 扫描文件数硬上限（防止超大仓库长时间阻塞）
const MAX_SCAN_FILES: usize = 20_000;
/// 跳过大于该字节数的文件（4 MiB）
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// 生成的/依赖目录，搜索时跳过
const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", "__pycache__", ".venv", "venv",
    ".pytest_cache", ".mypy_cache", ".next", ".turbo", ".nuxt", ".svelte-kit",
    ".output", "coverage",
];

/// 工具参数
#[derive(Deserialize)]
pub struct SearchFileArgs {
    /// 关键词数组：一行包含任意一个关键词即命中（match_all=true 时需包含全部）
    pub keywords: Vec<String>,
    /// 搜索根目录（绝对或相对工作区），默认工作区根目录
    #[serde(default)]
    pub path: Option<String>,
    /// true = 一行需包含所有关键词；false = 包含任意一个即可，默认 false
    #[serde(default)]
    pub match_all: Option<bool>,
    /// 是否区分大小写，默认 false（不区分）
    #[serde(default)]
    pub case_sensitive: Option<bool>,
    /// 最多返回命中行数，默认 300
    #[serde(default)]
    pub max_matches: Option<usize>,
    /// 命中行前后各显示多少行上下文（默认 0 = 只显示命中行；上下文行以 · 前缀标记）
    #[serde(default)]
    pub context: Option<usize>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("search_file error: {0}")]
pub struct SearchFileError(String);

/// 工作区全文搜索工具
///
/// `cwd` 为可选工作区：设置后作为默认搜索根，相对路径以此为基准。
pub struct SearchFileTool {
    cwd: Option<PathBuf>,
}

impl SearchFileTool {
    pub fn new() -> Self {
        Self { cwd: None }
    }

    /// 指定工作区目录，默认在此目录下全文搜索
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self { cwd: Some(cwd) }
    }
}

impl Default for SearchFileTool {
    fn default() -> Self {
        Self::new()
    }
}

/// 单个文件的命中块：(相对展示路径, 文件总行数, 全文行列表, Vec<(行号, 是否命中)>)
/// 第 3 项总填充（取行内容统一走全文行），第 4 项 context=0 时即命中行、>0 时含上下文行。
type HitBlock = (String, usize, Vec<String>, Vec<(usize, bool)>);

/// 判断是否为二进制文件：探测前 8KB 是否含 NUL 字节
fn is_binary(bytes: &[u8]) -> bool {
    let probe = &bytes[..bytes.len().min(8192)];
    probe.contains(&0)
}

impl Tool for SearchFileTool {
    const NAME: &'static str = "search_file";

    type Error = SearchFileError;
    type Args = SearchFileArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "在工作区目录下递归全文搜索：对全部文本文件逐行做字面关键词匹配（非正则），返回命中行的文件路径+行号+内容。\
             keywords 为数组，一行含任意一个即命中（match_all=true 需全含）；\
             默认不区分大小写（case_sensitive=true 区分）；context=N 显示上下文；\
             自动跳过 .git/node_modules/target 等生成目录、二进制与 >4MiB 文件；\
             默认最多 {DEFAULT_MAX_MATCHES} 行命中。分工：文件名→list_files/glob；正则→grep；语义→search_codebase。"
        )
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "keywords": { "type": "array", "items": { "type": "string" }, "description": "关键词数组，一行含任意一个即命中" },
                "path": { "type": "string", "description": "搜索根目录（绝对或相对工作区），默认工作区根" },
                "match_all": { "type": "boolean", "description": "true=一行需含所有关键词", "default": false },
                "case_sensitive": { "type": "boolean", "description": "是否区分大小写，默认 false", "default": false },
                "max_matches": { "type": "integer", "description": "最多返回命中行数，默认 300", "default": DEFAULT_MAX_MATCHES },
                "context": { "type": "integer", "description": "命中行前后各显示 N 行上下文，默认 0" }
            },
            "required": ["keywords"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 关键词预处理：去空、按大小写策略归一化
        let case_sensitive = args.case_sensitive.unwrap_or(false);
        let match_all = args.match_all.unwrap_or(false);
        let max_matches = args.max_matches.unwrap_or(DEFAULT_MAX_MATCHES).max(1);
        let context = args.context.unwrap_or(0).min(20);
        let keywords: Vec<String> = args
            .keywords
            .iter()
            .map(|k| k.trim())
            .filter(|k| !k.is_empty())
            .map(|k| {
                if case_sensitive {
                    k.to_string()
                } else {
                    k.to_lowercase()
                }
            })
            .collect();
        if keywords.is_empty() {
            return Err(SearchFileError(
                "keywords 不能为空：请至少提供一个关键词".to_string(),
            ));
        }

        // 搜索根：path 参数优先，否则工作区根（无 cwd 时回退进程 cwd）
        let root = match &args.path {
            Some(p) => resolve_path(p, self.cwd.as_deref()),
            None => self.cwd.clone().unwrap_or_else(|| PathBuf::from(".")),
        };
        let meta = fs::metadata(&root)
            .await
            .map_err(|e| SearchFileError(format!("访问搜索根目录失败 [{}]: {e}", root.display())))?;
        if !meta.is_dir() {
            return Err(SearchFileError(format!(
                "搜索根不是目录 [{}]",
                root.display()
            )));
        }

        // 迭代式目录遍历（栈），避免递归深度问题
        let mut stack: Vec<PathBuf> = vec![root.clone()];
        let mut scanned_files: usize = 0;
        let mut skipped_scan_cap = false;
        // 命中块：每个文件一块 (相对路径, 总行数, Vec<(行号, 行内容)>)
        let mut blocks: Vec<HitBlock> = Vec::new();

        while let Some(dir) = stack.pop() {
            let mut rd = fs::read_dir(&dir)
                .await
                .map_err(|e| SearchFileError(format!("读取目录失败 [{}]: {e}", dir.display())))?;
            while let Some(entry) = rd
                .next_entry()
                .await
                .map_err(|e| SearchFileError(format!("读取目录条目失败 [{}]: {e}", dir.display())))?
            {
                let name = entry.file_name().to_string_lossy().into_owned();
                let file_meta = entry
                    .metadata()
                    .await
                    .map_err(|e| SearchFileError(format!("读取元数据失败 [{}]: {e}", entry.path().display())))?;
                if file_meta.is_dir() {
                    if !SKIP_DIRS.contains(&name.as_str()) {
                        stack.push(entry.path());
                    }
                    continue;
                }
                if !file_meta.is_file() {
                    continue;
                }
                if scanned_files >= MAX_SCAN_FILES {
                    skipped_scan_cap = true;
                    continue;
                }
                if file_meta.len() > MAX_FILE_BYTES {
                    continue;
                }
                scanned_files += 1;

                let bytes = fs::read(entry.path())
                    .await
                    .map_err(|e| SearchFileError(format!("读取文件失败 [{}]: {e}", entry.path().display())))?;
                if is_binary(&bytes) {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes).into_owned();

                // 逐行匹配：收集命中行号，再按 context 扩展为显示列表
                let file_lines: Vec<&str> = text.lines().collect();
                let total_lines = file_lines.len();
                let mut hit_lines: Vec<usize> = Vec::new();
                for (i, line) in file_lines.iter().enumerate() {
                    let haystack = if case_sensitive {
                        (*line).to_string()
                    } else {
                        line.to_lowercase()
                    };
                    let hit = if match_all {
                        keywords.iter().all(|k| haystack.contains(k))
                    } else {
                        keywords.iter().any(|k| haystack.contains(k))
                    };
                    if hit {
                        hit_lines.push(i + 1);
                        if hit_lines.len() >= MAX_MATCHES_PER_FILE {
                            break;
                        }
                    }
                }
                if hit_lines.is_empty() {
                    continue;
                }
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

                // 展示路径：优先相对工作区 cwd（让 LLM 回传 read_file/edit_file 时路径一致，
                // 避免 path=sub 时返回 "inner.rs" 导致 read_file 解析为 cwd/inner.rs 找不到），
                // 其次相对搜索根，最后用绝对路径。Windows 下统一为 / 分隔。
                let path = entry.path();
                let display = self
                    .cwd
                    .as_deref()
                    .and_then(|cwd| path.strip_prefix(cwd).ok())
                    .or_else(|| path.strip_prefix(&root).ok())
                    .map(|r| r.display().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let display = if cfg!(windows) {
                    display.replace('\\', "/")
                } else {
                    display
                };
                  let all_lines: Vec<String> = file_lines.iter().map(|s| s.to_string()).collect();
                  blocks.push((display, total_lines, all_lines, show_lines));
              }
          }

        // 组装输出
        let keyword_display = keywords
            .iter()
            .map(|k| {
                if case_sensitive {
                    k.clone()
                } else {
                    format!("{k}（不区分大小写）")
                }
            })
            .collect::<Vec<_>>()
            .join("、");
        let total_hits: usize = blocks.iter().map(|(_, _, _, m)| m.len()).sum();
        if blocks.is_empty() {
            return Ok(format!(
                "未找到包含关键词 [{}] 的匹配行（共扫描 {scanned_files} 个文件）",
                keyword_display
            ));
        }

        let mut out = String::with_capacity(total_hits.min(max_matches) * 96 + 128);
        let mut shown = 0usize;
        let mut truncated = false;
        for (path_display, total_lines, all_lines, show_lines) in &blocks {
            if shown >= max_matches {
                truncated = true;
                break;
            }
            // path 行附上文件总行数，便于 LLM 判断文件规模后决定是否精读
            out.push_str(&format!("path: {path_display}（共 {total_lines} 行）\n"));
            // 行号宽度按该文件总行数计算，与 read_file 完全对齐
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
        let mut summary = format!(
            "搜索完成：关键词 [{}]，共扫描 {scanned_files} 个文件，{} 个文件命中 {} 处，输出 {shown} 行（context={context}）：\n",
            keyword_display,
            blocks.len(),
            total_hits,
        );
            if truncated {
                summary.push_str(&format!(
                    "[已达最大显示行数 {max_matches}，剩余命中已省略；可缩小关键词范围或增大 max_matches]"
                ));
            }
        if skipped_scan_cap {
            summary.push_str(&format!(
                "[已达到扫描文件数上限 {MAX_SCAN_FILES}，部分文件未扫描]"
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
        std::env::temp_dir().join(format!("effisuite-search-test-{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn search_finds_keyword_in_nested_files() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("src/nested")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {\n    let tokio_runtime = 1;\n}\n").unwrap();
        std::fs::write(dir.join("src/nested/lib.rs"), "pub fn helper() {\n    tokio::spawn(async {});\n}\n").unwrap();
        std::fs::write(dir.join("README.md"), "no match here\n").unwrap();
        // 生成目录应被跳过
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::write(dir.join("target/debug/out.rs"), "tokio inside target\n").unwrap();

        let tool = SearchFileTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchFileArgs {
                keywords: vec!["tokio".to_string()],
                path: None,
                match_all: None,
                case_sensitive: None,
                max_matches: None,
                context: None,
            })
            .await
            .unwrap();

        assert!(out.contains("2 个文件命中"), "out: {out}");
        assert!(out.contains("path: src/main.rs"), "out: {out}");
        assert!(out.contains("let tokio_runtime = 1;"), "out: {out}");
        assert!(out.contains("path: src/nested/lib.rs"), "out: {out}");
        assert!(out.contains("tokio::spawn(async {});"), "out: {out}");
        // target 目录被跳过
        assert!(!out.contains("target"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_match_all_and_case_insensitive() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "Hello World\nhello rust\nHELLO there\n").unwrap();

        let tool = SearchFileTool::with_cwd(dir.clone());

        // 不区分大小写 + 任意关键词：三行都命中
        let out = tool
            .call(SearchFileArgs {
                keywords: vec!["hello".to_string(), "rust".to_string()],
                path: None,
                match_all: None,
                case_sensitive: None,
                max_matches: None,
                context: None,
            })
            .await
            .unwrap();
        assert!(out.contains("命中 3 处"), "out: {out}");
        assert!(out.contains("Hello World") && out.contains("hello rust") && out.contains("HELLO there"), "out: {out}");

        // match_all：必须同时包含 hello 和 rust
        let out = tool
            .call(SearchFileArgs {
                keywords: vec!["hello".to_string(), "rust".to_string()],
                path: None,
                match_all: Some(true),
                case_sensitive: None,
                max_matches: None,
                context: None,
            })
            .await
            .unwrap();
        assert!(out.contains("1 处"), "out: {out}");
        assert!(out.contains("hello rust"));

        // 区分大小写：只有 HELLO there 命中（Hello World 大小写不匹配）
        let out = tool
            .call(SearchFileArgs {
                keywords: vec!["HELLO".to_string()],
                path: None,
                match_all: None,
                case_sensitive: Some(true),
                max_matches: None,
                context: None,
            })
            .await
            .unwrap();
        assert!(out.contains("命中 1 处"), "out: {out}");
        assert!(out.contains("HELLO there") && !out.contains("Hello World"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_no_match_and_empty_keywords() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "nothing here\n").unwrap();
        let tool = SearchFileTool::with_cwd(dir.clone());

        let out = tool
            .call(SearchFileArgs {
                keywords: vec!["不存在的词".to_string()],
                path: None,
                match_all: None,
                case_sensitive: None,
                max_matches: None,
                context: None,
            })
            .await
            .unwrap();
        assert!(out.contains("未找到"));

        let r = tool
            .call(SearchFileArgs {
                keywords: vec![],
                path: None,
                match_all: None,
                case_sensitive: None,
                max_matches: None,
                context: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("keywords"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_skips_binary_files() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // 含 NUL 字节的"二进制"文件
        std::fs::write(dir.join("bin.dat"), b"\x00\x01\x02tokio\x00\x03").unwrap();
        std::fs::write(dir.join("a.txt"), "tokio text\n").unwrap();

        let tool = SearchFileTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchFileArgs {
                keywords: vec!["tokio".to_string()],
                path: None,
                match_all: None,
                case_sensitive: None,
                max_matches: None,
                context: None,
            })
            .await
            .unwrap();
        assert!(out.contains("path: a.txt"));
        assert!(!out.contains("bin.dat"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_respects_subdir_path() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("root.txt"), "tokio root\n").unwrap();
        std::fs::write(dir.join("sub/inner.txt"), "tokio inner\n").unwrap();

        let tool = SearchFileTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchFileArgs {
                keywords: vec!["tokio".to_string()],
                path: Some("sub".to_string()),
                match_all: None,
                case_sensitive: None,
                max_matches: None,
                context: None,
            })
            .await
            .unwrap();
        // 路径基准相对工作区 cwd：path=sub 时返回 "sub/inner.txt" 而非 "inner.txt"，
        // 保证 LLM 回传 read_file/edit_file 时路径可解析
        assert!(out.contains("path: sub/inner.txt"));
        assert!(!out.contains("root.txt"));

        std::fs::remove_dir_all(&dir).ok();
    }
    #[tokio::test]
    async fn search_with_context_shows_surrounding_lines() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "l1\nl2\ntokio hit\nl4\nl5\n").unwrap();
        let tool = SearchFileTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchFileArgs {
                keywords: vec!["tokio".to_string()],
                path: None,
                match_all: None,
                case_sensitive: None,
                max_matches: None,
                context: Some(1),
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

}
