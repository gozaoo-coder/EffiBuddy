use std::sync::Arc;

use effisuite_core::{ClawHubClient, Skill, SkillIndex, SkillStore};
use rig_core::tool::Tool;
use serde::Deserialize;

use super::{now_ms, short_id};

// =========================================================
// InstallClawHubSkillTool：从 ClawHub 下载并安装技能
// =========================================================

#[derive(Deserialize)]
pub struct InstallClawHubSkillArgs {
    /// ClawHub 技能 slug（来自 search_clawhub_skills 结果）
    pub slug: String,
}

#[derive(Debug, thiserror::Error)]
#[error("install clawhub skill error: {0}")]
pub struct InstallClawHubSkillError(String);

/// 从 ClawHub 安装技能工具
///
/// 持有：
/// - `ClawHubClient`：HTTP 下载 + 元数据查询
/// - `SkillStore`：持久化安装的技能
/// - `SkillIndex`：安装后 rebuild 索引，让下一轮 RAG 自动注入看到新技能
/// - `skills_dir`：技能解压根目录
///
/// 安装流程与 Tauri 层 `clawhub_install_skill` 命令一致：
/// 1. 幂等检查（find_by_clawhub_slug）
/// 2. 下载 ZIP（5min 超时）
/// 3. spawn_blocking 解压到 <skills_dir>/<slug>/
/// 4. 解析 SKILL.md frontmatter + 正文
/// 5. 落盘 SkillStore（preamble=SKILL.md 正文）
/// 6. rebuild SkillIndex
pub struct InstallClawHubSkillTool {
    client: ClawHubClient,
    store: SkillStore,
    index: Arc<SkillIndex>,
    skills_dir: std::path::PathBuf,
}

impl InstallClawHubSkillTool {
    pub fn new(
        client: ClawHubClient,
        store: SkillStore,
        index: Arc<SkillIndex>,
        skills_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            client,
            store,
            index,
            skills_dir,
        }
    }
}

impl Tool for InstallClawHubSkillTool {
    const NAME: &'static str = "install_clawhub_skill";

    type Error = InstallClawHubSkillError;
    type Args = InstallClawHubSkillArgs;
    type Output = String;

  fn description(&self) -> String {
      "从 ClawHub 下载并安装指定技能，安装后立即可用（RAG 自动注入 / enable_skill 启用）。\
       slug 来自 search_clawhub_skills 结果；安装幂等，重复安装返回已存在 id。"
          .to_string()
  }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "slug": {
                    "type": "string",
                    "description": "ClawHub 技能 slug（来自 search_clawhub_skills 结果）"
                }
            },
            "required": ["slug"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        use effisuite_core::clawhub::{extract_zip_to, parse_skill_md};

        let slug = args.slug.trim();
        if slug.is_empty() {
            return Err(InstallClawHubSkillError("slug 不能为空".to_string()));
        }

        // 1. 幂等：已安装则直接返回
        if let Some(existing) = self
            .store
            .find_by_clawhub_slug(slug)
            .await
            .map_err(|e| InstallClawHubSkillError(e.to_string()))?
        {
            return Ok(format!(
                "技能「{}」（slug={}）已安装，无需重复安装。可调用 enable_skill(id={}) 启用。",
                existing.name, slug, short_id(&existing.id)
            ));
        }

        // 2. 拉详情获取 owner / latest_version
        let detail = self
            .client
            .get_skill(slug)
            .await
            .map_err(|e| InstallClawHubSkillError(format!("获取技能详情失败: {e}")))?;
        let owner_handle = detail
            .owner
            .as_ref()
            .and_then(|o| o.handle.clone())
            .unwrap_or_default();
        let version = detail
            .latest_version
            .as_ref()
            .map(|v| v.version.clone())
            .unwrap_or_default();

        // 3. 下载 ZIP
        let zip_bytes = self
            .client
            .download_skill_zip(slug, None, None)
            .await
            .map_err(|e| InstallClawHubSkillError(format!("下载技能包失败: {e}")))?;

        // 4. 解压到 <skills_dir>/<slug>/
        let dest_dir = self.skills_dir.join(slug);
        let dest_for_blocking = dest_dir.clone();
        tokio::task::spawn_blocking(move || extract_zip_to(&dest_for_blocking, &zip_bytes))
            .await
            .map_err(|e| InstallClawHubSkillError(format!("解压任务调度失败: {e}")))?
            .map_err(|e| InstallClawHubSkillError(format!("解压失败: {e}")))?;

        // 5. 解析 SKILL.md：提取 frontmatter 字段 + 正文作为 preamble
        let skill_md_path = dest_dir.join("SKILL.md");
        let (name, description, parsed_version, body) =
            match tokio::fs::read_to_string(&skill_md_path).await {
                Ok(content) => {
                    let p = parse_skill_md(&content);
                    (
                        if p.name.is_empty() { slug.to_string() } else { p.name },
                        if p.description.is_empty() {
                            format!("ClawHub 技能: {}", slug)
                        } else {
                            p.description
                        },
                        if p.version.is_empty() { version.clone() } else { p.version },
                        p.body,
                    )
                }
                Err(_) => (
                    slug.to_string(),
                    format!("ClawHub 技能: {}", slug),
                    version.clone(),
                    String::new(),
                ),
            };

        // 6. 落盘 SkillStore
        let skill = Skill {
            id: slug.to_string(),
            name,
            description,
            preamble: body,
            tools: Vec::new(),
            working_dir: Some(dest_dir.to_string_lossy().into_owned()),
            created_at: now_ms(),
            builtin: false,
            source: Some("clawhub".to_string()),
            source_slug: Some(slug.to_string()),
            source_owner: if owner_handle.is_empty() {
                None
            } else {
                Some(owner_handle)
            },
            source_version: if parsed_version.is_empty() {
                None
            } else {
                Some(parsed_version)
            },
        };
        self.store
            .save(&skill)
            .await
            .map_err(|e| InstallClawHubSkillError(e.to_string()))?;

        // 7. rebuild SkillIndex，让下一轮 RAG 自动注入看到新技能
        self.index
            .rebuild_from_store(&self.store)
            .await
            .map_err(|e| InstallClawHubSkillError(e.to_string()))?;

        Ok(format!(
            "已安装技能「{}」（slug={}，id={}）。\
             下一轮对话的 RAG 自动注入会包含它，也可直接调用 enable_skill(id={}) 启用。",
            skill.name,
            slug,
            short_id(&skill.id),
            short_id(&skill.id)
        ))
    }
}
