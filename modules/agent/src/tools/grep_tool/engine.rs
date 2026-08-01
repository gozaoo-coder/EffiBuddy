//! grep 匹配引擎：辅助函数与命中收集逻辑。
//!
//! 包含二进制探测、glob 匹配、行号映射、逐行/多行命中收集、
//! 路径展示与输出容量预估等纯函数，由 `mod.rs` 的 `Tool::call` 调用。

use std::collections::BTreeSet;

use regex::Regex;

use super::{MAX_MATCHES_PER_FILE, MODE_CONTENT, MODE_COUNT, MODE_FILES};

/// 判断是否为二进制文件：探测前 8KB 是否含 NUL 字节
#[inline]
pub(super) fn is_binary(bytes: &[u8]) -> bool {
    let probe = &bytes[..bytes.len().min(8192)];
    probe.contains(&0)
}

/// 简单 glob 匹配（支持 `*` 任意序列、`?` 单字符），只对文件名匹配，不对路径。
/// 采用经典双指针 + 回溯算法，O(n) 时间 O(1) 空间。
#[inline]
pub(super) fn glob_match(pattern: &str, name: &str) -> bool {
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
pub(super) fn line_index_of(line_starts: &[usize], offset: usize) -> usize {
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
pub(super) fn compute_line_starts(text: &str) -> Vec<usize> {
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
pub(super) fn collect_hits_linebyline(re: &Regex, file_lines: &[&str]) -> Vec<usize> {
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
pub(super) fn collect_hits_multiline(re: &Regex, text: &str, total_lines: usize) -> Vec<usize> {
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

/// 把路径转为展示路径：优先相对工作区 cwd（让 LLM 回传 read_file/edit_file 时路径一致，
/// 避免 path=sub 时返回 "inner.rs" 导致 read_file 解析为 cwd/inner.rs 找不到），
/// 其次相对搜索根，最后用绝对路径。Windows 下统一为 `/` 分隔。
#[inline]
pub(super) fn display_path(
    path: &std::path::Path,
    cwd: Option<&std::path::Path>,
    root: &std::path::Path,
) -> String {
    let display = cwd
        .and_then(|c| path.strip_prefix(c).ok())
        .or_else(|| path.strip_prefix(root).ok())
        .map(|r| r.display().to_string())
        .unwrap_or_else(|| path.display().to_string());
    if cfg!(windows) {
        display.replace('\\', "/")
    } else {
        display
    }
}

/// 按 mode + 命中规模预估输出缓冲区容量，减少 String 扩容拷贝。
#[inline]
pub(super) fn estimate_capacity(mode: &str, total_hits: usize, max_matches: usize) -> usize {
    let n = total_hits.min(max_matches);
    match mode {
        MODE_CONTENT => n * 96 + 256,
        MODE_FILES => n * 64 + 256,
        MODE_COUNT => n * 80 + 256,
        _ => 256,
    }
}
