//! search_codebase 工具：基于关键词加权排序的代码搜索
//!
//! 用自然语言描述搜索代码库中的相关代码。不是简单的关键词精确匹配，而是
//! 基于简化版 TF-IDF 的"语义相似度"排序：
//!
//! 1. 从自然语言查询中提取关键词（分词、去停用词、转小写、去重）
//! 2. 遍历工作区代码文件（栈式遍历，跳过 .git / target / node_modules 等）
//! 3. 对每个文件做简化版 TF-IDF 相似度匹配：
//!    - 关键词命中次数（位置加权：标识符定义 > 注释 > 字符串 > 普通行）
//!    - 文件长度归一化（除以 sqrt(行数)，避免长文件刷分）
//!    - 关键词覆盖率奖励（命中比例越高，得分越高）
//! 4. 按得分排序，返回 Top-N 代码块
//! 5. 自动扩展到包含匹配行的完整函数/结构体代码块
//!
//! # 与 search_file 的区别
//!
//! - `search_file`：关键词精确匹配，返回所有命中行（按文件分组）
//! - `search_codebase`：自然语言查询，按语义相似度排序，返回 Top-N 代码块
//!
//! # 与 search_memory 的区别
//!
//! - `search_memory`：跨会话历史对话记忆（基于 [`MemoryIndex`]）
//! - `search_codebase`：当前工作区代码文件（独立轻量索引，不持久化）
//!
//! # 性能要点（对齐 user_rules）
//!
//! - 异步遍历（`tokio::fs`），单次扫描上限 20_000 文件
//! - 跳过 >4 MiB 大文件与二进制（NUL 字节探测）
//! - 仅扫描 `CODE_EXTS` 中的代码文件扩展名
//! - 全文 `to_lowercase` 一次，避免热路径每行重复分配
//! - 结构体字段按大小降序，最小化 padding
//! - 迭代器适配器优先，无 `for i in 0..len` 索引循环

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::fs;

use super::resolve_path;
use super::text_utils::{format_numbered_line, line_number_width};

/// 默认最大返回结果数
const MAX_RESULTS: usize = 20;
/// 扫描文件数硬上限（防止超大仓库长时间阻塞）
const MAX_SCAN_FILES: usize = 20_000;
/// 跳过大于该字节数的文件（4 MiB）
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// 单个结果代码块最大行数（防止返回整个大文件）
const MAX_BLOCK_LINES: usize = 80;
/// 上下文扩展时向上查找定义行的最大行数
const MAX_BACKWARD_SEARCH: usize = 50;
/// 跳过的生成/依赖目录
const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", "__pycache__", ".venv", "venv",
    ".pytest_cache", ".mypy_cache", ".next", ".turbo", ".nuxt", ".svelte-kit",
    ".output", "coverage",
];
/// 仅搜索的代码文件扩展名（不含点）
const CODE_EXTS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp", "h", "hpp",
    "rb", "php", "swift", "kt", "vue", "svelte", "md", "toml", "yaml", "yml", "json",
];
/// 英文停用词 + 中文停用词
const STOP_WORDS: &[&str] = &[
    // 英文
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being",
    "have", "has", "had", "do", "does", "did", "will", "would", "could",
    "should", "may", "might", "must", "shall", "can",
    "to", "of", "in", "on", "at", "by", "for", "with", "about", "against",
    "between", "into", "through", "during", "before", "after", "above",
    "below", "from", "up", "down", "out", "off", "over", "under", "again",
    "further", "then", "once",
    "and", "or", "but", "if", "else", "when", "where", "why", "how",
    "all", "each", "every", "both", "few", "more", "most", "other", "some",
    "such", "no", "nor", "not", "only", "own", "same", "so", "than", "too",
    "very",
    "i", "me", "my", "we", "our", "you", "your", "he", "him", "his", "she",
    "her", "it", "its", "they", "them", "their",
    "what", "which", "who", "whom",
    // 中文
    "的", "了", "在", "是", "我", "你", "他", "她", "它", "们", "这", "那",
    "和", "与", "或", "但", "如果", "那么", "当", "为", "把", "被", "让",
    "可以", "能", "会", "要", "想", "做", "去", "来", "到", "上", "下",
    "里", "外", "中", "前", "后",
];

/// 工具参数
///
/// 字段按大小降序：`String`（24B）= `Option<Vec<String>>`（24B）> `Option<usize>`（16B）。
#[derive(Deserialize)]
pub struct SearchCodebaseArgs {
    /// 自然语言查询（描述想找什么代码）
    pub query: String,
    /// 搜索根目录数组（绝对或相对工作区），默认工作区根目录
    #[serde(default)]
    pub target_directories: Option<Vec<String>>,
    /// 最大返回结果数，默认 20
    #[serde(default)]
    pub max_results: Option<usize>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("search_codebase error: {0}")]
pub struct SearchCodebaseError(String);

/// 语义代码搜索工具
///
/// `cwd` 为可选工作区：设置后作为默认搜索根，相对路径以此为基准。
pub struct SearchCodebaseTool {
    cwd: Option<PathBuf>,
}

impl SearchCodebaseTool {
    pub fn new() -> Self {
        Self { cwd: None }
    }

    /// 指定工作区目录，默认在此目录下搜索
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self { cwd: Some(cwd) }
    }
}

