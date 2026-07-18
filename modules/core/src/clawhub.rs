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

    /// 把 `reqwest::Response` 解码为指定类型，失败时把原始响应体附加到错误信息中。
    ///
    /// ClawHub 响应为 JSON，但字段命名/结构可能与本地类型不一致（如 camelCase 与 snake_case），
    /// 直接返回 `reqwest::Error`（"error decoding response body"）会丢失原始 body 上下文，
    /// 不利于排查。这里先取 bytes，再 `serde_json::from_slice`，失败时包装为 `Decode` 错误。
    async fn decode_json<T: serde::de::DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> std::result::Result<T, ClawHubError> {
        let bytes = resp.bytes().await.map_err(ClawHubError::Http)?;
        serde_json::from_slice::<T>(&bytes).map_err(|e| {
            // 截取前 500 字节避免错误信息过长
            let body_preview = String::from_utf8_lossy(&bytes[..bytes.len().min(500)]);
            ClawHubError::Decode(format!(
                "{}; 原始响应（前 500 字节）：{}",
                e, body_preview
            ))
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
        self.decode_json(resp).await
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
        self.decode_json(resp).await
    }

    /// `GET /api/v1/skills/{slug}` - 获取技能详情
    pub async fn get_skill(&self, slug: &str) -> std::result::Result<SkillResponse, ClawHubError> {
        let path = format!("/api/v1/skills/{}", urlencoding_encode(slug));
        let resp = self.handle_response(self.inner.http.get(self.url(&path)).send().await?).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ClawHubError::SlugNotFound(slug.to_string()));
        }
        self.decode_json(resp).await
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
        self.decode_json(resp).await
    }

    /// `GET /api/v1/plugins/search?q=...` - 搜索插件
    ///
    /// 注意：该端点在 2026-07 实测返回 503，前端应处理失败回退到本地过滤。
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
        self.decode_json(resp).await
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
        self.decode_json(resp).await
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
///
/// 注意：ClawHub API 全部使用 camelCase 字段命名（`displayName`/`createdAt`/`nextCursor`），
/// 因此本模块所有响应结构体均添加 `#[serde(rename_all = "camelCase")]`。
/// 详见 <https://docs.openclaw.ai/clawhub/http-api>
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillListResponse {
    pub items: Vec<SkillListItem>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Skills 列表项
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// 平台 / 系统声明（`metadata.os` / `metadata.systems`），可能为 null
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// 技能最新版本信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillLatestVersion {
    pub version: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub changelog: String,
    #[serde(default)]
    pub license: Option<String>,
}

/// `GET /api/v1/skills/{slug}` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillResponse {
    pub skill: SkillDetail,
    #[serde(default)]
    pub latest_version: Option<SkillLatestVersion>,
    #[serde(default)]
    pub owner: Option<Owner>,
    /// moderation 仅在技能被标记或所有者查看时返回（文档明示）
    #[serde(default)]
    pub moderation: Option<Moderation>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// 技能详情（比 ListItem 多出 moderation 字段）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    pub slug: String,
    pub display_name: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub topics: Vec<String>,
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
    #[serde(default)]
    pub engine_version: Option<String>,
    #[serde(default)]
    pub updated_at: Option<u64>,
}

/// `GET /api/v1/search` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
}

/// 搜索结果项
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// 注意：搜索结果中的 `downloads` 是顶级字段而非嵌套在 stats 中
    #[serde(default)]
    pub downloads: Option<u64>,
    #[serde(default)]
    pub updated_at: Option<u64>,
    #[serde(default)]
    pub owner_handle: Option<String>,
    #[serde(default)]
    pub owner: Option<Owner>,
}

/// `GET /api/v1/plugins` 响应（与 packages 共享 PackageListResponse 结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageListResponse {
    pub items: Vec<PackageCatalogItem>,
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub total_count: Option<u64>,
}

/// 插件 / 包列表项
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    pub runtime_id: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub verification_tier: Option<String>,
    #[serde(default)]
    pub stats: Option<serde_json::Value>,
}

/// `GET /api/v1/plugins/search` 响应
///
/// 注意：实测该端点在 2026-07 时段返回 503，API 未稳定。
/// 保留类型定义供前端调用，调用方需处理失败回退。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSearchResponse {
    pub results: Vec<PackageSearchResult>,
}

