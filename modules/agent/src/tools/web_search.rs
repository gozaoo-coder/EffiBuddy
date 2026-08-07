//! web_search 工具：让 LLM 通过搜索引擎 API 搜索网络信息
//!
//! 支持多引擎：
//! - **Serper (Google)**：POST `{base_url}/search`，头 `X-API-KEY`
//! - **Bing**：GET `{base_url}?q=...`，头 `Ocp-Apim-Subscription-Key`
//! - **通用**：可配置 base_url，按 Serper 兼容格式解析（默认引擎）
//!
//! 返回格式化的搜索结果列表（标题、URL、摘要）。工具持有搜索 API
//! 配置的**共享句柄** `Arc<RwLock<Option<WebSearchConfig>>>`：Tauri 命令层
//! 在用户切换搜索引擎时更新，工具调用时读取最新配置。None 时工具不可用，
//! 调用返回友好错误。
//!
//! 性能：结构体字段按大小降序；错误信息截断避免过长；结果 Vec 用迭代器
//! 链构建；reqwest Client 每次调用创建（可接受，Client 内部 Arc 共享连接池）；
//! 读锁临界区极短——仅 clone 配置快照后立即释放，HTTP IO 全在锁外。

use std::sync::Arc;
use std::time::Duration;

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

/// 默认返回结果数量
const DEFAULT_NUM_RESULTS: usize = 5;
/// 最大返回结果数量（钳制上限，防止滥用）
const MAX_NUM_RESULTS: usize = 10;
/// HTTP 请求超时（15 秒）
const REQUEST_TIMEOUT_SECS: u64 = 15;
/// User-Agent 标识
const USER_AGENT: &str = "EffiSuite-Agent/0.1 (+https://github.com/EffiSuite/EffiSuite)";
/// 错误信息最大字符数（截断 API 返回的冗长错误体）
const MAX_ERR_CHARS: usize = 500;

/// Serper 默认端点
const SERPER_DEFAULT_ENDPOINT: &str = "https://google.serper.dev/search";
/// Bing 默认端点
const BING_DEFAULT_ENDPOINT: &str = "https://api.bing.microsoft.com/v7.0/search";

/// 搜索结果项
///
/// 字段均为 `String`（24B），大小一致，无需重排。
#[derive(Debug, serde::Serialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// 工具参数
///
/// 字段按大小降序：`String` / `Option<String>`（均 24B，得益于 `String` 的
/// `NonNull` niche 优化使 `Option<String>` 零额外开销）> `Option<usize>`（16B，
/// `usize` 无 niche 故带判别位）。serde 按字段名反序列化，声明顺序不影响解析。
#[derive(Deserialize)]
pub struct WebSearchArgs {
    /// 搜索查询
    pub query: String,
    /// 语言限制（如 "lang_en" / "lang_zh"），可选
    #[serde(default)]
    pub lr: Option<String>,
    /// 返回结果数量，默认 5，最大 10
    #[serde(default)]
    pub num: Option<usize>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("web_search error: {0}")]
pub struct WebSearchError(String);

/// 搜索引擎配置
///
/// 字段均为 `String`（24B），大小一致。`engine` 取值：
/// "serper" / "google" → Serper 格式；"bing" → Bing 格式；其他 → Serper 兼容。
#[derive(Clone)]
pub struct WebSearchConfig {
    pub api_key: String,
    pub base_url: String,
    pub engine: String,
}

/// 网络搜索工具
///
/// `config` 为共享句柄：Tauri 命令层在用户切换搜索引擎时更新，
/// 工具调用时读取最新配置。None 时工具不可用（返回错误）。
pub struct WebSearchTool {
    config: Arc<RwLock<Option<WebSearchConfig>>>,
}

impl WebSearchTool {
    /// 创建工具：传入共享配置句柄。
    ///
    /// 推荐用法：调用层持有 `Arc<RwLock<Option<WebSearchConfig>>>`，
    /// 用户在设置中切换搜索引擎时 write 更新，工具每次调用 read 最新值。
    pub fn new(config: Arc<RwLock<Option<WebSearchConfig>>>) -> Self {
        Self { config }
    }

