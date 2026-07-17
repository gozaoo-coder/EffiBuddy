//! ClawHub HTTP API 客户端
//!
//! 封装 ClawHub 公共 REST API（`https://clawhub.ai/api/v1/...`），
//! 提供 Skills 与 Plugins 的列表 / 搜索 / 详情 / 下载能力。
//!
//! 设计要点：
//! - 全异步：基于 `reqwest`，与 tauri 命令层无锁衔接
//! - 零拷贝反序列化：响应 JSON 直接 `into_json::<T>()`，避免中间 `Value`
//! - 速率限制感知：429 时返回 `ClawHubError::RateLimited { retry_after }`，
//!   调用方可指数退避重试
//! - 紧凑类型：仅保留 UI / 安装流程必需字段，避免与 OpenAPI 1:1 复制
//! - 廉价 clone：`ClawHubClient` 内部 `Arc<reqwest::Client>`，可跨任务克隆
//!
//! 参考：<https://docs.openclaw.ai/clawhub/http-api>

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::CoreError;

/// ClawHub 站点根 URL（可被 `CLAWHUB_SITE` 覆盖，但此处固定默认值）
pub const CLAWHUB_BASE_URL: &str = "https://clawhub.ai";

/// HTTP 客户端默认超时（30s）。下载 ZIP 时按需覆盖到更长。
const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// 下载 ZIP 时的超时（5 min，ClawHub 下载限速 1200/min/IP）。
const DOWNLOAD_TIMEOUT_SECS: u64 = 300;

/// ClawHub 专用错误类型。
///
/// `RateLimited` 携带 `retry_after`（秒），便于上层实现抖动退避。
#[derive(Debug, thiserror::Error)]
pub enum ClawHubError {
    #[error("clawhub request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("clawhub rate limited; retry after {retry_after:?} seconds")]
    RateLimited { retry_after: Option<u64> },

    #[error("clawhub api error: status={status}, body={body}")]
    Api { status: u16, body: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("zip extract error: {0}")]
    Zip(String),

    #[error("invalid response: {0}")]
    Decode(String),

    #[error("skill slug not found: {0}")]
    SlugNotFound(String),

    #[error("package not found: {0}")]
    PackageNotFound(String),
}

impl From<ClawHubError> for CoreError {
    #[inline]
    fn from(e: ClawHubError) -> Self {
        CoreError::Config(e.to_string())
    }
}

impl From<ClawHubError> for String {
    /// 便于 Tauri 命令把错误直接以字符串返回前端
    #[inline]
    fn from(e: ClawHubError) -> Self {
        e.to_string()
    }
}

/// ClawHub 客户端：可被廉价 clone（内部 `Arc<reqwest::Client>`）
#[derive(Clone)]
pub struct ClawHubClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    /// 用于下载的超时配置单独构建一个 client（避免每次构造）
    http_long: reqwest::Client,
    base_url: String,
}

impl ClawHubClient {
    /// 创建默认客户端（指向 `https://clawhub.ai`）
    pub fn new() -> Self {
        Self::with_base_url(CLAWHUB_BASE_URL)
    }

