//! 评分：简化版 TF-IDF 评分与代码块定位

use super::constants::{MAX_BACKWARD_SEARCH, MAX_BLOCK_LINES};

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
pub(super) fn score_file(
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
pub(super) fn position_weight(line: &str, lower_line: &str) -> f64 {
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
pub(super) fn is_definition_line(line: &str) -> bool {
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
pub(super) fn find_code_block(lines: &[&str], hit_line: usize) -> (usize, usize) {
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
