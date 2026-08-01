//! RAG 记忆增强：分词器
//!
//! 提供索引侧与查询侧共用的 [`tokenize`] 实现：按空白与中英文标点切分，
//! 对 CJK 字符做**单字 + bigram（二元字组）**拆分，转小写。
//!
//! 该函数是 [`MemoryIndex`](super::MemoryIndex) 的唯一分词实现，
//! `search_history`/`storage` 等模块通过 `effisuite_core::tokenize` 复用，
//! 保证索引侧与查询侧行为一致。

/// 分词：按空白与中英文标点切分，对 CJK 字符做**单字 + bigram（二元字组）**拆分，转小写
///
/// 这是 `MemoryIndex` 的唯一分词实现，`search_history`/`storage` 等模块通过
/// `effisuite_core::tokenize` 复用，保证索引侧与查询侧行为一致。
///
/// # 为什么对 CJK 做单字 + bigram？
///
/// - 纯整串作 token 会导致 BM25 倒排表只能精确命中整串：索引 "我们讨论过异步编程"
///   生成单个 token，查询 "异步" 时倒排表中没有 "异步" 这个 key，召回率为 0。
/// - 纯单字精度低、噪声大（如 "的"/"是" 命中大量文档，IDF 失效）。
/// - bigram 是无词典中文检索的经典折中：召回与精度兼顾，零依赖，跨平台稳定。
///
/// # 示例
///
/// - `"异步编程"` → `["异步", "步编", "编程", "异", "步", "编", "程"]`
/// - `"Rust 编程"` → `["rust", "编程", "编", "程"]`
pub fn tokenize(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(content.len() / 2 + 4);
    let mut ascii_buf = String::new();
    let mut cjk_buf: Vec<char> = Vec::new();

    for c in content.chars() {
        let is_sep = c.is_whitespace() || "，。、；：！？,.:;!?\"'`()[]{}【】《》".contains(c);
        if is_sep {
            flush_ascii(&mut ascii_buf, &mut out);
            flush_cjk(&mut cjk_buf, &mut out);
        } else if is_cjk(c) {
            flush_ascii(&mut ascii_buf, &mut out);
            cjk_buf.push(c);
        } else {
            flush_cjk(&mut cjk_buf, &mut out);
            ascii_buf.push(c);
        }
    }
    flush_ascii(&mut ascii_buf, &mut out);
    flush_cjk(&mut cjk_buf, &mut out);
    out
}

/// 判定字符是否为 CJK（中日韩）表意文字或假名
///
/// 覆盖 CJK 统一表意文字主区、扩展 A/B、平假名、片假名。
/// 仅对 CJK 做单字拆分；ASCII 与其他文字按整词切分。
#[inline]
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'      // CJK 统一表意文字（主区）
        | '\u{3400}'..='\u{4DBF}'    // CJK 扩展 A
        | '\u{20000}'..='\u{2A6DF}'  // CJK 扩展 B
        | '\u{3040}'..='\u{309F}'    // 平假名
        | '\u{30A0}'..='\u{30FF}'    // 片假名
    )
}

/// flush ASCII / 非分词缓冲区为一个整词 token，转小写
#[inline]
fn flush_ascii(buf: &mut String, out: &mut Vec<String>) {
    if !buf.is_empty() {
        out.push(buf.to_lowercase());
        buf.clear();
    }
}

/// flush CJK 缓冲区：先输出 bigram（相邻二字组合），再输出单字
///
/// bigram 优先于单字写入顺序无语义影响（BM25 按 token 汇总分数）。
/// 单字 + bigram 同时索引：查询 "异步" 会命中 bigram "异步"（高 IDF），
/// 也会命中 "异"/"步" 单字（低 IDF），分数对称。
#[inline]
fn flush_cjk(cjk: &mut Vec<char>, out: &mut Vec<String>) {
    if cjk.is_empty() {
        return;
    }
    // bigram（相邻二字组合）
    for w in cjk.windows(2) {
        let mut s = String::with_capacity(8);
        s.push(w[0]);
        s.push(w[1]);
        out.push(s);
    }
    // 单字
    for c in cjk.iter() {
        out.push(c.to_string());
    }
    cjk.clear();
}