    /// 创建指向指定 base_url 的客户端（用于自建镜像或测试）
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .user_agent(concat!("EffiSuite-ClawHub/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest::Client 构建失败");
        let http_long = reqwest::Client::builder()
            .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
            .user_agent(concat!("EffiSuite-ClawHub/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest::Client(long) 构建失败");
        Self {
            inner: Arc::new(Inner {
                http,
                http_long,
                base_url,
            }),
        }
    }

    /// 拼接完整 URL
    #[inline]
    fn url(&self, path: &str) -> String {
        // path 形如 "/api/v1/skills"
        format!("{}{}", self.inner.base_url, path)
    }

    /// 处理响应：429 → RateLimited；非 2xx → Api；2xx → 直接返回 bytes/text
    async fn handle_response(
        &self,
        resp: reqwest::Response,
    ) -> std::result::Result<reqwest::Response, ClawHubError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        if status.as_u16() == 429 {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());
            return Err(ClawHubError::RateLimited { retry_after });
        }
        let body = resp.text().await.unwrap_or_default();
        Err(ClawHubError::Api {
            status: status.as_u16(),
            body,
        })
    }

    // =========================================================
    // Skills：列表 / 搜索 / 详情
    // =========================================================

    /// `GET /api/v1/skills` - 列出技能（按更新时间倒序）
    ///
    /// `sort` 可选值：`updated`(默认) / `recommended` / `createdAt` / `downloads` / `stars` / `trending`
    pub async fn list_skills(
        &self,
        limit: Option<u32>,
        sort: Option<&str>,
        cursor: Option<&str>,
    ) -> std::result::Result<SkillListResponse, ClawHubError> {
        let mut req = self.inner.http.get(self.url("/api/v1/skills"));
        if let Some(l) = limit {
            req = req.query(&[("limit", l)]);
        }
        if let Some(s) = sort {
            req = req.query(&[("sort", s)]);
        }
        if let Some(c) = cursor {
            req = req.query(&[("cursor", c)]);
        }
        let resp = self.handle_response(req.send().await?).await?;
        resp.json().await.map_err(ClawHubError::Http)
    }

    /// `GET /api/v1/search?q=...` - 全文搜索技能（按相关性排序）
    pub async fn search_skills(
        &self,
        q: &str,
        limit: Option<u32>,
    ) -> std::result::Result<SearchResponse, ClawHubError> {
        let mut req = self
            .inner
            .http
            .get(self.url("/api/v1/search"))
            .query(&[("q", q)]);
        if let Some(l) = limit {
            req = req.query(&[("limit", l)]);
        }
        let resp = self.handle_response(req.send().await?).await?;
        resp.json().await.map_err(ClawHubError::Http)
    }

    /// `GET /api/v1/skills/{slug}` - 获取技能详情
    pub async fn get_skill(&self, slug: &str) -> std::result::Result<SkillResponse, ClawHubError> {
        let path = format!("/api/v1/skills/{}", urlencoding_encode(slug));
        let resp = self.handle_response(self.inner.http.get(self.url(&path)).send().await?).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ClawHubError::SlugNotFound(slug.to_string()));
        }
        resp.json().await.map_err(ClawHubError::Http)
    }

    // =========================================================
    // Plugins / Packages：列表 / 搜索 / 详情
    // =========================================================

    /// `GET /api/v1/plugins` - 列出插件（code-plugin + bundle-plugin）
    pub async fn list_plugins(
        &self,
        limit: Option<u32>,
        sort: Option<&str>,
        cursor: Option<&str>,
    ) -> std::result::Result<PackageListResponse, ClawHubError> {
        let mut req = self.inner.http.get(self.url("/api/v1/plugins"));
        if let Some(l) = limit {
            req = req.query(&[("limit", l)]);
        }
        if let Some(s) = sort {
            req = req.query(&[("sort", s)]);
        }
        if let Some(c) = cursor {
            req = req.query(&[("cursor", c)]);
        }
        let resp = self.handle_response(req.send().await?).await?;
        resp.json().await.map_err(ClawHubError::Http)
    }

    /// `GET /api/v1/plugins/search?q=...` - 搜索插件
    pub async fn search_plugins(
        &self,
        q: &str,
        limit: Option<u32>,
    ) -> std::result::Result<PackageSearchResponse, ClawHubError> {
        let mut req = self
            .inner
            .http
            .get(self.url("/api/v1/plugins/search"))
            .query(&[("q", q)]);
        if let Some(l) = limit {
            req = req.query(&[("limit", l)]);
        }
        let resp = self.handle_response(req.send().await?).await?;
        resp.json().await.map_err(ClawHubError::Http)
    }

    /// `GET /api/v1/packages/{name}` - 获取包详情（unified catalog，支持 skill 与 plugin）
    pub async fn get_package(
        &self,
        name: &str,
    ) -> std::result::Result<PackageResponse, ClawHubError> {
        let path = format!("/api/v1/packages/{}", urlencoding_encode(name));
        let resp = self.handle_response(self.inner.http.get(self.url(&path)).send().await?).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ClawHubError::PackageNotFound(name.to_string()));
        }
        resp.json().await.map_err(ClawHubError::Http)
    }

    // =========================================================
    // 下载
    // =========================================================

    /// `GET /api/v1/download?slug=...` - 下载技能 ZIP 字节流
    ///
    /// 返回原始字节，由调用方解压到目标目录。
    /// 使用 `http_long` client（5min 超时），适配大包下载。
    pub async fn download_skill_zip(
        &self,
        slug: &str,
        version: Option<&str>,
        tag: Option<&str>,
    ) -> std::result::Result<Vec<u8>, ClawHubError> {
        let mut req = self
            .inner
            .http_long
            .get(self.url("/api/v1/download"))
            .query(&[("slug", slug)]);
        if let Some(v) = version {
            req = req.query(&[("version", v)]);
        }
        if let Some(t) = tag {
            req = req.query(&[("tag", t)]);
        }
        let resp = self.handle_response(req.send().await?).await?;
        let bytes = resp.bytes().await.map_err(ClawHubError::Http)?;
        Ok(bytes.to_vec())
    }

    /// `GET /api/v1/packages/{name}/download` - 下载 plugin 包字节流
    pub async fn download_package(&self, name: &str) -> std::result::Result<Vec<u8>, ClawHubError> {
        let path = format!("/api/v1/packages/{}/download", urlencoding_encode(name));
        let resp = self
            .handle_response(self.inner.http_long.get(self.url(&path)).send().await?)
            .await?;
        let bytes = resp.bytes().await.map_err(ClawHubError::Http)?;
        Ok(bytes.to_vec())
    }
}

