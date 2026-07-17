//! web_fetch 工具：让 LLM 抓取网页内容
//!
//! 用 reqwest 发起 GET 请求，设置 User-Agent，超时 15s。
//! 返回响应体的前 max_chars 字符。HTML 标签做最简单的剥离
//! （去除 <...> 标签和 script/style 块），LLM 能直接理解。
//! 非 2xx 状态码视为错误。

use rig_core::tool::Tool;
use serde::Deserialize;

/// 默认最大返回字符数
const DEFAULT_MAX_CHARS: usize = 8000;
/// HTTP 请求超时（15 秒）
const REQUEST_TIMEOUT_SECS: u64 = 15;
/// User-Agent 标识
const USER_AGENT: &str = "EffiSuite-Agent/0.1 (+https://github.com/EffiSuite/EffiSuite)";

/// 工具参数
///
/// 字段按大小降序：String（24B）> Option<usize>（16B）。
#[derive(Deserialize)]
pub struct WebFetchArgs {
    /// 要抓取的 URL
    pub url: String,
    /// 最大返回字符数，默认 8000
    #[serde(default)]
    pub max_chars: Option<usize>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("web_fetch error: {0}")]
pub struct WebFetchError(String);

/// 网页抓取工具，无状态
pub struct WebFetchTool;

impl WebFetchTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for WebFetchTool {
    const NAME: &'static str = "web_fetch";

    type Error = WebFetchError;
    type Args = WebFetchArgs;
    type Output = String;

    fn description(&self) -> String {
        "抓取指定 URL 的网页内容并返回文本。GET 请求，超时 15s，\
         默认最多返回 8000 字符。会剥离 HTML 标签与 script/style 块。\
         非 2xx 状态码视为错误。适用于获取文档、API 响应等。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "要抓取的完整 URL（含 http/https）"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "最大返回字符数，默认 8000",
                    "default": DEFAULT_MAX_CHARS
                }
            },
            "required": ["url"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let max_chars = args.max_chars.unwrap_or(DEFAULT_MAX_CHARS).max(1);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| WebFetchError(format!("构建 HTTP client 失败: {e}")))?;

        let resp = client
            .get(&args.url)
            .send()
            .await
            .map_err(|e| WebFetchError(format!("请求失败 [{}]: {e}", args.url)))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(WebFetchError(format!(
                "HTTP 状态非 2xx [{}]: {} {}",
                args.url,
                status.as_u16(),
                status.canonical_reason().unwrap_or("Unknown")
            )));
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        let body = resp
            .text()
            .await
            .map_err(|e| WebFetchError(format!("读取响应体失败 [{}]: {e}", args.url)))?;

        // 若是 HTML，做最简单的标签剥离；否则原样返回
        let cleaned = if content_type.contains("html") {
            strip_html(&body)
        } else {
            body
        };

        // 截断到 max_chars 字符（在 UTF-8 字符边界处）
        let truncated = if cleaned.len() > max_chars {
            let mut end = max_chars;
            if end > cleaned.len() {
                end = cleaned.len();
            }
            // 回退到字符边界
            while end > 0 && !cleaned.is_char_boundary(end) {
                end -= 1;
            }
            cleaned[..end].to_string()
        } else {
            cleaned
        };

        Ok(truncated)
    }
}

/// 极简 HTML 文本剥离
///
/// 1. 移除 <script>...</script> 和 <style>...</style> 块（含内容）
/// 2. 移除所有 <...> 标签
/// 3. 解码常见 HTML 实体（&amp; &lt; &gt; &quot; &#39; &nbsp;）
/// 4. 折叠多余空白
///
/// 不使用第三方 HTML 解析库，保持依赖最小。
/// 使用 char 边界迭代，UTF-8 安全。
fn strip_html(html: &str) -> String {
    let lower = html.to_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;

    while i < html.len() {
        // 检测 <script 或 <style 块（带词边界，避免误匹配 <scripting>）
        if let Some(close) = block_close_tag(&lower[i..]) {
            if let Some(pos) = lower[i..].find(close) {
                i += pos + close.len();
                continue;
            }
            // 找不到闭合标签则按普通标签处理，避免吞掉后续内容
        }

        // 检测普通标签 <...>
        if html.as_bytes()[i] == b'<' {
            match html[i..].find('>') {
                Some(pos) => {
                    i += pos + 1;
                    continue;
                }
                None => {
                    // 未闭合的 <，原样输出并前进 1 字节（'<' 是 ASCII）
                    out.push('<');
                    i += 1;
                    continue;
                }
            }
        }

        // 拷贝下一个字符（UTF-8 安全：按字符边界前进）
        let ch = html[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    // 解码常见实体
    let decoded = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // 折叠连续空白
    collapse_whitespace(&decoded)
}

/// 判断字符串是否以 `<script` 或 `<style` 块标签开头（带词边界）
///
/// 返回对应的闭合标签，用于块跳过。例如 `<script>`、`<script `、`<script\n`
/// 都视为 script 块开始；但 `<scripting>` 不算（因为 `i` 是字母）。
fn block_close_tag(s: &str) -> Option<&'static str> {
    if let Some(rest) = s.strip_prefix("<script") {
        return match rest.chars().next() {
            None => Some("</script>"),
            Some(c) if c.is_alphanumeric() => None,
            _ => Some("</script>"),
        };
    }
    if let Some(rest) = s.strip_prefix("<style") {
        return match rest.chars().next() {
            None => Some("</style>"),
            Some(c) if c.is_alphanumeric() => None,
            _ => Some("</style>"),
        };
    }
    None
}

/// 折叠连续空白为单个空格，保留换行
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c == '\n' || c == '\r' {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            prev_space = false;
        } else if c.is_whitespace() {
            prev_space = true;
        } else {
            if prev_space {
                out.push(' ');
                prev_space = false;
            }
            out.push(c);
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_removes_tags() {
        let html = "<html><body><h1>Hello</h1><p>World &amp; <b>Rust</b></p></body></html>";
        let text = strip_html(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World & Rust"));
        assert!(!text.contains("<"));
    }

    #[test]
    fn strip_html_removes_script_blocks() {
        let html = "<p>before</p><script>alert('x')</script><p>after</p>";
        let text = strip_html(html);
        assert!(text.contains("before"));
        assert!(text.contains("after"));
        assert!(!text.contains("alert"));
        assert!(!text.contains("<script"));
    }

    #[test]
    fn strip_html_removes_style_blocks() {
        let html = "<style>body { color: red; }</style><p>content</p>";
        let text = strip_html(html);
        assert!(text.contains("content"));
        assert!(!text.contains("color"));
        assert!(!text.contains("<style"));
    }

    #[test]
    fn collapse_whitespace_compacts_runs() {
        let s = "hello    world\n\n\nfoo";
        let out = collapse_whitespace(s);
        assert_eq!(out, "hello world\nfoo");
    }
}