    /// 创建不可用工具（config = None 的独立句柄）。
    ///
    /// 用于无配置场景下仍想注册工具占位的情况；调用时会返回友好错误。
    pub fn disabled() -> Self {
        Self::new(Arc::new(RwLock::new(None)))
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::disabled()
    }
}

impl Tool for WebSearchTool {
    const NAME: &'static str = "web_search";

    type Error = WebSearchError;
    type Args = WebSearchArgs;
    type Output = String;

  fn description(&self) -> String {
      "通过搜索引擎 API 搜索网络信息，返回结果（标题、URL、摘要）。\
       支持数量控制（默认 5，最大 10）与语言限制（lang_en/lang_zh）。"
          .to_string()
  }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜索查询关键词"
                },
                "num": {
                    "type": "integer",
                    "description": "返回结果数量，默认 5，最大 10",
                    "default": DEFAULT_NUM_RESULTS
                },
                "lr": {
                    "type": "string",
                      "description": "语言限制：lang_en/lang_zh",
                    "default": null
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 空查询拒绝（trim 后为空）：纯校验，先于配置检查，防御性更强
        let query = args.query.trim();
        if query.is_empty() {
            return Err(WebSearchError("查询不能为空".into()));
        }

        // 读锁取配置快照：临界区极短，仅 clone 后立即释放，HTTP IO 全在锁外。
        // 用户在设置中切换搜索引擎后，下次调用即可读到新配置（动态生效）。
        let cfg = self.config.read().await.clone().ok_or_else(|| {
            WebSearchError(
                "未配置搜索引擎（api_key/base_url/engine），web_search 工具不可用".into(),
            )
        })?;

        let num = resolve_num(args.num);

        // 构建复用 client（Client 内部 Arc 共享连接池，每次创建开销极低）
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| WebSearchError(format!("构建 HTTP client 失败: {e}")))?;

        // 按引擎分发：bing 单独走，其余（serper/google/未知）按 Serper 兼容格式
        let results = if cfg.engine.eq_ignore_ascii_case("bing") {
            search_bing(&client, &cfg, query, num, args.lr.as_deref()).await?
        } else {
            search_serper(&client, &cfg, query, num, args.lr.as_deref()).await?
        };

        Ok(format_results(query, &results))
    }
}

// ---------------------------------------------------------------------------
// 引擎实现
// ---------------------------------------------------------------------------

/// Serper (Google) 搜索：POST `{base_url}/search`
///
/// - 请求头：`X-API-KEY: {api_key}`
/// - 请求体：`{ "q": query, "num": n, "hl": lang }`
/// - 响应：`{ "organic": [{ "title", "link", "snippet" }] }`
///
/// `base_url` 为空时用默认端点；已含 `/search` 后缀则原样使用。
async fn search_serper(
    client: &reqwest::Client,
    cfg: &WebSearchConfig,
    query: &str,
    num: usize,
    lr: Option<&str>,
) -> Result<Vec<SearchResult>, WebSearchError> {
    let endpoint = serper_endpoint(&cfg.base_url);

    let mut body = serde_json::json!({
        "q": query,
        "num": num,
    });
    if let Some(lang) = lr.and_then(strip_lang_prefix) {
        body["hl"] = serde_json::Value::String(lang.to_string());
    }

    let resp = client
        .post(&endpoint)
        .header("X-API-KEY", &cfg.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| WebSearchError(format!("Serper 请求失败: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(WebSearchError(format!(
            "Serper API 返回错误 {}: {}",
            status,
            truncate(&text, MAX_ERR_CHARS)
        )));
    }

    let data: SerperResponse = resp
        .json()
        .await
        .map_err(|e| WebSearchError(format!("解析 Serper 响应失败: {e}")))?;

    // take(num) 作为安全网：即使 API 返回更多也只取请求的数量
    Ok(data
        .organic
        .into_iter()
        .take(num)
        .map(|o| SearchResult {
            title: o.title.unwrap_or_default(),
            url: o.link.unwrap_or_default(),
            snippet: o.snippet.unwrap_or_default(),
        })
        .collect())
}