impl Default for ClawHubClient {
    fn default() -> Self {
        Self::new()
    }
}

// =========================================================
// 响应类型：紧贴 OpenAPI schema，仅保留 UI / 安装必需字段
// =========================================================

/// `GET /api/v1/skills` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillListResponse {
    pub items: Vec<SkillListItem>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Skills 列表项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillListItem {
    pub slug: String,
    pub display_name: String,
    #[serde(default)]
    pub summary: Option<String>,
    /// 主题分类（如 ["Productivity"]）
    #[serde(default)]
    pub topics: Vec<String>,
    /// 版本 tag 映射，如 `{ "latest": "1.2.3" }`
    #[serde(default)]
    pub tags: serde_json::Value,
    /// 统计信息（downloads/stars 等，结构松散）
    #[serde(default)]
    pub stats: serde_json::Value,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub latest_version: Option<SkillLatestVersion>,
}

/// 技能最新版本信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillLatestVersion {
    pub version: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub changelog: String,
}

/// `GET /api/v1/skills/{slug}` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillResponse {
    pub skill: SkillDetail,
    #[serde(default)]
    pub latest_version: Option<SkillLatestVersion>,
    #[serde(default)]
    pub owner: Option<Owner>,
    #[serde(default)]
    pub moderation: Option<Moderation>,
}

/// 技能详情（比 ListItem 多出 moderation 字段）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDetail {
    pub slug: String,
    pub display_name: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tags: serde_json::Value,
    #[serde(default)]
    pub stats: serde_json::Value,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

/// 所有者信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Owner {
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
}

/// 安全审核信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Moderation {
    #[serde(default)]
    pub is_suspicious: bool,
    #[serde(default)]
    pub is_malware_blocked: bool,
    #[serde(default)]
    pub verdict: Option<String>,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

/// `GET /api/v1/search` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
}

/// 搜索结果项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub updated_at: Option<u64>,
    #[serde(default)]
    pub owner_handle: Option<String>,
    #[serde(default)]
    pub owner: Option<Owner>,
}

/// `GET /api/v1/plugins` 响应（与 packages 共享 PackageListResponse 结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageListResponse {
    pub items: Vec<PackageCatalogItem>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// 插件 / 包列表项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageCatalogItem {
    pub name: String,
    pub display_name: String,
    /// `skill` | `code-plugin` | `bundle-plugin`
    #[serde(default)]
    pub family: String,
    /// `official` | `community` | `private`
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub is_official: bool,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub owner_handle: Option<String>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub latest_version: Option<String>,
}

/// `GET /api/v1/plugins/search` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSearchResponse {
    pub results: Vec<PackageSearchResult>,
}

/// 插件搜索结果项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSearchResult {
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub owner_handle: Option<String>,
    #[serde(default)]
    pub updated_at: Option<u64>,
}

/// `GET /api/v1/packages/{name}` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageResponse {
    #[serde(default)]
    pub package: Option<PackageDetail>,
    #[serde(default)]
    pub owner: Option<Owner>,
}

/// 包详情
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageDetail {
    pub name: String,
    pub display_name: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub is_official: bool,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub owner_handle: Option<String>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub latest_version: Option<String>,
}

// =========================================================
// 工具函数
// =========================================================

