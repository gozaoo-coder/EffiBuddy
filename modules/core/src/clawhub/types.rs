use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;

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
        let resp: SkillListResponse =
            serde_json::from_str(json).expect("应能解码 SkillListResponse");
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].slug, "weather");
        assert_eq!(resp.items[0].display_name, "Weather");
        assert_eq!(resp.items[0].created_at, 1730000000000);
        assert_eq!(resp.items[0].updated_at, 1730000001000);
        assert_eq!(resp.next_cursor.as_deref(), Some("cursor-xyz"));
        let lv = resp.items[0]
            .latest_version
            .as_ref()
            .expect("latest_version 应存在");
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
        let resp: PackageListResponse =
            serde_json::from_str(json).expect("应能解码 PackageListResponse");
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
        assert_eq!(
            resp.owner.as_ref().and_then(|o| o.handle.clone()),
            Some("steipete".to_string())
        );
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