/// Bing 搜索：GET `{base_url}?q=...&count=...&setLang=...`
///
/// - 请求头：`Ocp-Apim-Subscription-Key: {api_key}`
/// - 响应：`{ "webPages": { "value": [{ "name", "url", "snippet" }] } }`
///
/// `base_url` 为空时用默认端点。
async fn search_bing(
    client: &reqwest::Client,
    cfg: &WebSearchConfig,
    query: &str,
    num: usize,
    lr: Option<&str>,
) -> Result<Vec<SearchResult>, WebSearchError> {
    let endpoint = bing_endpoint(&cfg.base_url);
    // count 需存活至 query() 调用结束（reqwest 立即序列化 query，但绑定变量更稳）
    let count = num.to_string();

    let mut req = client
        .get(&endpoint)
        .header("Ocp-Apim-Subscription-Key", &cfg.api_key)
        .query(&[("q", query), ("count", count.as_str())]);

    // setLang 接受 2 字母语言码，与 lr 的 "lang_xx" 格式经 strip 后天然匹配
    if let Some(lang) = lr.and_then(strip_lang_prefix) {
        req = req.query(&[("setLang", lang)]);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| WebSearchError(format!("Bing 请求失败: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(WebSearchError(format!(
            "Bing API 返回错误 {}: {}",
            status,
            truncate(&text, MAX_ERR_CHARS)
        )));
    }

    let data: BingResponse = resp
        .json()
        .await
        .map_err(|e| WebSearchError(format!("解析 Bing 响应失败: {e}")))?;

    let pages = data.web_pages.map(|w| w.value).unwrap_or_default();
    Ok(pages
        .into_iter()
        .take(num)
        .map(|p| SearchResult {
            title: p.name.unwrap_or_default(),
            url: p.url.unwrap_or_default(),
            snippet: p.snippet.unwrap_or_default(),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// 响应反序列化结构
// ---------------------------------------------------------------------------

/// Serper 响应：`{ "organic": [...] }`
#[derive(Deserialize)]
struct SerperResponse {
    #[serde(default)]
    organic: Vec<SerperOrganic>,
}

#[derive(Deserialize)]
struct SerperOrganic {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
}

/// Bing 响应：`{ "webPages": { "value": [...] } }`（字段为 camelCase）
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BingResponse {
    #[serde(default)]
    web_pages: Option<BingWebPages>,
}

#[derive(Deserialize)]
struct BingWebPages {
    #[serde(default)]
    value: Vec<BingWebPage>,
}

#[derive(Deserialize)]
struct BingWebPage {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
}

// ---------------------------------------------------------------------------
// 纯函数辅助（可单测，无网络依赖）
// ---------------------------------------------------------------------------

/// 格式化搜索结果为可读文本
///
/// 输出示例：
/// ```text
/// 搜索 "rust async" 共找到 2 条结果：
///
/// 1. [Title](url)
///    snippet...
///
/// 2. [Title2](url2)
///    snippet2...
/// ```
fn format_results(query: &str, results: &[SearchResult]) -> String {
    use std::fmt::Write;
    let n = results.len();
    // 预估容量：头部 ~64B + 每条 ~256B（标题+url+摘要+序号+缩进）
    let mut out = String::with_capacity(64 + n * 256);
    let _ = writeln!(out, "搜索 \"{query}\" 共找到 {n} 条结果：\n");
    for (i, r) in results.iter().enumerate() {
        let _ = writeln!(out, "{}. [{}]({})", i + 1, r.title, r.url);
        let _ = writeln!(out, "   {}", r.snippet);
        if i + 1 < n {
            out.push('\n');
        }
    }
    out
}

/// 解析并钳制结果数量：默认 5，范围 [1, 10]
#[inline]
fn resolve_num(num: Option<usize>) -> usize {
    num.unwrap_or(DEFAULT_NUM_RESULTS).clamp(1, MAX_NUM_RESULTS)
}

/// 从 "lang_en" 提取 "en"；若已是裸语言码（如 "en"）则原样返回。
/// 全空白返回 None。
#[inline]
fn strip_lang_prefix(lr: &str) -> Option<&str> {
    // 先 trim 输入再 strip_prefix：处理 "  lang_fr  " 这类带空格的输入
    // （trim 返回的 &str 仍引用原字符串，无分配）
    let trimmed = lr.trim();
    let code = trimmed.strip_prefix("lang_").unwrap_or(trimmed);
    if code.is_empty() {
        None
    } else {
        Some(code)
    }
}

/// 解析 Serper 端点：空 → 默认；已含 `/search` → 原样；否则追加 `/search`
fn serper_endpoint(base_url: &str) -> String {
    let base = base_url.trim();
    if base.is_empty() {
        return SERPER_DEFAULT_ENDPOINT.to_string();
    }
    let base = base.trim_end_matches('/');
    if base.ends_with("/search") {
        base.to_string()
    } else {
        format!("{base}/search")
    }
}

/// 解析 Bing 端点：空 → 默认；否则去尾部 `/` 原样使用
fn bing_endpoint(base_url: &str) -> String {
    let base = base_url.trim();
    if base.is_empty() {
        BING_DEFAULT_ENDPOINT.to_string()
    } else {
        base.trim_end_matches('/').to_string()
    }
}

/// 截断字符串到 max 字符，避免错误信息过长（错误路径，短串，性能非关键）
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push_str("...");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_core::tool::Tool;

    fn mock_config(engine: &str) -> WebSearchConfig {
        WebSearchConfig {
            api_key: "test-key".into(),
            base_url: String::new(),
            engine: engine.into(),
        }
    }

    /// 用一份配置构造工具（共享句柄持有 Some(cfg)），便于测试
    fn tool_with_config(cfg: WebSearchConfig) -> WebSearchTool {
        WebSearchTool::new(Arc::new(RwLock::new(Some(cfg))))
    }

    fn mock_results() -> Vec<SearchResult> {
        vec![
            SearchResult {
                title: "Asynchronous Programming in Rust".into(),
                url: "https://doc.rust-lang.org/async-book/".into(),
                snippet: "Rust 异步编程官方指南，介绍 async/await 语法和运行时...".into(),
            },
            SearchResult {
                title: "Tokio: A runtime for writing reliable async software".into(),
                url: "https://tokio.rs/".into(),
                snippet: "Tokio 是 Rust 最流行的异步运行时...".into(),
            },
        ]
    }

    #[test]
    fn format_results_renders_list() {
        let results = mock_results();
        let out = format_results("rust async runtime", &results);
        assert!(out.contains("搜索 \"rust async runtime\" 共找到 2 条结果"));
        assert!(out.contains("1. [Asynchronous Programming in Rust]"));
        assert!(out.contains("https://doc.rust-lang.org/async-book/"));
        assert!(out.contains("2. [Tokio: A runtime"));
        assert!(out.contains("Tokio 是 Rust 最流行的异步运行时"));
    }

    #[test]
    fn format_results_empty() {
        let out = format_results("nothing", &[]);
        assert!(out.contains("共找到 0 条结果"));
    }

    #[test]
    fn resolve_num_clamps() {
        assert_eq!(resolve_num(None), 5);
        assert_eq!(resolve_num(Some(0)), 1);
        assert_eq!(resolve_num(Some(1)), 1);
        assert_eq!(resolve_num(Some(3)), 3);
        assert_eq!(resolve_num(Some(10)), 10);
        assert_eq!(resolve_num(Some(100)), 10);
    }

    #[test]
    fn strip_lang_prefix_works() {
        assert_eq!(strip_lang_prefix("lang_en"), Some("en"));
        assert_eq!(strip_lang_prefix("lang_zh"), Some("zh"));
        assert_eq!(strip_lang_prefix("en"), Some("en"));
        assert_eq!(strip_lang_prefix("  lang_fr  "), Some("fr"));
        assert_eq!(strip_lang_prefix(""), None);
        assert_eq!(strip_lang_prefix("   "), None);
    }

    #[test]
    fn serper_endpoint_resolution() {
        assert_eq!(serper_endpoint(""), SERPER_DEFAULT_ENDPOINT);
        assert_eq!(
            serper_endpoint("https://google.serper.dev"),
            "https://google.serper.dev/search"
        );
        assert_eq!(
            serper_endpoint("https://google.serper.dev/"),
            "https://google.serper.dev/search"
        );
        assert_eq!(
            serper_endpoint("https://google.serper.dev/search"),
            "https://google.serper.dev/search"
        );
    }

    #[test]
    fn bing_endpoint_resolution() {
        assert_eq!(bing_endpoint(""), BING_DEFAULT_ENDPOINT);
        assert_eq!(
            bing_endpoint("https://api.bing.microsoft.com/v7.0/search"),
            "https://api.bing.microsoft.com/v7.0/search"
        );
        assert_eq!(
            bing_endpoint("https://api.bing.microsoft.com/v7.0/search/"),
            "https://api.bing.microsoft.com/v7.0/search"
        );
    }

    #[test]
    fn parameters_schema_has_defaults() {
        let tool = WebSearchTool::disabled();
        let params = tool.parameters();
        assert_eq!(
            params["properties"]["num"]["default"].as_u64(),
            Some(DEFAULT_NUM_RESULTS as u64)
        );
        assert_eq!(params["required"][0].as_str(), Some("query"));
    }

    #[tokio::test]
    async fn config_none_returns_error() {
        let tool = WebSearchTool::disabled();
        let res = tool
            .call(WebSearchArgs {
                query: "rust".into(),
                lr: None,
                num: None,
            })
            .await;
        assert!(res.is_err());
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("未配置") || msg.contains("不可用"), "got: {msg}");
    }

    #[tokio::test]
    async fn empty_query_rejected() {
        // 即便配置了引擎，空查询也先被拒绝
        let tool = tool_with_config(mock_config("serper"));
        let res = tool
            .call(WebSearchArgs {
                query: "   ".into(),
                lr: None,
                num: None,
            })
            .await;
        assert!(res.is_err());
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("空"), "got: {msg}");
    }

    #[test]
    fn tool_name_and_description() {
        let tool = WebSearchTool::disabled();
        assert_eq!(WebSearchTool::NAME, "web_search");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn serper_response_parses() {
        let json = r#"{
            "organic": [
                {"title": "A", "link": "https://a.com", "snippet": "sa"},
                {"title": "B", "link": "https://b.com", "snippet": "sb"}
            ]
        }"#;
        let resp: SerperResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.organic.len(), 2);
        assert_eq!(resp.organic[0].title.as_deref(), Some("A"));
        assert_eq!(resp.organic[1].link.as_deref(), Some("https://b.com"));
    }

    #[test]
    fn serper_response_empty_organic() {
        let json = r#"{"organic": []}"#;
        let resp: SerperResponse = serde_json::from_str(json).unwrap();
        assert!(resp.organic.is_empty());
    }

    #[test]
    fn serper_response_missing_organic_defaults() {
        let json = r#"{}"#;
        let resp: SerperResponse = serde_json::from_str(json).unwrap();
        assert!(resp.organic.is_empty());
    }

    #[test]
    fn bing_response_parses() {
        let json = r#"{
            "webPages": {
                "value": [
                    {"name": "A", "url": "https://a.com", "snippet": "sa"}
                ]
            }
        }"#;
        let resp: BingResponse = serde_json::from_str(json).unwrap();
        let pages = resp.web_pages.unwrap().value;
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].name.as_deref(), Some("A"));
        assert_eq!(pages[0].url.as_deref(), Some("https://a.com"));
    }

    #[test]
    fn bing_response_missing_webpages() {
        let json = r#"{}"#;
        let resp: BingResponse = serde_json::from_str(json).unwrap();
        assert!(resp.web_pages.is_none());
    }

    #[test]
    fn truncate_long_string() {
        let s = "x".repeat(10);
        let t = truncate(&s, 5);
        assert_eq!(t, "xxxxx...");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        let s = "hello";
        assert_eq!(truncate(s, 10), "hello");
    }
}