impl Default for SearchCodebaseTool {
    fn default() -> Self {
        Self::new()
    }
}

/// 单个搜索结果（一个代码块）
///
/// 字段按大小降序：`String`/`Vec<String>`（24B）> `usize`（8B）> `f64`（8B）。
struct CodeHit {
    /// 相对搜索根的展示路径（已归一化为 / 分隔）
    display_path: String,
    /// 命中的关键词（小写，按查询顺序）
    matched_keywords: Vec<String>,
    /// 代码块原文（按行）
    block_lines: Vec<String>,
    /// 文件总行数
    total_lines: usize,
    /// 代码块起始行号（1-based，含）
    block_start: usize,
    /// 得分（越高越相关）
    score: f64,
}

impl Tool for SearchCodebaseTool {
    const NAME: &'static str = "search_codebase";

    type Error = SearchCodebaseError;
    type Args = SearchCodebaseArgs;
    type Output = String;

    fn description(&self) -> String {
        let cwd_hint = self
            .cwd
            .as_ref()
            .map(|p| format!("当前工作区：{}（默认搜索根，相对路径以此为准）", p.display()))
            .unwrap_or_else(|| "未设置工作区，默认在进程工作目录下搜索".to_string());
        format!(
            "用自然语言描述搜索代码库中的相关代码（基于关键词加权排序的代码搜索，非关键词精确匹配）。\
             输入一句自然语言描述（如「how does authentication work」「处理用户登录的逻辑」），\
             自动提取关键词后在工作区代码文件中检索最相关的代码块。\n\n\
             **与其他查找工具的边界**：\n\
             - `list_files`：列目录树，不按模式过滤、不读内容\n\
             - `glob`：按文件名模式匹配，不读内容\n\
             - `search_file`：字面关键词搜文件内容（非正则，返回所有命中行）\n\
             - `grep`：正则表达式搜文件内容\n\
             - `search_codebase`（本工具）：基于关键词加权排序（简化版 TF-IDF）的代码搜索，返回 Top-N 最相关代码块，适合「我想找做 X 的代码但不知道具体函数名」的场景\n\n\
             **返回格式**：每个结果包含文件路径、命中关键词、得分、代码块（带行号），\
             自动扩展到包含匹配行的完整函数/结构体。\n\n\
             自动跳过生成目录（.git / node_modules / target / dist 等）与非代码文件，\
             仅扫描代码文件（rs / py / js / ts / go / java / c / cpp / ...）。\
             默认最多返回 {MAX_RESULTS} 个结果。\n{cwd_hint}"
        )
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "自然语言查询（描述想找什么代码），如「how does authentication work」"
                },
                "target_directories": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "搜索根目录数组（绝对或相对工作区），默认搜索整个工作区"
                },
                "max_results": {
                    "type": "integer",
                    "description": "最大返回结果数，默认 20",
                    "default": MAX_RESULTS
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 1. 提取关键词
        let query = args.query.trim();
        if query.is_empty() {
            return Err(SearchCodebaseError("query 不能为空".to_string()));
        }
        let keywords = extract_keywords(query);
        if keywords.is_empty() {
            return Err(SearchCodebaseError(format!(
                "无法从查询「{query}」中提取有效关键词（请提供更具体的描述）"
            )));
        }

        let max_results = args.max_results.unwrap_or(MAX_RESULTS).clamp(1, 50);

        // 2. 解析搜索根目录列表
        let roots = resolve_roots(&args.target_directories, self.cwd.as_deref())?;
        if roots.is_empty() {
            return Err(SearchCodebaseError("没有有效的搜索根目录".to_string()));
        }

        // 3. 遍历代码文件并打分
        let mut hits: Vec<CodeHit> = Vec::new();
        let mut scanned_files: usize = 0;
        let mut skipped_scan_cap = false;
        // 多根目录去重：同一文件被多个根覆盖时只算一次（按绝对路径去重）
        let mut visited: HashSet<PathBuf> = HashSet::new();

        for root in &roots {
            let mut stack: Vec<PathBuf> = vec![root.clone()];
            while let Some(dir) = stack.pop() {
                let mut rd = match fs::read_dir(&dir).await {
                    Ok(r) => r,
                    Err(_) => continue, // 跳过无权限或已删除的目录
                };
                while let Ok(Some(entry)) = rd.next_entry().await {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let file_meta = match entry.metadata().await {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    if file_meta.is_dir() {
                        if !SKIP_DIRS.contains(&name.as_str()) {
                            stack.push(entry.path());
                        }
                        continue;
                    }
                    if !file_meta.is_file() {
                        continue;
                    }
                    // 仅扫描代码文件
                    if !is_code_file(&name) {
                        continue;
                    }
                    if scanned_files >= MAX_SCAN_FILES {
                        skipped_scan_cap = true;
                        continue;
                    }
                    if file_meta.len() > MAX_FILE_BYTES {
                        continue;
                    }

                    let path = entry.path();
                    // 多根目录去重（同一文件被多个根覆盖时只算一次）
                    if !visited.insert(path.clone()) {
                        continue;
                    }
                    scanned_files += 1;

                    // 读取并打分
                    let bytes = match fs::read(&path).await {
                        Ok(b) => b,
                        Err(_) => continue,
                    };
                    if is_binary(&bytes) {
                        continue;
                    }
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    let lines: Vec<&str> = text.lines().collect();
                    // 全文小写一次，避免热路径每行重复分配
                    let lower_text = text.to_lowercase();
                    let lower_lines: Vec<&str> = lower_text.lines().collect();
                    let (score, matched_kw, hit_lines) =
                        score_file(&lines, &lower_lines, &keywords);
                    if score <= 0.0 || hit_lines.is_empty() {
                        continue;
                    }

                    // 取得分最高的命中行（权重 × 命中次数最大），扩展到完整代码块
                    let best_hit = hit_lines
                        .iter()
                        .copied()
                        .fold((hit_lines[0], 0.0_f64), |(best, best_s), line_no| {
                            let idx = line_no.saturating_sub(1).min(lines.len().saturating_sub(1));
                            let count: f64 = keywords
                                .iter()
                                .map(|kw| lower_lines[idx].matches(kw.as_str()).count() as f64)
                                .sum();
                            let s = count * position_weight(lines[idx], lower_lines[idx]);
                            if s > best_s {
                                (line_no, s)
                            } else {
                                (best, best_s)
                            }
                        })
                        .0;
                    let (block_start, block_end) = find_code_block(&lines, best_hit);
                    let block_end_clamped = block_end.min(lines.len());
                    let block_lines: Vec<String> = lines
                        [block_start.saturating_sub(1)..block_end_clamped]
                        .iter()
                        .map(|s| s.to_string())
                        .collect();

                    // 展示路径：优先相对工作区 cwd（让 LLM 回传 read_file/edit_file 时路径一致），
                    // 其次相对搜索根，最后用绝对路径。Windows 下统一为 / 分隔。
                    let display = self
                        .cwd
                        .as_deref()
                        .and_then(|cwd| path.strip_prefix(cwd).ok())
                        .or_else(|| path.strip_prefix(root).ok())
                        .map(|r| r.display().to_string())
                        .unwrap_or_else(|| path.display().to_string());
                    let display = if cfg!(windows) {
                        display.replace('\\', "/")
                    } else {
                        display
                    };

                    hits.push(CodeHit {
                        display_path: display,
                        matched_keywords: matched_kw,
                        block_lines,
                        total_lines: lines.len(),
                        block_start,
                        score,
                    });
                }
            }
        }

        // 4. 排序 + 截断（按得分降序）
        hits.sort_by(|a, b| {
            b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(max_results);

        // 5. 格式化输出
        let keyword_display = keywords.join(", ");
        if hits.is_empty() {
            return Ok(format!(
                "未找到与「{query}」相关的代码（共扫描 {scanned_files} 个文件，关键词: {keyword_display}）"
            ));
        }

        // 归一化得分到 0-1（最高分映射为 1.0，便于用户判断相对相关性）
        let max_score = hits
            .iter()
            .map(|h| h.score)
            .fold(0.0_f64, f64::max)
            .max(1.0);

        let mut out = String::with_capacity(hits.len() * 512);
        out.push_str(&format!(
            "找到 {} 个相关代码块（查询：「{query}」，关键词: {keyword_display}）：\n\n",
            hits.len()
        ));
        for (i, hit) in hits.iter().enumerate() {
            let normalized = hit.score / max_score;
            out.push_str(&format!(
                "{}. {}（共 {} 行，得分: {:.2}）\n",
                i + 1,
                hit.display_path,
                hit.total_lines,
                normalized
            ));
            out.push_str(&format!(
                "   匹配关键词: {}\n\n",
                hit.matched_keywords.join(", ")
            ));
            // 行号宽度按该文件总行数计算，与 read_file / search_file 完全对齐
            let width = line_number_width(hit.total_lines);
            for (offset, line) in hit.block_lines.iter().enumerate() {
                let line_no = hit.block_start + offset;
                out.push_str(&format_numbered_line(line_no, width, line));
                out.push('\n');
            }
            out.push('\n');
        }

        if skipped_scan_cap {
            out.push_str(&format!(
                "[已达到扫描文件数上限 {MAX_SCAN_FILES}，部分文件未扫描]"
            ));
        }

        Ok(out)
    }
}

/// 解析搜索根目录列表
///
/// - `None` 或空列表 → 使用 cwd（或回退到 "."）
/// - `Some(dirs)` → 对每个路径用 `resolve_path` 解析为绝对路径
fn resolve_roots(
    target_directories: &Option<Vec<String>>,
    cwd: Option<&Path>,
) -> Result<Vec<PathBuf>, SearchCodebaseError> {
    let fallback = || -> PathBuf {
        cwd.map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
    };

    match target_directories {
        None => {
            let root = fallback();
            validate_root(&root)?;
            Ok(vec![root])
        }
        Some(dirs) if dirs.is_empty() => {
            let root = fallback();
            validate_root(&root)?;
            Ok(vec![root])
        }
        Some(dirs) => {
            let mut roots: Vec<PathBuf> = Vec::with_capacity(dirs.len());
            for d in dirs {
                let trimmed = d.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let p = resolve_path(trimmed, cwd);
                validate_root(&p)?;
                roots.push(p);
            }
            Ok(roots)
        }
    }
}

/// 校验搜索根目录是否存在且为目录
fn validate_root(p: &Path) -> Result<(), SearchCodebaseError> {
    let meta = std::fs::metadata(p).map_err(|e| {
        SearchCodebaseError(format!("访问搜索根目录失败 [{}]: {e}", p.display()))
    })?;
    if !meta.is_dir() {
        return Err(SearchCodebaseError(format!(
            "搜索根不是目录 [{}]",
            p.display()
        )));
    }
    Ok(())
}

/// 从自然语言查询中提取关键词
///
/// 步骤：
/// 1. 按非字母数字字符分词（保留中文连续字符与下划线）
/// 2. 转小写
/// 3. 去停用词
/// 4. 去重（保留首次出现顺序）
/// 5. 过滤太短的词（<2 字符，但保留中文单字）
/// 6. 纯 CJK 多字 token 额外拆分单字（中文无空格分词，单字允许更宽匹配）
fn extract_keywords(query: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::with_capacity(8);
    let mut seen: HashSet<String> = HashSet::new();

    for token in tokenize(query) {
        let lower = token.to_lowercase();
        if !is_valid_keyword(&lower) {
            continue;
        }
        if STOP_WORDS.contains(&lower.as_str()) {
            continue;
        }
        if seen.insert(lower.clone()) {
            result.push(lower.clone());
        }
        // 纯 CJK 多字 token：额外发射单字（中文无空格分词，
        // "处理用户登录" 无法拆词，单字可作为兜底匹配）
        if lower.chars().count() > 1 && is_pure_cjk(&lower) {
            for ch in lower.chars() {
                let s = ch.to_string();
                if !is_valid_keyword(&s) {
                    continue;
                }
                if STOP_WORDS.contains(&s.as_str()) {
                    continue;
                }
                if seen.insert(s.clone()) {
                    result.push(s);
                }
            }
        }
    }
    result
}

/// 判断字符串是否全部由 CJK 字符组成
#[inline]
fn is_pure_cjk(s: &str) -> bool {
    s.chars().all(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

/// 分词：按非字母数字字符分割，连续 CJK 字符作为一个 token
///
/// - 连续的 ASCII 字母数字/下划线/CJK 字符作为一个 token
///   （如 `authentication`、`verify_token`、`处理`、`用户`）
/// - 中文无空格分词，建议用空格分隔词语以获得更精准的匹配
/// - 其他字符（空格、标点、符号）作为分隔符
#[inline]
fn tokenize(s: &str) -> Vec<&str> {
    s.split(|c: char| !(c.is_alphanumeric() || c == '_' || ('\u{4e00}'..='\u{9fff}').contains(&c)))
        .filter(|t| !t.is_empty())
        .collect()
}

/// 判断 token 是否为有效关键词
///
/// 规则：
/// - 长度 >= 2，或
/// - 包含中文字符（中文单字也算）
#[inline]
fn is_valid_keyword(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    if s.len() >= 2 {
        return true;
    }
    // 单字符：仅当中文时保留
    s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

/// 判断是否为代码文件扩展名
#[inline]
fn is_code_file(name: &str) -> bool {
    let ext = match name.rsplit('.').next() {
        Some(e) => e,
        None => return false,
    };
    CODE_EXTS.contains(&ext)
}

/// 判断是否为二进制文件：探测前 8KB 是否含 NUL 字节
#[inline]
fn is_binary(bytes: &[u8]) -> bool {
    let probe = &bytes[..bytes.len().min(8192)];
    probe.contains(&0)
}

/// 计算单个文件的匹配得分
///
/// 简化版 TF-IDF：
/// - 对每个关键词，统计在每行中的命中次数
/// - 位置加权：定义行 3.0 / 注释 2.0 / 字符串 1.5 / 普通 1.0
/// - 文件长度归一化：除以 sqrt(行数)，避免长文件刷分
/// - 关键词覆盖率奖励：乘以 (1 + 命中比例)
///
/// 返回 `(得分, 命中的关键词列表, 命中的行号列表)`
///
/// `lines` 与 `lower_lines` 必须行号一一对应（同一段文本的原文与小写形式）。
fn score_file(
    lines: &[&str],
    lower_lines: &[&str],
    keywords: &[String],
) -> (f64, Vec<String>, Vec<usize>) {
    if lines.is_empty() || keywords.is_empty() {
        return (0.0, Vec::new(), Vec::new());
    }

    let mut total_score: f64 = 0.0;
    let mut matched: Vec<bool> = vec![false; keywords.len()];
    let mut hit_lines: Vec<usize> = Vec::new();

    for (i, (original, lower_line)) in lines.iter().zip(lower_lines.iter()).enumerate() {
        let mut line_score: f64 = 0.0;
        for (ki, kw) in keywords.iter().enumerate() {
            // 子串匹配（关键词已在 extract_keywords 中转小写）
            let count = lower_line.matches(kw.as_str()).count();
            if count > 0 {
                matched[ki] = true;
                line_score += count as f64;
            }
        }
        if line_score > 0.0 {
            let weight = position_weight(original, lower_line);
            total_score += line_score * weight;
            hit_lines.push(i + 1);
        }
    }

    if total_score <= 0.0 {
        return (0.0, Vec::new(), Vec::new());
    }

    // 文件长度归一化（除以 sqrt(行数)）
    let normalized = total_score / (lines.len() as f64).sqrt().max(1.0);

    // 关键词覆盖率奖励（命中比例越高，得分越高）
    let hit_count = matched.iter().filter(|&&m| m).count();
    let coverage = hit_count as f64 / keywords.len() as f64;
    let final_score = normalized * (1.0 + coverage);

    let matched_vec: Vec<String> = matched
        .iter()
        .zip(keywords.iter())
        .filter(|(m, _)| **m)
        .map(|(_, kw)| kw.clone())
        .collect();

    (final_score, matched_vec, hit_lines)
}

/// 根据行内容计算位置权重
///
/// - 函数/类定义行：3.0（最相关）
/// - 注释行：2.0
/// - 字符串行：1.5
/// - 普通行：1.0
#[inline]
fn position_weight(line: &str, lower_line: &str) -> f64 {
    let trimmed = line.trim_start();
    // 注释（// # /* * -- 等）
    if trimmed.starts_with("//")
        || trimmed.starts_with("/*")
        || trimmed.starts_with("*")
        || trimmed.starts_with("#")
        || trimmed.starts_with("--")
    {
        return 2.0;
    }
    // 函数/类/结构体定义
    if is_definition_line(line) {
        return 3.0;
    }
    // 字符串（粗略判断：含引号）
    if lower_line.contains('"') || lower_line.contains('\'') {
        return 1.5;
    }
    1.0
}

/// 判断是否为函数/类/结构体等定义行
#[inline]
fn is_definition_line(line: &str) -> bool {
    let t = line.trim_start();
    if t.is_empty() {
        return false;
    }
    // 跳过注释行（避免误判）
    if t.starts_with("//")
        || t.starts_with("/*")
        || t.starts_with("*")
        || t.starts_with("#")
        || t.starts_with("--")
    {
        return false;
    }
    const PREFIXES: &[&str] = &[
        // Rust
        "fn ", "async fn ", "pub fn ", "pub async fn ", "pub(crate) fn ",
        "struct ", "enum ", "trait ", "impl ", "mod ",
        // Python
        "def ", "class ", "async def ",
        // JS/TS
        "function ", "function* ",
        // Go
        "func ",
        // TS/JS
        "interface ", "type ",
        // Java/Kotlin
        "public class ", "private class ", "protected class ",
        "public final class ", "fun ",
    ];
    PREFIXES.iter().any(|p| t.starts_with(p))
}

/// 找到包含 `hit_line` 的代码块（函数/结构体）的行范围
///
/// 策略：
/// 1. 向上查找最近的定义行（最多 `MAX_BACKWARD_SEARCH` 行）
/// 2. 找到定义行后，向下基于大括号匹配确定块结束
/// 3. 找不到定义行时，以 `hit_line` 为中心向上向下各扩展 5 行
/// 4. 单个代码块最多 `MAX_BLOCK_LINES` 行
/// 5. 无大括号语言（Python 等）遇到下一个定义行即停止
///
/// 返回 `(start, end)` 1-based 行号，含两端
fn find_code_block(lines: &[&str], hit_line: usize) -> (usize, usize) {
    let total = lines.len();
    if total == 0 {
        return (1, 1);
    }
    let hit_idx = hit_line.saturating_sub(1).min(total - 1);

    // 向上查找最近的定义行
    let search_start = hit_idx.saturating_sub(MAX_BACKWARD_SEARCH);
    let mut start_idx = hit_idx;
    let mut found_def = false;
    for i in (search_start..=hit_idx).rev() {
        if is_definition_line(lines[i]) {
            start_idx = i;
            found_def = true;
            break;
        }
    }
    if !found_def {
        // 未找到定义行：以 hit_line 为中心扩展 5 行
        start_idx = hit_idx.saturating_sub(5);
    }

    // 向下基于大括号匹配
    let mut brace_count: i32 = 0;
    let mut found_brace = false;
    let mut end_idx = hit_idx;
    let max_end = (start_idx + MAX_BLOCK_LINES - 1).min(total - 1);

    for i in start_idx..=max_end {
        let line = lines[i];
        let opens = line.matches('{').count() as i32;
        let closes = line.matches('}').count() as i32;
        if opens > 0 {
            found_brace = true;
        }
        if found_brace {
            brace_count += opens - closes;
        }
        end_idx = i;
        // 大括号匹配完成且已超过 hit_line
        if found_brace && brace_count <= 0 && i >= hit_idx {
            break;
        }
        // 无大括号语言（Python 等）：遇到下一个定义行停止
        if !found_brace && i > hit_idx && is_definition_line(lines[i]) {
            end_idx = i - 1;
            break;
        }
    }

    (start_idx + 1, end_idx + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        std::env::temp_dir()
            .join(format!("effisuite-search-codebase-{}", uuid::Uuid::new_v4()))
    }

    /// 测试辅助：构造原文 + 小写行后调用 score_file
    fn score_for_test(
        lines: &[&str],
        keywords: &[String],
    ) -> (f64, Vec<String>, Vec<usize>) {
        let lower: Vec<String> = lines.iter().map(|l| l.to_lowercase()).collect();
        let lower_refs: Vec<&str> = lower.iter().map(|s| s.as_str()).collect();
        score_file(lines, &lower_refs, keywords)
    }

    // ============ extract_keywords ============

    #[test]
    fn extract_keywords_basic() {
        let kws = extract_keywords("how does authentication work");
        assert!(kws.iter().any(|k| k == "authentication"));
        assert!(kws.iter().any(|k| k == "work"));
        assert!(!kws.iter().any(|k| k == "how")); // 停用词
        assert!(!kws.iter().any(|k| k == "does")); // 停用词
    }

    #[test]
    fn extract_keywords_chinese() {
        // 中文按字符分词，用空格分隔词语
        let kws = extract_keywords("处理 用户 登录 逻辑");
        assert!(kws.iter().any(|k| k == "处理"));
        assert!(kws.iter().any(|k| k == "用户"));
        assert!(kws.iter().any(|k| k == "登录"));
        assert!(kws.iter().any(|k| k == "逻辑"));
    }

    #[test]
    fn extract_keywords_chinese_single_chars() {
        // 无空格的中文按单字分词
        let kws = extract_keywords("处理用户登录的逻辑");
        // "的" 是停用词，应被过滤
        assert!(!kws.iter().any(|k| k == "的"));
        // 其他单字应保留
        assert!(kws.iter().any(|k| k == "处"));
        assert!(kws.iter().any(|k| k == "理"));
        assert!(kws.iter().any(|k| k == "登"));
        assert!(kws.iter().any(|k| k == "录"));
    }

    #[test]
    fn extract_keywords_dedup() {
        let kws = extract_keywords("auth auth auth");
        assert_eq!(kws, vec!["auth"]);
    }

    #[test]
    fn extract_keywords_strips_short_english() {
        // 单字符英文应被过滤
        let kws = extract_keywords("a b auth");
        assert_eq!(kws, vec!["auth"]);
    }

    #[test]
    fn extract_keywords_empty_query() {
        assert!(extract_keywords("").is_empty());
        assert!(extract_keywords("   ").is_empty());
        assert!(extract_keywords("the a an is").is_empty());
    }

    // ============ is_definition_line ============

    #[test]
    fn is_definition_line_detects_rust() {
        assert!(is_definition_line("pub fn verify_token(token: &str) -> Result<Claims> {"));
        assert!(is_definition_line("struct User {"));
        assert!(is_definition_line("enum Role {"));
        assert!(is_definition_line("impl User {"));
        assert!(is_definition_line("async fn fetch() {"));
        assert!(is_definition_line("pub(crate) fn helper() {"));
    }

    #[test]
    fn is_definition_line_detects_python() {
        assert!(is_definition_line("def verify_token(token):"));
        assert!(is_definition_line("class User:"));
        assert!(is_definition_line("async def fetch():"));
    }

    #[test]
    fn is_definition_line_detects_other_langs() {
        assert!(is_definition_line("function foo() {"));
        assert!(is_definition_line("func bar() {"));
        assert!(is_definition_line("interface Baz {"));
        assert!(is_definition_line("type Quux = {"));
    }

    #[test]
    fn is_definition_line_ignores_comments() {
        assert!(!is_definition_line("// fn commented_out() {"));
        assert!(!is_definition_line("# def commented():"));
        assert!(!is_definition_line("/* class Old { */"));
        assert!(!is_definition_line("-- fn sql_comment"));
    }

    #[test]
    fn is_definition_line_ignores_empty() {
        assert!(!is_definition_line(""));
        assert!(!is_definition_line("   "));
    }

    // ============ is_code_file ============

    #[test]
    fn is_code_file_recognizes_extensions() {
        assert!(is_code_file("main.rs"));
        assert!(is_code_file("app.py"));
        assert!(is_code_file("index.ts"));
        assert!(is_code_file("Component.tsx"));
        assert!(is_code_file("main.go"));
        assert!(is_code_file("Cargo.toml"));
        assert!(is_code_file("README.md"));
    }

    #[test]
    fn is_code_file_rejects_non_code() {
        assert!(!is_code_file("auth.txt"));
        assert!(!is_code_file("Cargo.lock"));
        assert!(!is_code_file("image.png"));
        assert!(!is_code_file("archive.tar.gz")); // ext = "gz"，不在列表
        assert!(!is_code_file("noext"));
    }

    // ============ find_code_block ============

    #[test]
    fn find_code_block_expands_to_function() {
        let lines: Vec<&str> = vec![
            "use std::io;",                                            // 1
            "",                                                        // 2
            "pub fn verify_token(token: &str) -> Result<Claims> {",   // 3
            "    let key = get_secret_key();",                        // 4
            "    let claims = decode::<Claims>(token, &key)?;",       // 5
            "    Ok(claims)",                                          // 6
            "}",                                                       // 7
            "",                                                        // 8
            "pub fn other_function() {",                              // 9
            "    todo!()",                                             // 10
            "}",                                                       // 11
        ];
        // 命中第 4 行 → 应扩展到第 3-7 行（完整函数）
        let (start, end) = find_code_block(&lines, 4);
        assert_eq!(start, 3);
        assert_eq!(end, 7);
    }

    #[test]
    fn find_code_block_falls_back_when_no_def() {
        let lines: Vec<&str> = vec![
            "let a = 1;",  // 1
            "let b = 2;",  // 2
            "let c = 3;",  // 3
            "let d = 4;",  // 4
            "let e = 5;",  // 5
            "let f = 6;",  // 6
            "let g = 7;",  // 7
            "let h = 8;",  // 8
            "let i = 9;",  // 9
            "let j = 10;", // 10
            "let k = 11;", // 11
        ];
        // 命中第 6 行，无定义行 → 以 6 为中心扩展 ±5 = 1-11
        let (start, end) = find_code_block(&lines, 6);
        assert_eq!(start, 1);
        assert_eq!(end, 11);
    }

    #[test]
    fn find_code_block_python_stops_at_next_def() {
        let lines: Vec<&str> = vec![
            "def verify_token(token):",      // 1
            "    key = get_secret()",        // 2
            "    return token == key",       // 3
            "",                              // 4
            "def other_function():",         // 5
            "    pass",                      // 6
        ];
        // 命中第 2 行 → 从 def 开始到下一个 def 之前（第 1-4 行）
        let (start, end) = find_code_block(&lines, 2);
        assert_eq!(start, 1);
        assert_eq!(end, 4);
    }

    #[test]
    fn find_code_block_clamps_to_max_block_lines() {
        // 一个超长函数（无大括号匹配的边界情况）
        let owned: Vec<String> = (0..200).map(|i| format!("    let v{i} = {i};")).collect();
        let lines: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
        // 命中第 100 行，无定义行，无大括号 → 扩展到 start+MAX_BLOCK_LINES
        let (start, end) = find_code_block(&lines, 100);
        assert!(end - start + 1 <= MAX_BLOCK_LINES, "block too large: {}", end - start + 1);
    }

    // ============ score_file ============

    #[test]
    fn score_file_ranks_definition_higher() {
        let def_lines: Vec<&str> = vec![
            "pub fn auth_verify(token: &str) -> bool {",
            "    let key = \"secret\";",
            "    token == key",
            "}",
        ];
        let non_def_lines: Vec<&str> = vec![
            "    let s = \"auth_verify\";",
            "    let t = \"hello\";",
            "    let u = \"world\";",
            "    let v = \"foo\";",
        ];
        let keywords = vec!["auth".to_string(), "verify".to_string()];
        let (def_score, _, _) = score_for_test(&def_lines, &keywords);
        let (non_def_score, _, _) = score_for_test(&non_def_lines, &keywords);
        assert!(
            def_score > non_def_score,
            "def: {def_score}, non_def: {non_def_score}"
        );
    }

    #[test]
    fn score_file_rewards_keyword_coverage() {
        let lines: Vec<&str> = vec![
            "pub fn auth_verify(token: &str) -> bool {",
            "    let key = get_secret();",
            "    token == key",
            "}",
        ];
        let single_kw = vec!["auth".to_string()];
        let multi_kw = vec!["auth".to_string(), "verify".to_string()];
        let (single_score, _, _) = score_for_test(&lines, &single_kw);
        let (multi_score, _, _) = score_for_test(&lines, &multi_kw);
        // 命中更多关键词时，覆盖率奖励让得分更高
        assert!(
            multi_score > single_score,
            "single: {single_score}, multi: {multi_score}"
        );
    }

    #[test]
    fn score_file_empty_inputs() {
        let empty: Vec<&str> = vec![];
        let kws = vec!["auth".to_string()];
        let (s, m, h) = score_for_test(&empty, &kws);
        assert_eq!(s, 0.0);
        assert!(m.is_empty());
        assert!(h.is_empty());

        let lines = vec!["pub fn auth() {}"];
        let empty_kws: Vec<String> = vec![];
        let (s, _, _) = score_for_test(&lines, &empty_kws);
        assert_eq!(s, 0.0);
    }

    #[test]
    fn score_file_no_match_returns_zero() {
        let lines: Vec<&str> = vec!["fn main() {", "    println!(\"hi\");", "}"];
        let kws = vec!["auth".to_string(), "token".to_string()];
        let (s, m, h) = score_for_test(&lines, &kws);
        assert_eq!(s, 0.0);
        assert!(m.is_empty());
        assert!(h.is_empty());
    }

    // ============ 端到端集成测试 ============

    #[tokio::test]
    async fn search_finds_relevant_code() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("src/auth.rs"),
            "pub fn verify_token(token: &str) -> Result<Claims> {\n    let key = get_secret_key();\n    let claims = decode::<Claims>(token, &key)?;\n    Ok(claims)\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src/main.rs"),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();

        let tool = SearchCodebaseTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchCodebaseArgs {
                query: "verify token".to_string(),
                target_directories: None,
                max_results: None,
            })
            .await
            .unwrap();

        assert!(out.contains("找到 1 个相关代码块"), "out: {out}");
        assert!(out.contains("src/auth.rs"), "out: {out}");
        assert!(out.contains("verify_token"), "out: {out}");
        // 不相关的 main.rs 不应出现
        assert!(!out.contains("src/main.rs"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_returns_block_with_line_numbers() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("auth.rs"),
            "use std::io;\n\npub fn verify_token(token: &str) -> bool {\n    let key = \"secret\";\n    token == key\n}\n",
        )
        .unwrap();

        let tool = SearchCodebaseTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchCodebaseArgs {
                query: "authentication token verify".to_string(),
                target_directories: None,
                max_results: None,
            })
            .await
            .unwrap();

        // 应包含带行号的代码块（行号与 read_file 对齐：右对齐 + 两空格）
        assert!(out.contains("3  pub fn verify_token"), "out: {out}");
        assert!(out.contains("4      let key = \"secret\";"), "out: {out}");
        assert!(out.contains("得分:"), "out: {out}");
        assert!(out.contains("匹配关键词:"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_skips_non_code_files() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // .txt 文件不在 CODE_EXTS 中
        std::fs::write(dir.join("auth.txt"), "verify_token secret\n").unwrap();
        // .lock 文件也不在
        std::fs::write(dir.join("Cargo.lock"), "auth verify\n").unwrap();
        std::fs::write(dir.join("auth.rs"), "pub fn auth() {}\n").unwrap();

        let tool = SearchCodebaseTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchCodebaseArgs {
                query: "auth".to_string(),
                target_directories: None,
                max_results: None,
            })
            .await
            .unwrap();

        assert!(out.contains("auth.rs"), "out: {out}");
        assert!(!out.contains("auth.txt"), "out: {out}");
        assert!(!out.contains("Cargo.lock"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_skips_generated_dirs() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::write(dir.join("target/debug/out.rs"), "auth verify\n").unwrap();
        std::fs::write(dir.join("main.rs"), "pub fn auth() {}\n").unwrap();

        let tool = SearchCodebaseTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchCodebaseArgs {
                query: "auth".to_string(),
                target_directories: None,
                max_results: None,
            })
            .await
            .unwrap();

        assert!(out.contains("main.rs"), "out: {out}");
        assert!(!out.contains("target"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_respects_max_results() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // 创建 5 个匹配文件
        for i in 0..5 {
            std::fs::write(
                dir.join(format!("auth{i}.rs")),
                format!("pub fn auth_verify_{i}() -> bool {{ true }}\n"),
            )
            .unwrap();
        }

        let tool = SearchCodebaseTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchCodebaseArgs {
                query: "auth verify".to_string(),
                target_directories: None,
                max_results: Some(3),
            })
            .await
            .unwrap();

        assert!(out.contains("找到 3 个相关代码块"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_no_match_returns_message() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();

        let tool = SearchCodebaseTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchCodebaseArgs {
                query: "nonexistent concept xyz".to_string(),
                target_directories: None,
                max_results: None,
            })
            .await
            .unwrap();

        assert!(out.contains("未找到"), "out: {out}");
        assert!(out.contains("nonexistent concept xyz"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_rejects_empty_query() {
        let tool = SearchCodebaseTool::new();
        let r = tool
            .call(SearchCodebaseArgs {
                query: "   ".to_string(),
                target_directories: None,
                max_results: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("query"));
    }

    #[tokio::test]
    async fn search_rejects_stopwords_only_query() {
        let tool = SearchCodebaseTool::new();
        let r = tool
            .call(SearchCodebaseArgs {
                query: "the a an is how".to_string(),
                target_directories: None,
                max_results: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("无法从查询"));
    }

    #[tokio::test]
    async fn search_target_directories() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("sub1")).unwrap();
        std::fs::create_dir_all(dir.join("sub2")).unwrap();
        std::fs::write(dir.join("sub1/auth.rs"), "pub fn auth() {}\n").unwrap();
        std::fs::write(dir.join("sub2/other.rs"), "pub fn other() {}\n").unwrap();

        let tool = SearchCodebaseTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchCodebaseArgs {
                query: "auth".to_string(),
                target_directories: Some(vec!["sub1".to_string()]),
                max_results: None,
            })
            .await
            .unwrap();

        assert!(out.contains("sub1/auth.rs"), "out: {out}");
        assert!(!out.contains("sub2/other.rs"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_multiple_target_directories() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("sub1")).unwrap();
        std::fs::create_dir_all(dir.join("sub2")).unwrap();
        std::fs::write(dir.join("sub1/auth.rs"), "pub fn auth() {}\n").unwrap();
        std::fs::write(dir.join("sub2/token.rs"), "pub fn token() {}\n").unwrap();

        let tool = SearchCodebaseTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchCodebaseArgs {
                query: "auth token".to_string(),
                target_directories: Some(vec!["sub1".to_string(), "sub2".to_string()]),
                max_results: None,
            })
            .await
            .unwrap();

        assert!(out.contains("sub1/auth.rs"), "out: {out}");
        assert!(out.contains("sub2/token.rs"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_skips_binary_files() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // 含 NUL 字节的"二进制"代码文件（按扩展名是 .rs，但内容是二进制）
        std::fs::write(dir.join("bin.rs"), b"\x00\x01\x02auth\x00\x03").unwrap();
        std::fs::write(dir.join("real.rs"), "pub fn auth() {}\n").unwrap();

        let tool = SearchCodebaseTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchCodebaseArgs {
                query: "auth".to_string(),
                target_directories: None,
                max_results: None,
            })
            .await
            .unwrap();

        assert!(out.contains("real.rs"), "out: {out}");
        assert!(!out.contains("bin.rs"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_chinese_query() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("user.rs"),
            "pub struct User {\n    pub name: String,\n    pub id: u64,\n}\n\npub fn create_user(name: &str) -> User {\n    User { name: name.to_string(), id: 0 }\n}\n",
        )
        .unwrap();

        let tool = SearchCodebaseTool::with_cwd(dir.clone());
        let out = tool
            .call(SearchCodebaseArgs {
                query: "user create".to_string(),
                target_directories: None,
                max_results: None,
            })
            .await
            .unwrap();

        assert!(out.contains("user.rs"), "out: {out}");
        assert!(out.contains("create_user"), "out: {out}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
