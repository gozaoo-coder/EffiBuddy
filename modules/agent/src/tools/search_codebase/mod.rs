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
//!
//! # 模块组织
//!
//! - [`constants`]：扫描与评分常量
//! - [`keywords`]：关键词提取（分词、去停用词、CJK 处理）
//! - [`walker`]：搜索根解析与文件过滤
//! - [`scorer`]：简化版 TF-IDF 评分与代码块定位

use std::collections::HashSet;
use std::path::PathBuf;

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::fs;

use super::text_utils::{format_numbered_line, line_number_width};

mod constants;
mod keywords;
mod scorer;
mod walker;

use constants::{MAX_FILE_BYTES, MAX_RESULTS, MAX_SCAN_FILES, SKIP_DIRS};
use keywords::extract_keywords;
use scorer::{find_code_block, position_weight, score_file};
use walker::{is_binary, is_code_file, resolve_roots};

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
        "用自然语言描述搜索代码库中的相关代码（基于关键词加权排序，非精确匹配）。\
         输入一句自然语言描述（如「how does authentication work」），自动提取关键词后检索最相关代码块。\
         返回每个结果：文件路径、命中关键词、得分、代码块（带行号），自动扩展到完整函数/结构体。\
         自动跳过生成目录与非代码文件，仅扫描代码文件（rs/py/js/ts/go/java/c/cpp/...）；默认最多 20 个结果。\
         分工：文件名→list_files/glob；字面关键词→search_file；正则→grep。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "自然语言查询（描述想找什么代码）" },
                "target_directories": { "type": "array", "items": { "type": "string" }, "description": "搜索根目录数组，默认整个工作区" },
                "max_results": { "type": "integer", "description": "最大返回结果数，默认 20", "default": MAX_RESULTS }
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

#[cfg(test)]
mod tests;
#[cfg(test)]
mod integration_tests;
