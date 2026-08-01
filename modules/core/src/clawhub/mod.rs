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

mod client;
mod error;
mod skill_md;
mod types;
mod zip;

pub use client::{ClawHubClient, CLAWHUB_BASE_URL};
pub use error::ClawHubError;
pub use skill_md::{parse_skill_md, ParsedSkillMd};
pub use types::{
    Moderation, Owner, PackageCatalogItem, PackageDetail, PackageListResponse, PackageResponse,
    PackageSearchResponse, PackageSearchResult, SearchResult, SearchResponse, SkillDetail,
    SkillLatestVersion, SkillListItem, SkillListResponse, SkillResponse,
};
pub use zip::extract_zip_to;
