use effisuite_core::ClawHubClient;
use rig_core::tool::Tool;
use serde::Deserialize;

// =========================================================
// SearchClawHubSkillsTool：从 ClawHub 搜索技能
// =========================================================

#[derive(Deserialize)]
pub struct SearchClawHubSkillsArgs {
    /// 搜索关键词
    pub query: String,
    /// 最多返回多少条结果，默认 10
    #[serde(default = "default_search_limit")]
    pub limit: u32,
}

fn default_search_limit() -> u32 {
    10
}

#[derive(Debug, thiserror::Error)]
#[error("search clawhub skills error: {0}")]
pub struct SearchClawHubSkillsError(String);

/// 从 ClawHub 搜索技能工具
///
/// 持有 `ClawHubClient` 共享句柄。当本地无匹配技能时，agent 据此
/// 从 ClawHub 远程搜索，找到合适 slug 后调 install_clawhub_skill 安装。
pub struct SearchClawHubSkillsTool {
    client: ClawHubClient,
}

impl SearchClawHubSkillsTool {
    pub fn new(client: ClawHubClient) -> Self {
        Self { client }
    }
}

impl Tool for SearchClawHubSkillsTool {
    const NAME: &'static str = "search_clawhub_skills";

    type Error = SearchClawHubSkillsError;
    type Args = SearchClawHubSkillsArgs;
    type Output = String;

    fn description(&self) -> String {
        "从 ClawHub 远程技能市场搜索技能（不限于本地已安装）。\
         当用户需要的能力本地已安装技能都无法满足时调用。\
         找到合适技能后，用返回的 slug 调用 install_clawhub_skill 安装。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜索关键词，如 'weather' / '翻译' / 'code review'"
                },
                "limit": {
                    "type": "integer",
                    "description": "最多返回的结果条数，默认 10",
                    "default": 10
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let query = args.query.trim();
        if query.is_empty() {
            return Err(SearchClawHubSkillsError("query 不能为空".to_string()));
        }
        let limit = if args.limit == 0 { 10 } else { args.limit };

        let resp = self
            .client
            .search_skills(query, Some(limit))
            .await
            .map_err(|e| SearchClawHubSkillsError(e.to_string()))?;

        if resp.results.is_empty() {
            return Ok(format!("未在 ClawHub 找到与「{}」相关的技能。", query));
        }

        let mut out = String::with_capacity(resp.results.len() * 128);
        out.push_str(&format!("在 ClawHub 找到 {} 个相关技能：\n", resp.results.len()));
        for (i, r) in resp.results.iter().enumerate() {
            let slug = r.slug.as_deref().unwrap_or("(无 slug)");
            let name = r.display_name.as_deref().unwrap_or("(未命名)");
            let summary = r.summary.as_deref().unwrap_or("(无简介)");
            let owner = r
                .owner_handle
                .as_deref()
                .or_else(|| r.owner.as_ref().and_then(|o| o.handle.as_deref()))
                .unwrap_or("(未知)");
            let version = r.version.as_deref().unwrap_or("?");
            out.push_str(&format!(
                "{}. [slug={}] {} (v{}, by {})\n   {}\n",
                i + 1,
                slug,
                name,
                version,
                owner,
                summary
            ));
        }
        out.push_str("\n提示：用 install_clawhub_skill(slug) 安装选定技能。");
        Ok(out)
    }
}