/// 简易 URL 路径段编码：把 `/`、`#`、`?` 等保留字符转义。
///
/// 对于 slug 与 package name（可能含 `@`/`/`），需要编码以避免被解析为路径。
/// 使用 `percent_encoding` 会引入额外依赖，这里手写最小子集。
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        // 安全字符：字母、数字、`-`、`_`、`.`、`~`
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// 把 ZIP 字节流解压到指定目录（同步，调用方应在 `spawn_blocking` 中调用）。
///
/// - 创建 `dest_dir`（若不存在）
/// - 跳过非文件条目（目录自动创建）
/// - 防御性拒绝绝对路径与 `..` 路径（zip-slip 攻击防护）
pub fn extract_zip_to(dest_dir: &std::path::Path, zip_bytes: &[u8]) -> std::result::Result<(), ClawHubError> {
    use std::path::{Component, Path};
    std::fs::create_dir_all(dest_dir).map_err(ClawHubError::Io)?;
    // 规范化 dest_dir 用于后续比较（canonicalize 要求路径存在，create_dir_all 已确保）
    let dest_canon = dest_dir
        .canonicalize()
        .map_err(ClawHubError::Io)
        .or_else(|_| Ok::<_, ClawHubError>(dest_dir.to_path_buf()))?;
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| ClawHubError::Zip(e.to_string()))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| ClawHubError::Zip(e.to_string()))?;
        let entry_name = entry.name().to_string();
        // 逐组件检查：禁止 `..`、绝对路径前缀（Windows 盘符 / Unix 根）
        let entry_path = Path::new(&entry_name);
        for component in entry_path.components() {
            match component {
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(ClawHubError::Zip(format!(
                        "zip entry 路径越权（含 `..` 或绝对路径）：{}",
                        entry_name
                    )));
                }
                _ => {}
            }
        }
        let out_path = dest_canon.join(entry_path);
        // 二次防御：canonicalize 父目录，确保最终路径仍在 dest_canon 之下
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(ClawHubError::Io)?;
            if let Ok(parent_canon) = parent.canonicalize() {
                if !parent_canon.starts_with(&dest_canon) {
                    return Err(ClawHubError::Zip(format!(
                        "zip entry 解析后路径越权：{}",
                        entry_name
                    )));
                }
            }
        }
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(ClawHubError::Io)?;
            continue;
        }
        let mut out_file = std::fs::File::create(&out_path).map_err(ClawHubError::Io)?;
        std::io::copy(&mut entry, &mut out_file).map_err(ClawHubError::Io)?;
    }
    Ok(())
}

/// 解析 `SKILL.md` 的 YAML frontmatter，提取 `name` / `description` / `version`。
///
/// frontmatter 格式：
/// ```yaml
/// ---
/// name: my-skill
/// description: Short summary
/// version: 1.0.0
/// ---
/// ```
///
/// 失败时返回空结构（不阻断安装，仅用作展示）。preamble 使用整个文件内容。
pub fn parse_skill_md(content: &str) -> ParsedSkillMd {
    let mut parsed = ParsedSkillMd::default();
    // 整个文件作为 preamble
    parsed.preamble = content.to_string();

    // 检测 frontmatter：以 `---\n` 开头
    if !content.starts_with("---") {
        return parsed;
    }
    // 找到结束 `---`
    let after_start = &content[3..];
    let end = match after_start.find("\n---") {
        Some(idx) => idx,
        None => return parsed,
    };
    let yaml_block = &after_start[..end];
    // 简单按行解析：name: / description: / version: 行
    for line in yaml_block.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("name:") {
            parsed.name = rest.trim().trim_matches('"').trim_matches('\'').to_string();
        } else if let Some(rest) = trimmed.strip_prefix("description:") {
            parsed.description = rest.trim().trim_matches('"').trim_matches('\'').to_string();
        } else if let Some(rest) = trimmed.strip_prefix("version:") {
            parsed.version = rest.trim().trim_matches('"').trim_matches('\'').to_string();
        }
    }
    parsed
}

/// `parse_skill_md` 的返回结构
#[derive(Debug, Default, Clone)]
pub struct ParsedSkillMd {
    pub name: String,
    pub description: String,
    pub version: String,
    /// 整个 SKILL.md 内容（作为 preamble）
    pub preamble: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_skill_md_with_frontmatter() {
        let content = "---\nname: weather\ndescription: Get current weather\nversion: 1.0.0\n---\n# Weather\nQuick one-liner:\n```\ncurl wttr.in\n```\n";
        let parsed = parse_skill_md(content);
        assert_eq!(parsed.name, "weather");
        assert_eq!(parsed.description, "Get current weather");
        assert_eq!(parsed.version, "1.0.0");
        assert!(parsed.preamble.contains("# Weather"));
    }

    #[test]
    fn parse_skill_md_without_frontmatter() {
        let content = "# Plain skill\nNo frontmatter here.";
        let parsed = parse_skill_md(content);
        assert!(parsed.name.is_empty());
        assert!(parsed.description.is_empty());
        assert_eq!(parsed.preamble, content);
    }

    #[test]
    fn urlencoding_encode_special_chars() {
        assert_eq!(urlencoding_encode("weather"), "weather");
        assert_eq!(urlencoding_encode("@openclaw/whatsapp"), "%40openclaw%2Fwhatsapp");
        assert_eq!(urlencoding_encode("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn extract_zip_rejects_path_traversal() {
        // 构造一个恶意 zip：含 `../evil.txt` 条目
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::SimpleFileOptions = Default::default();
            zip.start_file("../evil.txt", opts).unwrap();
            zip.write_all(b"pwned").unwrap();
            zip.finish().unwrap();
        }
        let tmp = std::env::temp_dir().join(format!("effisuite-zip-test-{}", uuid::Uuid::new_v4()));
        let result = extract_zip_to(&tmp, &buf.into_inner());
        assert!(result.is_err(), "extract_zip_to 应拒绝路径越权条目");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
