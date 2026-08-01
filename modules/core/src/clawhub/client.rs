use std::sync::Arc;
use std::time::Duration;

use super::error::ClawHubError;
use super::types::{
    PackageListResponse, PackageResponse, PackageSearchResponse, SearchResponse, SkillListResponse,
    SkillResponse,
};

/// ClawHub 站点根 URL（可被 `CLAWHUB_SITE` 覆盖，但此处固定默认值）
pub const CLAWHUB_BASE_URL: &str = "https://clawhub.ai";

/// HTTP 客户端默认超时（30s）。下载 ZIP 时按需覆盖到更长。
const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// 下载 ZIP 时的超时（5 min，ClawHub 下载限速 1200/min/IP）。
const DOWNLOAD_TIMEOUT_SECS: u64 = 300;

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
            ClawHubError::Decode(format!("{}; 原始响应（前 500 字节）：{}", e, body_preview))
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
        let resp = self
            .handle_response(self.inner.http.get(self.url(&path)).send().await?)
            .await?;
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
        let resp = self
            .handle_response(self.inner.http.get(self.url(&path)).send().await?)
            .await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencoding_encode_special_chars() {
        assert_eq!(urlencoding_encode("weather"), "weather");
        assert_eq!(
            urlencoding_encode("@openclaw/whatsapp"),
            "%40openclaw%2Fwhatsapp"
        );
        assert_eq!(urlencoding_encode("a b/c"), "a%20b%2Fc");
    }
}
