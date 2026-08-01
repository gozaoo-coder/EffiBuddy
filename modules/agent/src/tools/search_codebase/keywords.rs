//! 关键词提取：从自然语言查询中分词、去停用词、转小写、去重

use std::collections::HashSet;

use super::constants::STOP_WORDS;

/// 从自然语言查询中提取关键词
///
/// 步骤：
/// 1. 按非字母数字字符分词（保留中文连续字符与下划线）
/// 2. 转小写
/// 3. 去停用词
/// 4. 去重（保留首次出现顺序）
/// 5. 过滤太短的词（<2 字符，但保留中文单字）
/// 6. 纯 CJK 多字 token 额外拆分单字（中文无空格分词，单字允许更宽匹配）
pub(super) fn extract_keywords(query: &str) -> Vec<String> {
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