/// 插件搜索结果项
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
    pub runtime_id: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub verification_tier: Option<String>,
    #[serde(default)]
    pub stats: Option<serde_json::Value>,
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
/// 失败时返回空结构（不阻断安装，仅用作展示）。
/// - `preamble`：整个文件内容（含 frontmatter），保留供需要完整内容的场景使用
/// - `body`：去除 frontmatter 后的正文，作为 LLM 系统消息注入最干净
pub fn parse_skill_md(content: &str) -> ParsedSkillMd {
    let mut parsed = ParsedSkillMd::default();
    // 整个文件作为 preamble（保留完整内容）
    parsed.preamble = content.to_string();
    // 默认 body 等于 preamble（无 frontmatter 时两者一致）
    parsed.body = content.to_string();

    // 检测 frontmatter：以 `---` 开头
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
    // 提取正文：跳过结束的 `\n---`（4 字节）与紧随其后的单个换行
    let after_frontmatter = &after_start[end + 4..];
    let body = after_frontmatter
        .strip_prefix('\n')
        .or_else(|| after_frontmatter.strip_prefix("\r\n"))
        .unwrap_or(after_frontmatter);
    parsed.body = body.to_string();
    parsed
}

/// `parse_skill_md` 的返回结构
#[derive(Debug, Default, Clone)]
pub struct ParsedSkillMd {
    pub name: String,
    pub description: String,
    pub version: String,
    /// 整个 SKILL.md 内容（含 frontmatter）
    pub preamble: String,
    /// SKILL.md 正文（去除 frontmatter 后的内容）。
    /// 无 frontmatter 时与 `preamble` 一致。
    /// 作为 LLM 系统消息注入时使用此字段，避免 YAML 噪声污染上下文。
    pub body: String,
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
        // body 应去除 frontmatter，仅保留正文
        assert!(!parsed.body.starts_with("---"));
        assert!(parsed.body.starts_with("# Weather"));
        assert!(parsed.body.contains("curl wttr.in"));
    }

    #[test]
    fn parse_skill_md_without_frontmatter() {
        let content = "# Plain skill\nNo frontmatter here.";
        let parsed = parse_skill_md(content);
        assert!(parsed.name.is_empty());
        assert!(parsed.description.is_empty());
        assert_eq!(parsed.preamble, content);
        // 无 frontmatter 时 body 等于 preamble
        assert_eq!(parsed.body, content);
    }

    #[test]
    fn parse_skill_md_body_strips_crlf_newline() {
        // 验证 CRLF 行尾下 body 仍能正确剥离 frontmatter
        let content = "---\r\nname: x\r\n---\r\nbody line";
        let parsed = parse_skill_md(content);
        assert_eq!(parsed.name, "x");
        assert_eq!(parsed.body, "body line");
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

    // ---------- camelCase 反序列化回归测试 ----------
    // 这些测试用真实 API 返回的 JSON 结构（camelCase 字段名）验证本地结构体能正确解码。
    // 防止 "error decoding response body" 类错误回归。

    #[test]
    fn skill_list_response_decodes_camel_case() {
        let json = r#"{
            "items": [
                {
                    "slug": "weather",
                    "displayName": "Weather",
                    "summary": "Get current weather",
                    "topics": ["Productivity"],
                    "tags": { "latest": "1.2.3" },
                    "stats": { "downloads": 100 },
                    "createdAt": 1730000000000,
                    "updatedAt": 1730000001000,
                    "latestVersion": {
                        "version": "1.2.3",
                        "createdAt": 1730000000000,
                        "changelog": "init",
                        "license": null
                    },
                    "metadata": null
                }
            ],
            "nextCursor": "cursor-xyz"
        }"#;
        let resp: SkillListResponse = serde_json::from_str(json).expect("应能解码 SkillListResponse");
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].slug, "weather");
        assert_eq!(resp.items[0].display_name, "Weather");
        assert_eq!(resp.items[0].created_at, 1730000000000);
        assert_eq!(resp.items[0].updated_at, 1730000001000);
        assert_eq!(resp.next_cursor.as_deref(), Some("cursor-xyz"));
        let lv = resp.items[0].latest_version.as_ref().expect("latest_version 应存在");
        assert_eq!(lv.version, "1.2.3");
        assert_eq!(lv.changelog, "init");
    }

    #[test]
    fn search_response_decodes_camel_case() {
        let json = r#"{
            "results": [
                {
                    "score": 4.138,
                    "slug": "weather",
                    "displayName": "Weather",
                    "summary": "Get current weather",
                    "version": null,
                    "downloads": 164330,
                    "updatedAt": 1778485729679,
                    "ownerHandle": "steipete",
                    "owner": {
                        "handle": "steipete",
                        "displayName": "Peter Steinberger",
                        "image": "https://example.com/a.png"
                    }
                }
            ]
        }"#;
        let resp: SearchResponse = serde_json::from_str(json).expect("应能解码 SearchResponse");
        assert_eq!(resp.results.len(), 1);
        let r = &resp.results[0];
        assert_eq!(r.slug.as_deref(), Some("weather"));
        assert_eq!(r.display_name.as_deref(), Some("Weather"));
        assert_eq!(r.owner_handle.as_deref(), Some("steipete"));
        assert_eq!(r.downloads, Some(164330));
        let owner = r.owner.as_ref().expect("owner 应存在");
        assert_eq!(owner.handle.as_deref(), Some("steipete"));
        assert_eq!(owner.display_name.as_deref(), Some("Peter Steinberger"));
    }

    #[test]
    fn package_list_response_decodes_camel_case() {
        let json = r#"{
            "items": [
                {
                    "name": "@openclaw/whatsapp",
                    "displayName": "WhatsApp",
                    "family": "code-plugin",
                    "channel": "official",
                    "isOfficial": true,
                    "summary": "OpenClaw WhatsApp channel plugin",
                    "ownerHandle": "openclaw",
                    "runtimeId": "whatsapp",
                    "icon": "https://cdn.simpleicons.org/whatsapp",
                    "categories": ["channels"],
                    "topics": ["WhatsApp"],
                    "createdAt": 1777700677247,
                    "updatedAt": 1784288051062,
                    "latestVersion": "2026.7.1",
                    "verificationTier": "source-linked",
                    "stats": { "downloads": 153955 }
                }
            ],
            "nextCursor": "pkg-cursor",
            "totalCount": 1562
        }"#;
        let resp: PackageListResponse = serde_json::from_str(json).expect("应能解码 PackageListResponse");
        assert_eq!(resp.items.len(), 1);
        let item = &resp.items[0];
        assert_eq!(item.name, "@openclaw/whatsapp");
        assert_eq!(item.display_name, "WhatsApp");
        assert!(item.is_official);
        assert_eq!(item.family, "code-plugin");
        assert_eq!(item.runtime_id.as_deref(), Some("whatsapp"));
        assert_eq!(item.latest_version.as_deref(), Some("2026.7.1"));
        assert_eq!(resp.total_count, Some(1562));
    }

    #[test]
    fn skill_response_decodes_with_optional_moderation() {
        // 文档明示：moderation 仅在技能被标记或所有者查看时返回；普通调用可能缺该字段
        let json = r#"{
            "skill": {
                "slug": "gifgrep",
                "displayName": "GifGrep",
                "summary": "Gif search",
                "topics": [],
                "tags": {},
                "stats": {},
                "createdAt": 0,
                "updatedAt": 0
            },
            "latestVersion": {
                "version": "1.0.0",
                "createdAt": 0,
                "changelog": ""
            },
            "owner": {
                "handle": "steipete",
                "displayName": "Peter",
                "image": null
            },
            "metadata": null
        }"#;
        let resp: SkillResponse = serde_json::from_str(json).expect("应能解码 SkillResponse");
        assert_eq!(resp.skill.slug, "gifgrep");
        assert!(resp.moderation.is_none(), "缺 moderation 字段时应为 None");
        assert!(resp.metadata.is_none());
        assert_eq!(resp.owner.as_ref().and_then(|o| o.handle.clone()), Some("steipete".to_string()));
    }

    #[test]
    fn moderation_decodes_camel_case() {
        let json = r#"{
            "isSuspicious": true,
            "isMalwareBlocked": false,
            "verdict": "suspicious",
            "reasonCodes": ["suspicious.dynamic_code_execution"],
            "summary": "Detected: dynamic code execution",
            "engineVersion": "v2.0.0",
            "updatedAt": 1730000000000
        }"#;
        let m: Moderation = serde_json::from_str(json).expect("应能解码 Moderation");
        assert!(m.is_suspicious);
        assert!(!m.is_malware_blocked);
        assert_eq!(m.verdict.as_deref(), Some("suspicious"));
        assert_eq!(m.reason_codes, vec!["suspicious.dynamic_code_execution"]);
        assert_eq!(m.engine_version.as_deref(), Some("v2.0.0"));
        assert_eq!(m.updated_at, Some(1730000000000));
    }
}
